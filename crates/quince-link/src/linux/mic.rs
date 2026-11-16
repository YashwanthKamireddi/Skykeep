//! The iPhone as a microphone every Linux program can choose: a PipeWire audio source.
//!
//! Sound arrives in blocks a few milliseconds long, not quite evenly, and is played out at the
//! pace of the computer's own audio clock. Between the two sits the shortest buffer that
//! doesn't break up, sized the way WebRTC sizes its own: from the longest recent gap between
//! arrivals (which jumps up at once and fades over seconds), plus a small margin.
//!
//! Surplus is never cut when sound arrives: a burst after a late block is exactly the sound
//! the next read needs, and cutting it there was what made the gaps. Only when the buffer has
//! stayed above its size for a whole second is the excess dropped, so lasting delay can't
//! build up either.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{anyhow, Result};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use spa::pod::{Pod, Value};

use crate::protocol::AUDIO_RATE;
use crate::stats::Delays;

/// Before any gaps are measured.
const START_MS: usize = 10;
const MOST_MS: usize = 60;
/// Added to the longest recent gap.
const MARGIN_MS: f64 = 2.0;
/// The longest gap fades by this much each block (to half in about three seconds).
const FADE: f64 = 0.998;
const PER_MS: usize = AUDIO_RATE as usize / 1000;

/// What the receiver writes and the audio thread reads.
struct Shared {
    samples: VecDeque<i16>,
    /// Capture time of the oldest sample waiting, for the delay figures.
    oldest_captured_us: i64,
    /// Margin beyond one block for the link's unevenness, in samples.
    target: usize,
    last_arrival_us: i64,
    /// The longest recent gap between arrivals, fading, in microseconds.
    longest_gap_us: f64,
    /// The fewest samples waiting at any read this second.
    lowest: usize,
    reads: usize,
    /// The block size programs read now, in samples.
    cycle: usize,
    playing: bool,
}

pub struct Mic {
    shared: Arc<Mutex<Shared>>,
    stop: pw::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
    pub delays: Arc<Mutex<Delays>>,
    /// Times the buffer ran dry and a moment of silence was played.
    pub underruns: Arc<AtomicU64>,
    /// The block size programs read now, in samples: the delay the computer's own audio
    /// setup adds whatever the source, which ours comes on top of.
    pub block: Arc<AtomicU64>,
}

