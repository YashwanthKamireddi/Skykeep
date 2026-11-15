//! A stand-in for the phone app, for tests and for measuring this side alone: a moving test
//! picture encoded as the phone encodes (H.264, no B-frames, real-time settings) and a tone in
//! 5 ms blocks, each stamped with this computer's clock so delays read directly.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::protocol::{Codec, Hello, Kind, Packet, VideoFormat, AUDIO_RATE, KEYFRAME, VERSION};
use crate::stats::now_us;

pub struct FakePhone {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for FakePhone {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Listens on a free loopback port, as the phone app listens behind the USB tunnel, and
/// streams to the first computer that connects.
pub fn start(width: u32, height: u32, fps: u32) -> Result<FakePhone> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::spawn(move || {
        if let Ok((link, _)) = listener.accept() {
            if let Err(e) = stream(link, width, height, fps, s) {
                eprintln!("fake phone stopped: {e:#}");
            }
        }
    });
    Ok(FakePhone { port, stop })
}

fn stream(link: TcpStream, width: u32, height: u32, fps: u32, stop: Arc<AtomicBool>) -> Result<()> {
    link.set_nodelay(true)?;
    let out = Arc::new(Mutex::new(link.try_clone()?));
    let send = |p: &Packet| -> Result<()> {
        let mut o = out.lock().unwrap();
        p.write_to(&mut *o)?;
        Ok(o.flush()?)
    };
    send(&Packet::message(
        Kind::Hello,
        &Hello {
            version: VERSION,
            name: "Test iPhone".into(),
        },
    ))?;
    send(&Packet::message(
        Kind::VideoFormat,
        &VideoFormat {
            codec: Codec::H264,
            width,
            height,
            fps,
        },
    ))?;

    // The computer asks for a keyframe when it had to drop pictures.
    let want_key = Arc::new(AtomicBool::new(true));
    {
        let (mut r, want_key, stop) = (link, want_key.clone(), stop.clone());
        std::thread::spawn(move || {
            while let Ok(Some(p)) = Packet::read_from(&mut r) {
                if p.kind == Kind::Control {
                    want_key.store(true, Ordering::Relaxed);
                }
            }
            stop.store(true, Ordering::Relaxed);
        });
    }

    // The tone, in 5 ms blocks, paced by the clock.
    {
        let (out, stop) = (out.clone(), stop.clone());
        std::thread::spawn(move || {
            let block = AUDIO_RATE as usize / 200;
            let start = Instant::now();
            let mut n: u64 = 0;
            while !stop.load(Ordering::Relaxed) {
                let body: Vec<u8> = (0..block)
                    .flat_map(|i| {
                        let t = (n as usize * block + i) as f64 / AUDIO_RATE as f64;
                        (((t * 440.0 * std::f64::consts::TAU).sin() * 8000.0) as i16).to_le_bytes()
                    })
                    .collect();
                let p = Packet {
                    kind: Kind::Audio,
                    flags: 0,
                    captured_us: now_us(),
                    body,
                };
                let mut o = out.lock().unwrap();
                if p.write_to(&mut *o).and_then(|_| o.flush()).is_err() {
                    break;
                }
                drop(o);
                n += 1;
                let next = start + Duration::from_micros(n * 5000);
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
            }
        });
    }

    let (source, sink, pipeline) = encoder(width, height, fps)?;
    let frame_bytes = (width * height * 3 / 2) as usize;
    let start = Instant::now();
    let mut i: u64 = 0;
    while !stop.load(Ordering::Relaxed) {
        if want_key.swap(false, Ordering::Relaxed) {
            let _ = source.send_event(
                gst_video::DownstreamForceKeyUnitEvent::builder()
                    .all_headers(true)
                    .build(),
            );
        }
        let captured = now_us();
        let mut buffer = gst::Buffer::with_size(frame_bytes)?;
        {
            let b = buffer.get_mut().unwrap();
            b.set_pts(gst::ClockTime::from_useconds(i * 1_000_000 / fps as u64));
            paint(&mut b.map_writable()?, width as usize, height as usize, i);
        }
        source
            .push_buffer(buffer)
            .map_err(|e| anyhow::anyhow!("the encoder stopped: {e:?}"))?;
        // Real-time settings: each picture comes out before the next goes in.
        if let Some(sample) = sink.try_pull_sample(gst::ClockTime::from_seconds(1)) {
            let buffer = sample.buffer().context("an empty encoded picture")?;
            let key = if buffer.flags().contains(gst::BufferFlags::DELTA_UNIT) {
                0
            } else {
                KEYFRAME
            };
            send(&Packet {
                kind: Kind::Video,
                flags: key,
                captured_us: captured,
                body: buffer.map_readable()?.to_vec(),
            })?;
        }
        i += 1;
        let next = start + Duration::from_micros(i * 1_000_000 / fps as u64);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    let _ = pipeline.set_state(gst::State::Null);
    // Hung up, as an unplugged cable does: the computer sees the link close at once.
    let _ = out.lock().unwrap().shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// H.264 as the phone sends it (no B-frames, real time, keyframes every two seconds), through
/// GStreamer: x264 with its zero-latency tuning, OpenH264 where x264 isn't installed.
fn encoder(
    width: u32,
    height: u32,
    fps: u32,
) -> Result<(gst_app::AppSrc, gst_app::AppSink, gst::Pipeline)> {
    gst::init()?;
    let raw = gst_video::VideoInfo::builder(gst_video::VideoFormat::I420, width, height)
        .fps(gst::Fraction::new(fps as i32, 1))
        .build()?
        .to_caps()?;
    let source = gst_app::AppSrc::builder()
        .caps(&raw)
        .format(gst::Format::Time)
        .is_live(true)
        .build();
    let encode = match gst::ElementFactory::make("x264enc").build() {
        Ok(e) => {
            e.set_property_from_str("tune", "zerolatency");
            e.set_property_from_str("speed-preset", "ultrafast");
            e.set_property("bframes", 0u32);
            e.set_property("key-int-max", fps * 2);
            e
        }
        Err(_) => gst::ElementFactory::make("openh264enc")
            .build()
            .context("GStreamer here has neither x264enc nor openh264enc to play the phone")?,
    };
    let sink = gst_app::AppSink::builder()
        .caps(
            &gst::Caps::builder("video/x-h264")
                .field("stream-format", "byte-stream")
                .field("alignment", "au")
                .build(),
        )
        .sync(false)
        .build();
    let pipeline = gst::Pipeline::new();
    pipeline.add_many([source.upcast_ref(), &encode, sink.upcast_ref()])?;
    gst::Element::link_many([source.upcast_ref(), &encode, sink.upcast_ref()])?;
    pipeline.set_state(gst::State::Playing)?;
    Ok((source, sink, pipeline))
}

/// A bar moving across a grey field, so every picture differs (I420).
fn paint(frame: &mut [u8], w: usize, h: usize, i: u64) {
    let bar = (i as usize * 8) % w;
    let (luma, chroma) = frame.split_at_mut(w * h);
    for y in 0..h {
        for x in 0..w {
            luma[y * w + x] = if x.abs_diff(bar) < 16 { 235 } else { 96 };
        }
    }
    chroma.fill(128);
}
