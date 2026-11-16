//! One connected phone: its packets read, its sound sent straight on, its pictures decoded on
//! their own thread so a picture being decoded never holds sound back.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{sync_channel, TrySendError};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

use crate::linux::decode::Decoder;
use crate::linux::mic::Mic;
use crate::linux::output::Output as Camera;
use crate::protocol::{Hello, Kind, Packet, VideoFormat, KEYFRAME};
use crate::stats::{now_us, Delays};

/// Pictures waiting for the decoder. Decoding takes a few milliseconds and pictures come every
/// 16, so more than this waiting means the decoder can't keep up: the waiting ones are
/// dropped and decoding starts again at the next keyframe, which is asked for at once.
const WAITING: usize = 3;

/// The camera and microphone programs see. They outlive any one connection: a phone unplugged
/// and plugged back, or switching lenses, keeps the same devices, so a call or a recording that
/// chose them carries on instead of losing its camera.
/// The camera and the picture size it was made for.
type SizedCamera = (Arc<Camera>, (u32, u32));

#[derive(Default)]
pub struct Devices {
    /// Skykeep's own PipeWire camera even where the webcam device exists (the tests, whose
    /// readers look for it by name).
    pub pipewire_only: bool,
    camera: Mutex<Option<SizedCamera>>,
    mic: Mutex<Option<Arc<Mic>>>,
}

impl Devices {
    /// Devices that publish Skykeep's own PipeWire camera even where the webcam device exists.
    pub fn pipewire_only() -> Devices {
        Devices {
            pipewire_only: true,
            ..Default::default()
        }
    }

    fn camera(&self, name: &str, format: &VideoFormat) -> Result<Arc<Camera>> {
        let mut slot = self.camera.lock().unwrap();
        if let Some((camera, size)) = slot.as_ref() {
            if *size == (format.width, format.height) {
                return Ok(camera.clone());
            }
        }
        // Only a different picture size makes a new camera.
        *slot = None;
        let camera = Arc::new(Camera::start(
            self.pipewire_only,
            name,
            format.width,
            format.height,
            format.fps,
        )?);
        Camera::keep_alive(&camera);
        *slot = Some((camera.clone(), (format.width, format.height)));
        Ok(camera)
    }

    fn mic(&self, name: &str) -> Result<Arc<Mic>> {
        let mut slot = self.mic.lock().unwrap();
        if let Some(m) = slot.as_ref() {
            return Ok(m.clone());
        }
        let m = Arc::new(Mic::start(name)?);
        *slot = Some(m.clone());
        Ok(m)
    }

    /// The next picture format makes the camera afresh (and finds the webcam device if it
    /// was just made).
    pub fn forget_camera(&self) {
        *self.camera.lock().unwrap() = None;
    }

    /// The phone went away: the camera shows black until it's back (the microphone is silent
    /// by itself once its buffer runs dry).
    pub fn phone_gone(&self) {
        if let Some((camera, _)) = self.camera.lock().unwrap().as_ref() {
            camera.black();
        }
    }
}

/// The phone's camera, as the phone says: sending, paused by iOS (locked, or another app in
/// front without picture in picture), or turned off by the person (microphone only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraState {
    Live,
    Paused,
    Off,
}

/// What a session leaves behind for the person (and the tests) to read.
#[derive(Default)]
pub struct Report {
    pub name: String,
    pub hardware_decoding: bool,
    pub pictures: u64,
    pub picture_delays: Option<Arc<Mutex<Delays>>>,
    /// Capture to arrival here, before decoding: the phone's share of the delay plus the cable.
    pub arrival_delays: Arc<Mutex<Delays>>,
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
    devices: Arc<Devices>,
    camera_paused: &(dyn Fn(CameraState) + Sync),
) -> Result<()> {
    let result = session(from_phone, ask, report, &devices, camera_paused);
    devices.phone_gone();
    result
}

