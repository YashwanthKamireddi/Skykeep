//! A stand-in for the phone app, for tests and for measuring this side alone: a moving test
//! picture encoded as the phone encodes (H.264, no B-frames, real-time settings) and a tone in
//! 5 ms blocks, each stamped with this computer's clock so delays read directly.

use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ffmpeg_next as ff;

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

    ff::init()?;
    let codec = ff::encoder::find_by_name("libx264")
        .context("FFmpeg here has no libx264 to play the phone")?;
    let mut enc = ff::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()?;
    enc.set_width(width);
    enc.set_height(height);
    enc.set_format(ff::format::Pixel::YUV420P);
    enc.set_time_base((1, fps as i32));
    enc.set_max_b_frames(0);
    enc.set_gop(fps * 2);
    let mut opts = ff::Dictionary::new();
    opts.set("preset", "ultrafast");
    opts.set("tune", "zerolatency");
    let mut enc = enc.open_with(opts)?;

    let mut frame = ff::frame::Video::new(ff::format::Pixel::YUV420P, width, height);
    let start = Instant::now();
    let mut i: i64 = 0;
    while !stop.load(Ordering::Relaxed) {
        paint(&mut frame, i);
        frame.set_pts(Some(i));
        frame.set_kind(if want_key.swap(false, Ordering::Relaxed) {
            ff::picture::Type::I
        } else {
            ff::picture::Type::None
        });
        let captured = now_us();
        enc.send_frame(&frame)?;
        let mut packet = ff::Packet::empty();
        while enc.receive_packet(&mut packet).is_ok() {
            let key = if packet.is_key() { KEYFRAME } else { 0 };
            send(&Packet {
                kind: Kind::Video,
                flags: key,
                captured_us: captured,
                body: packet.data().unwrap_or(&[]).to_vec(),
            })?;
        }
        i += 1;
        let next = start + Duration::from_micros(i as u64 * 1_000_000 / fps as u64);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    // Hung up, as an unplugged cable does: the computer sees the link close at once.
    let _ = out.lock().unwrap().shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// A bar moving across a grey field, so every picture differs.
fn paint(frame: &mut ff::frame::Video, i: i64) {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let stride = frame.stride(0);
    let bar = (i as usize * 8) % w;
    let luma = frame.data_mut(0);
    for y in 0..h {
        for x in 0..w {
            luma[y * stride + x] = if x.abs_diff(bar) < 16 { 235 } else { 96 };
        }
    }
    for plane in 1..3 {
        let s = frame.stride(plane);
        let d = frame.data_mut(plane);
        for y in 0..h / 2 {
            d[y * s..y * s + w / 2].fill(128);
        }
    }
}
