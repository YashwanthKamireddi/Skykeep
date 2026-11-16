//! The phone's pictures, decoded the moment they arrive, through GStreamer.
//!
//! GStreamer rather than linking FFmpeg directly: Skykeep already needs it (WebKitGTK plays
//! video through it), its interface stays the same across distribution releases where
//! FFmpeg's library versions change, and it brings each system's best decoder: the graphics
//! chip's (VA-API, `vah264dec`) when there is one, FFmpeg's or OpenH264 in software otherwise.
//!
//! Every setting favours delay over throughput: the parser and decoder hand each picture on
//! as soon as it is whole, the software decoder splits work within a picture rather than
//! across pictures (frame threads each hold a picture back), and nothing waits on a clock.

use anyhow::{anyhow, bail, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;

use crate::protocol::Codec;

/// One decoded picture, NV12: the full-size brightness plane, then the half-size colour plane
/// with blue and red interleaved, both packed without row padding.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub nv12: Vec<u8>,
}

pub struct Decoder {
    pipeline: gst::Pipeline,
    source: gst_app::AppSrc,
    pub hardware: bool,
}

/// Decoders in the order tried: the graphics chip's first.
const DECODERS: &[(&str, &str, bool)] = &[
    ("vah264dec", "vah265dec", true),
    ("avdec_h264", "avdec_h265", false),
    ("openh264dec", "", false),
];

impl Decoder {
    /// A decoder that hands each picture, with the capture time it was sent with, to `show`.
    pub fn new(
        codec: Codec,
        show: impl Fn(Picture, i64) + Send + Sync + 'static,
    ) -> Result<Decoder> {
        gst::init()?;
        let (caps, parser) = match codec {
            Codec::H264 => ("video/x-h264", "h264parse"),
            Codec::Hevc => ("video/x-h265", "h265parse"),
        };
        // The graphics chip's decoder needs the parser (both come with gst-plugins-bad); without
        // it, FFmpeg's decoder (gst-libav, which Skykeep depends on) takes whole pictures as
        // they come.
        let parse = gst::ElementFactory::make(parser).build().ok();
        let (decoder, hardware) = DECODERS
            .iter()
            .filter(|(_, _, hw)| parse.is_some() || !*hw)
            .filter_map(|(h264, h265, hw)| {
                let name = if codec == Codec::H264 { *h264 } else { *h265 };
                if name.is_empty() {
                    return None;
                }
                gst::ElementFactory::make(name)
                    .build()
                    .ok()
                    .map(|d| (d, *hw))
            })
            .next()
            .ok_or_else(|| {
                anyhow!(
                    "no {codec:?} decoder in GStreamer here: install gst-libav or gst-plugin-va"
                )
            })?;
        // The streams here never reorder pictures (the phone encodes without B-frames), so the
        // decoder may hand each one on at once instead of holding pictures back as a spec
        // decoder must when it can't know that.
        if decoder.has_property("compliance") {
            decoder.set_property_from_str("compliance", "flexible");
        }
        if decoder.has_property("thread-type") {
            decoder.set_property_from_str("thread-type", "slice");
        }

        let source = gst_app::AppSrc::builder()
            .caps(
                &gst::Caps::builder(caps)
                    .field("stream-format", "byte-stream")
                    .field("alignment", "au")
                    .build(),
            )
            .is_live(true)
            .format(gst::Format::Time)
            .build();
        let sink = gst_app::AppSink::builder()
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", gst::List::new(["NV12", "I420"]))
                    .build(),
            )
            .sync(false)
            .enable_last_sample(false)
            .build();
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(p) = picture(&sample) {
                        let captured = sample
                            .buffer()
                            .and_then(|b| b.pts())
                            .map(|t| t.useconds() as i64)
                            .unwrap_or(i64::MIN);
                        show(p, captured);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let pipeline = gst::Pipeline::new();
        let mut chain: Vec<&gst::Element> = vec![source.upcast_ref()];
        chain.extend(parse.as_ref());
        chain.extend([&decoder, sink.upcast_ref()]);
        pipeline.add_many(chain.iter().copied())?;
        gst::Element::link_many(chain.iter().copied())?;
        pipeline.set_state(gst::State::Playing)?;
        Ok(Decoder {
            pipeline,
            source,
            hardware,
        })
    }

    /// Hands one access unit to the decoder; its picture arrives at `show`. An error means the
    /// stream can't go on as it is: the caller starts again at a keyframe.
    pub fn decode(&mut self, unit: &[u8], captured_us: i64) -> Result<()> {
        if let Some(bus) = self.pipeline.bus() {
            if let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error]) {
                if let gst::MessageView::Error(e) = msg.view() {
                    bail!("the decoder failed: {}", e.error());
                }
            }
        }
        let mut buffer = gst::Buffer::from_slice(unit.to_vec());
        if captured_us >= 0 {
            buffer
                .get_mut()
                .unwrap()
                .set_pts(gst::ClockTime::from_useconds(captured_us as u64));
        }
        self.source
            .push_buffer(buffer)
            .map_err(|e| anyhow!("the decoder stopped: {e:?}"))?;
        Ok(())
    }

    /// Forgets everything decoded so far, so decoding can start again at a keyframe.
    pub fn flush(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = self.pipeline.set_state(gst::State::Playing);
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// The sample's planes copied out without row padding, I420's two colour planes interleaved
/// into NV12's one.
fn picture(sample: &gst::SampleRef) -> Option<Picture> {
    let info = gst_video::VideoInfo::from_caps(sample.caps()?).ok()?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(sample.buffer()?, &info).ok()?;
    let (w, h) = (info.width() as usize, info.height() as usize);
    let mut nv12 = vec![0u8; w * h * 3 / 2];
    let (luma, chroma) = nv12.split_at_mut(w * h);
    let y = frame.plane_data(0).ok()?;
    let ys = frame.plane_stride()[0] as usize;
    for row in 0..h {
        luma[row * w..row * w + w].copy_from_slice(&y[row * ys..row * ys + w]);
    }
    match info.format() {
        gst_video::VideoFormat::Nv12 => {
            let uv = frame.plane_data(1).ok()?;
            let s = frame.plane_stride()[1] as usize;
            for row in 0..h / 2 {
                chroma[row * w..row * w + w].copy_from_slice(&uv[row * s..row * s + w]);
            }
        }
        gst_video::VideoFormat::I420 => {
            let (u, v) = (frame.plane_data(1).ok()?, frame.plane_data(2).ok()?);
            let (us, vs) = (
                frame.plane_stride()[1] as usize,
                frame.plane_stride()[2] as usize,
            );
            for row in 0..h / 2 {
                let out = &mut chroma[row * w..row * w + w];
                for x in 0..w / 2 {
                    out[2 * x] = u[row * us + x];
                    out[2 * x + 1] = v[row * vs + x];
                }
            }
        }
        _ => return None,
    }
    Some(Picture {
        width: w as u32,
        height: h as u32,
        nv12,
    })
}