fn session(
    from_phone: impl Read,
    ask: impl Write + Send + 'static,
    report: Arc<Mutex<Report>>,
    devices: &Arc<Devices>,
    camera_paused: &(dyn Fn(CameraState) + Sync),
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

    // The phone stamps captures with its own clock. Over the cable a ping's round trip is well
    // under a millisecond, so the two clocks are matched to within that: the delays read true,
    // and the camera's and microphone's timing line up.
    let offset = Arc::new(AtomicI64::new(0));
    let best_round_trip = AtomicI64::new(i64::MAX);
    {
        let ask = ask.clone();
        std::thread::Builder::new()
            .name("iphone-clock".into())
            .spawn(move || {
                for i in 0u64.. {
                    let ping =
                        Packet::message(Kind::Control, &serde_json::json!({ "ping": now_us() }));
                    let mut a = ask.lock().unwrap();
                    if ping.write_to(&mut *a).and_then(|_| a.flush()).is_err() {
                        break;
                    }
                    drop(a);
                    // Five quick pings to start with, then one now and then for drift.
                    std::thread::sleep(std::time::Duration::from_millis(if i < 5 {
                        200
                    } else {
                        10_000
                    }));
                }
            })?;
    }
    let mut mic: Option<Arc<Mic>> = None;
    let mut pictures: Option<std::sync::mpsc::SyncSender<Packet>> = None;
    let mut decoder_thread = None;
    let mut skipping = false;
    // Set by the decoder when a picture couldn't be decoded: the ones after it can't be either,
    // so pictures wait for a keyframe, which is asked for at once.
    let broken = Arc::new(AtomicBool::new(false));
    let ask_keyframe = |report: &Arc<Mutex<Report>>| -> Result<()> {
        report.lock().unwrap().keyframes_asked += 1;
        let mut a = ask.lock().unwrap();
        Packet::message(Kind::Control, &serde_json::json!({ "keyframe": true }))
            .write_to(&mut *a)?;
        Ok(a.flush()?)
    };

    while let Some(packet) = Packet::read_from(&mut from_phone)? {
        match packet.kind {
            Kind::Audio => {
                if mic.is_none() {
                    let m = devices.mic(&format!("{} Microphone", hello.name))?;
                    {
                        let mut r = report.lock().unwrap();
                        r.sound_delays = Some(m.delays.clone());
                        r.mic_underruns = Some(m.underruns.clone());
                        r.mic_block = Some(m.block.clone());
                    }
                    mic = Some(m);
                }
                mic.as_ref().unwrap().hear(
                    &packet.body,
                    packet.captured_us - offset.load(Ordering::Relaxed),
                );
            }
            Kind::VideoFormat => {
                let format: VideoFormat = packet.json()?;
                drop(pictures.take());
                if let Some(t) = decoder_thread.take() {
                    let _ = std::thread::JoinHandle::join(t);
                }
                let (tx, rx) = sync_channel::<Packet>(WAITING);
                let name = format!("{} Camera", hello.name);
                let camera = devices.camera(&name, &format)?;
                let (for_decoder, broken_flag, clock) =
                    (report.clone(), broken.clone(), offset.clone());
                decoder_thread = Some(
                    std::thread::Builder::new()
                        .name("iphone-decode".into())
                        .spawn(move || {
                            if let Err(e) = decode_pictures(
                                format,
                                &camera,
                                rx,
                                for_decoder,
                                &broken_flag,
                                &clock,
                            ) {
                                eprintln!("iPhone camera stopped: {e:#}");
                            }
                        })?,
                );
                pictures = Some(tx);
                // A new decoder starts at a keyframe (a lens change sends a new format mid-stream).
                skipping = true;
                ask_keyframe(&report)?;
            }
            Kind::Video => {
                report
                    .lock()
                    .unwrap()
                    .arrival_delays
                    .lock()
                    .unwrap()
                    .record(packet.captured_us - offset.load(Ordering::Relaxed));
                let Some(tx) = &pictures else { continue };
                if broken.swap(false, Ordering::Relaxed) {
                    skipping = true;
                    ask_keyframe(&report)?;
                }
                if skipping && packet.flags & KEYFRAME == 0 {
                    continue;
                }
                skipping = false;
                match tx.try_send(packet) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        skipping = true;
                        ask_keyframe(&report)?;
                    }
                    Err(TrySendError::Disconnected(_)) => bail!("the camera stopped"),
                }
            }
            Kind::Control => {
                let reply: serde_json::Value = packet.json().unwrap_or_default();
                // iOS stopped the camera (the phone locked, or another app in front with no
                // picture in picture): black, not the last picture frozen, and said why.
                match reply["camera"].as_str() {
                    Some("paused") => {
                        devices.phone_gone();
                        camera_paused(CameraState::Paused);
                    }
                    Some("off") => {
                        devices.phone_gone();
                        camera_paused(CameraState::Off);
                    }
                    Some("live") => camera_paused(CameraState::Live),
                    _ => {}
                }
                if let (Some(sent), Some(phone)) = (reply["pong"].as_i64(), reply["phone"].as_i64())
                {
                    let round_trip = now_us() - sent;
                    if round_trip < best_round_trip.load(Ordering::Relaxed) {
                        best_round_trip.store(round_trip, Ordering::Relaxed);
                        offset.store(phone - (sent + round_trip / 2), Ordering::Relaxed);
                    }
                }
            }
            Kind::Hello => {}
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
    camera: &Arc<Camera>,
    rx: std::sync::mpsc::Receiver<Packet>,
    report: Arc<Mutex<Report>>,
    broken: &AtomicBool,
    clock: &AtomicI64,
) -> Result<()> {
    let (cam, counted) = (camera.clone(), report.clone());
    let (width, height) = (format.width, format.height);
    let mut decoder = Decoder::new(format.codec, move |picture, captured| {
        if picture.width == width && picture.height == height {
            cam.show(picture, captured);
            counted.lock().unwrap().pictures += 1;
        }
    })?;
    {
        let mut r = report.lock().unwrap();
        r.hardware_decoding = decoder.hardware;
        r.picture_delays = Some(camera.delays());
    }
    for packet in rx {
        let captured = packet.captured_us - clock.load(Ordering::Relaxed);
        // A keyframe's sequence header gets one more picture of room, so the decoder hands
        // each picture on at once instead of the next time one arrives (sps.rs).
        let roomier = (packet.flags & KEYFRAME != 0)
            .then(|| crate::sps::unit_with_room(&packet.body))
            .flatten();
        let unit = roomier.as_deref().unwrap_or(&packet.body);
        if let Err(e) = decoder.decode(unit, captured) {
            // One damaged picture: start again cleanly at the next keyframe.
            eprintln!("iPhone camera: {e:#}; waiting for a keyframe");
            decoder.flush();
            broken.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}