impl Mic {
    pub fn start(name: &str) -> Result<Mic> {
        let shared = Arc::new(Mutex::new(Shared {
            samples: VecDeque::with_capacity(MOST_MS * PER_MS * 2),
            oldest_captured_us: 0,
            target: START_MS * PER_MS,
            last_arrival_us: 0,
            longest_gap_us: 0.0,
            lowest: usize::MAX,
            reads: 0,
            cycle: 0,
            playing: false,
        }));
        let delays = Arc::new(Mutex::new(Delays::default()));
        let (stop, stop_rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let underruns = Arc::new(AtomicU64::new(0));
        let block = Arc::new(AtomicU64::new(0));
        let (s, d, u, b, name) = (
            shared.clone(),
            delays.clone(),
            underruns.clone(),
            block.clone(),
            name.to_owned(),
        );
        let thread = std::thread::Builder::new()
            .name("iphone-mic".into())
            .spawn(move || {
                if let Err(e) = run(&name, s, d, u, b, stop_rx, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
        ready_rx
            .recv()
            .map_err(|_| anyhow!("the microphone thread ended before it started"))??;
        Ok(Mic {
            shared,
            stop,
            thread: Some(thread),
            delays,
            underruns,
            block,
        })
    }

    /// Adds a block of samples the phone captured at `captured_us` (its first sample).
    pub fn hear(&self, block: &[u8], captured_us: i64) {
        let mut s = self.shared.lock().unwrap();
        if s.samples.is_empty() {
            s.oldest_captured_us = captured_us;
        }
        s.samples.extend(
            block
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b)),
        );
        let now = crate::stats::now_us();
        if s.last_arrival_us > 0 {
            let gap = (now - s.last_arrival_us) as f64;
            s.longest_gap_us = (s.longest_gap_us * FADE).max(gap);
            let ms = s.longest_gap_us / 1000.0 + MARGIN_MS;
            s.target = ((ms * PER_MS as f64) as usize).clamp(2 * PER_MS, MOST_MS * PER_MS);
        }
        s.last_arrival_us = now;
        // While nothing reads, sound older than one read's worth plus the margin is of no use:
        // a program opening the microphone hears now, not what was said before it opened it.
        // While reading, a hard ceiling only, for a reader that stopped altogether.
        let most = if s.playing {
            s.cycle + (MOST_MS * 2) * PER_MS
        } else {
            s.cycle + s.target
        };
        if s.samples.len() > most {
            let extra = s.samples.len() - most;
            s.samples.drain(..extra);
            s.oldest_captured_us = captured_us - (s.samples.len() as i64 * 1000 / PER_MS as i64);
        }
    }
}

impl Drop for Mic {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    name: &str,
    shared: Arc<Mutex<Shared>>,
    delays: Arc<Mutex<Delays>>,
    underruns: Arc<AtomicU64>,
    block: Arc<AtomicU64>,
    stop: pw::channel::Receiver<()>,
    ready: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let stream = pw::stream::StreamRc::new(
        core,
        "skykeep-iphone-microphone",
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Communication",
            *pw::keys::MEDIA_CLASS => "Audio/Source",
            *pw::keys::NODE_NAME => "skykeep-iphone-microphone",
            *pw::keys::NODE_DESCRIPTION => name,
            *pw::keys::AUDIO_CHANNELS => "1",
            // A short cycle: each block handed to programs is 2.7 ms of sound.
            *pw::keys::NODE_LATENCY => "128/48000",
            // Lower than any built-in device's, so the session manager never makes the iPhone
            // the default by itself: programs list it, the person picks it.
            *pw::keys::PRIORITY_SESSION => "100",
        },
    )?;

    let _listener = stream
        .add_local_listener_with_user_data(())
        .process(move |stream, _| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            let data = &mut datas[0];
            let Some(out) = data.data() else { return };
            let frames = if requested > 0 {
                requested.min(out.len() / 2)
            } else {
                out.len() / 2
            };
            let mut s = shared.lock().unwrap();
            // A program reading in large blocks needs a whole block ready each time: that, plus
            // the margin for the link's unevenness, is the least the buffer can be.
            s.cycle = frames;
            block.store(frames as u64, Ordering::Relaxed);
            if !s.playing && s.samples.len() >= frames + s.target {
                s.playing = true;
            }
            // A second of reads that all found more than needed: the surplus is lasting delay,
            // and goes now, between reads, where nothing is waiting for it.
            if s.playing {
                s.lowest = s.lowest.min(s.samples.len());
                s.reads += 1;
                if s.reads * frames >= AUDIO_RATE as usize {
                    let keep = frames + s.target;
                    if s.lowest > keep {
                        let extra = s.lowest - keep;
                        s.samples.drain(..extra);
                        s.oldest_captured_us += extra as i64 * 1000 / PER_MS as i64;
                    }
                    s.lowest = usize::MAX;
                    s.reads = 0;
                }
            }
            let have = if s.playing {
                s.samples.len().min(frames)
            } else {
                0
            };
            if have > 0 {
                delays.lock().unwrap().record(s.oldest_captured_us);
            }
            for (i, slot) in out[..frames * 2]
                .as_chunks_mut::<2>()
                .0
                .iter_mut()
                .enumerate()
            {
                let v = if i < have {
                    s.samples.pop_front().unwrap_or(0)
                } else {
                    0
                };
                slot.copy_from_slice(&v.to_le_bytes());
            }
            if have > 0 {
                s.oldest_captured_us += (have as i64) * 1000 / PER_MS as i64;
            }
            if s.playing && have < frames {
                // Ran dry: wait for a slightly deeper buffer before playing on.
                underruns.fetch_add(1, Ordering::Relaxed);
                s.playing = false;
                s.longest_gap_us += 1000.0;
            }
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = 2;
            *chunk.size_mut() = (frames * 2) as u32;
        })
        .register()?;

    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::S16LE);
    info.set_rate(AUDIO_RATE);
    info.set_channels(1);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_MONO;
    info.set_position(position);
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )?
    .0
    .into_inner();
    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut [Pod::from_bytes(&bytes).unwrap()],
    )?;

    let loop_ = mainloop.clone();
    let _stop = stop.attach(mainloop.loop_(), move |()| loop_.quit());
    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}
