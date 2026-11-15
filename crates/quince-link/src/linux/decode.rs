//! The phone's pictures, decoded the moment they arrive.
//!
//! On the graphics chip (VA-API) when it can, which costs almost no processor time, and in
//! software otherwise. Every setting is chosen for delay over throughput: one decoding thread
//! (frame threads each hold a picture back), low-delay output, and no reordering, since the
//! phone encodes without B-frames.

use std::ptr;

use anyhow::{bail, Result};
use ffmpeg_next::ffi::*;

use crate::protocol::Codec;

/// One decoded picture, NV12: the full-size brightness plane, then the half-size colour plane
/// with blue and red interleaved, both packed without row padding.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub nv12: Vec<u8>,
}

pub struct Decoder {
    ctx: *mut AVCodecContext,
    frame: *mut AVFrame,
    soft: *mut AVFrame,
    packet: *mut AVPacket,
    pub hardware: bool,
}

// The decoder is driven from one thread at a time; its pointers are its own.
unsafe impl Send for Decoder {}

unsafe extern "C" fn pick_vaapi(
    ctx: *mut AVCodecContext,
    offered: *const AVPixelFormat,
) -> AVPixelFormat {
    let mut p = offered;
    while *p != AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == AVPixelFormat::AV_PIX_FMT_VAAPI {
            return *p;
        }
        p = p.add(1);
    }
    // The chip can't take this stream after all (a profile it lacks): software takes over.
    avcodec_default_get_format(ctx, offered)
}

impl Decoder {
    pub fn new(codec: Codec) -> Result<Decoder> {
        ffmpeg_next::init()?;
        unsafe {
            let id = match codec {
                Codec::H264 => AVCodecID::AV_CODEC_ID_H264,
                Codec::Hevc => AVCodecID::AV_CODEC_ID_HEVC,
            };
            let found = avcodec_find_decoder(id);
            if found.is_null() {
                bail!("this computer's FFmpeg has no {codec:?} decoder");
            }
            let ctx = avcodec_alloc_context3(found);
            (*ctx).flags |= AV_CODEC_FLAG_LOW_DELAY as i32;
            (*ctx).flags2 |= AV_CODEC_FLAG2_FAST;
            (*ctx).thread_count = 1;

            let mut device: *mut AVBufferRef = ptr::null_mut();
            let hardware = av_hwdevice_ctx_create(
                &mut device,
                AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                ptr::null(),
                ptr::null_mut(),
                0,
            ) >= 0;
            if hardware {
                (*ctx).hw_device_ctx = av_buffer_ref(device);
                (*ctx).get_format = Some(pick_vaapi);
                av_buffer_unref(&mut device);
            }
            if avcodec_open2(ctx, found, ptr::null_mut()) < 0 {
                let mut c = ctx;
                avcodec_free_context(&mut c);
                bail!("the {codec:?} decoder would not open");
            }
            Ok(Decoder {
                ctx,
                frame: av_frame_alloc(),
                soft: av_frame_alloc(),
                packet: av_packet_alloc(),
                hardware,
            })
        }
    }

    /// Decodes one access unit; returns the picture it completes, if any. A damaged unit is
    /// an error the caller answers by asking the phone for a keyframe.
    pub fn decode(&mut self, unit: &[u8]) -> Result<Option<Picture>> {
        unsafe {
            if av_new_packet(self.packet, unit.len() as i32) < 0 {
                bail!("out of memory for a {}-byte picture", unit.len());
            }
            ptr::copy_nonoverlapping(unit.as_ptr(), (*self.packet).data, unit.len());
            let sent = avcodec_send_packet(self.ctx, self.packet);
            av_packet_unref(self.packet);
            if sent < 0 && sent != AVERROR(EAGAIN) {
                bail!("the decoder refused a picture ({sent})");
            }
            let got = avcodec_receive_frame(self.ctx, self.frame);
            if got == AVERROR(EAGAIN) {
                return Ok(None);
            }
            if got < 0 {
                bail!("the decoder failed on a picture ({got})");
            }
            let mut f = self.frame;
            if (*f).format == AVPixelFormat::AV_PIX_FMT_VAAPI as i32 {
                av_frame_unref(self.soft);
                (*self.soft).format = AVPixelFormat::AV_PIX_FMT_NV12 as i32;
                if av_hwframe_transfer_data(self.soft, f, 0) < 0 {
                    av_frame_unref(f);
                    bail!("the picture could not be copied off the graphics chip");
                }
                f = self.soft;
            }
            let picture = pack_nv12(f);
            av_frame_unref(self.frame);
            av_frame_unref(self.soft);
            picture.map(Some)
        }
    }
}

/// Copies the planes out without row padding, turning software decoding's three planes
/// (YUV 4:2:0) into NV12's two.
unsafe fn pack_nv12(f: *const AVFrame) -> Result<Picture> {
    let (w, h) = ((*f).width as usize, (*f).height as usize);
    let mut nv12 = vec![0u8; w * h * 3 / 2];
    let (luma, chroma) = nv12.split_at_mut(w * h);
    for row in 0..h {
        let src = (*f).data[0].add(row * (*f).linesize[0] as usize);
        ptr::copy_nonoverlapping(src, luma[row * w..].as_mut_ptr(), w);
    }
    let format = (*f).format;
    if format == AVPixelFormat::AV_PIX_FMT_NV12 as i32 {
        for row in 0..h / 2 {
            let src = (*f).data[1].add(row * (*f).linesize[1] as usize);
            ptr::copy_nonoverlapping(src, chroma[row * w..].as_mut_ptr(), w);
        }
    } else if format == AVPixelFormat::AV_PIX_FMT_YUV420P as i32
        || format == AVPixelFormat::AV_PIX_FMT_YUVJ420P as i32
    {
        for row in 0..h / 2 {
            let u = (*f).data[1].add(row * (*f).linesize[1] as usize);
            let v = (*f).data[2].add(row * (*f).linesize[2] as usize);
            let out = &mut chroma[row * w..row * w + w];
            for x in 0..w / 2 {
                out[2 * x] = *u.add(x);
                out[2 * x + 1] = *v.add(x);
            }
        }
    } else {
        bail!("the decoder gave pixel format {format}, which isn't 4:2:0");
    }
    Ok(Picture {
        width: w as u32,
        height: h as u32,
        nv12,
    })
}

impl Decoder {
    /// Forgets everything decoded so far, so decoding can start again at a keyframe.
    pub fn flush(&mut self) {
        unsafe { avcodec_flush_buffers(self.ctx) }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            av_frame_free(&mut self.frame);
            av_frame_free(&mut self.soft);
            av_packet_free(&mut self.packet);
            avcodec_free_context(&mut self.ctx);
        }
    }
}
