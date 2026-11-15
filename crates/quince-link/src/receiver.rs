//! One connected phone: its packets read, its sound sent straight on, its pictures decoded on
//! their own thread so a picture being decoded never holds sound back.

use std::io::{Read, Write};
use std::sync::mpsc::{sync_channel, TrySendError};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

use crate::linux::camera::Camera;
use crate::linux::decode::Decoder;
use crate::linux::mic::Mic;
use crate::protocol::{Hello, Kind, Packet, VideoFormat, KEYFRAME};
use crate::stats::Delays;

/// Pictures waiting for the decoder. Decoding takes a few milliseconds and pictures come every
/// 16, so more than this waiting means the decoder can't keep up: the waiting ones are
/// dropped and decoding starts again at the next keyframe, which is asked for at once.
const WAITING: usize = 3;

/// What a session leaves behind for the person (and the tests) to read.
#[derive(Default)]
pub struct Report {
    pub name: String,
    pub hardware_decoding: bool,
    pub pictures: u64,
    pub picture_delays: Option<Arc<Mutex<Delays>>>,
    pub sound_delays: Option<Arc<Mutex<Delays>>>,
    pub keyframes_asked: u64,
    pub mic_underruns: Option<Arc<std::sync::atomic::AtomicU64>>,
    pub mic_block: Option<Arc<std::sync::atomic::AtomicU64>>,
}

/// Serves one phone until it disconnects. `ask` carries requests back to the phone.
pub fn serve(
    from_phone: impl Read,
    ask: impl Write + Send + 'static,
    report: Arc<Mutex<Report>>,
) -> Result<()> {
    // Packets are read in pieces; a buffer makes that one system call per many packets.
    let mut from_phone = std::io::BufReader::with_capacity(1 << 20, from_phone);
    let hello = Packet::read_from(&mut from_phone)?
        .context("the phone closed the link before saying hello")?;
    if hello.kind != Kind::Hello {
        bail!("the phone's first packet wasn't a hello");
    }
    let hello: Hello = hello.json()?;
    report.lock().unwrap().name = hello.name.clone();

    let ask = Arc::new(Mutex::new(ask));
    let mut mic: Option<Mic> = None;
    let mut pictures: Option<std::sync::mpsc::SyncSender<Packet>> = None;
    let mut decoder_thread = None;
    let mut skipping = false;

    while let Some(packet) = Packet::read_from(&mut from_phone)? {
        match packet.kind {
            Kind::Audio => {
                if mic.is_none() {
                    let m = Mic::start(&format!("{} Microphone", hello.name))?;
                    {
                        let mut r = report.lock().unwrap();
                        r.sound_delays = Some(m.delays.clone());
                        r.mic_underruns = Some(m.underruns.clone());
                        r.mic_block = Some(m.block.clone());
                    }
                    mic = Some(m);
                }
                mic.as_ref().unwrap().hear(&packet.body, packet.captured_us);
            }
            Kind::VideoFormat => {
                let format: VideoFormat = packet.json()?;
                drop(pictures.take());
                if let Some(t) = decoder_thread.take() {
                    let _ = std::thread::JoinHandle::join(t);
                }
                let (tx, rx) = sync_channel::<Packet>(WAITING);
                let name = format!("{} Camera", hello.name);
                let report = report.clone();
                decoder_thread = Some(
                    std::thread::Builder::new()
                        .name("iphone-decode".into())
                        .spawn(move || {
                            if let Err(e) = decode_pictures(format, &name, rx, report) {
                                eprintln!("iPhone camera stopped: {e:#}");
                            }
                        })?,
                );
                pictures = Some(tx);
                skipping = false;
            }
            Kind::Video => {
                let Some(tx) = &pictures else { continue };
                if skipping && packet.flags & KEYFRAME == 0 {
                    continue;
                }
                skipping = false;
                match tx.try_send(packet) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        skipping = true;
                        report.lock().unwrap().keyframes_asked += 1;
                        let mut a = ask.lock().unwrap();
                        Packet::message(Kind::Control, &serde_json::json!({ "keyframe": true }))
                            .write_to(&mut *a)?;
                        a.flush()?;
                    }
                    Err(TrySendError::Disconnected(_)) => bail!("the camera stopped"),
                }
            }
            Kind::Hello | Kind::Control => {}
        }
    }
    drop(pictures);
    if let Some(t) = decoder_thread {
        let _ = t.join();
    }
    Ok(())
}

fn decode_pictures(
    format: VideoFormat,
    name: &str,
    rx: std::sync::mpsc::Receiver<Packet>,
    report: Arc<Mutex<Report>>,
) -> Result<()> {
    let mut decoder = Decoder::new(format.codec)?;
    let camera = Camera::start(name, format.width, format.height, format.fps)?;
    {
        let mut r = report.lock().unwrap();
        r.hardware_decoding = decoder.hardware;
        r.picture_delays = Some(camera.delays.clone());
    }
    for packet in rx {
        if let Some(picture) = decoder.decode(&packet.body)? {
            if picture.width == format.width && picture.height == format.height {
                camera.show(picture, packet.captured_us);
                report.lock().unwrap().pictures += 1;
            }
        }
    }
    Ok(())
}
