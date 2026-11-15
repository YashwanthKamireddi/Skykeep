//! The phone's camera and microphone, as programs see them: a stand-in phone streams over a
//! real socket, and ordinary PipeWire readers (as OBS or a browser would be) open the camera
//! and the microphone and count what arrives. Needs a running PipeWire, as every desktop has.

#![cfg(target_os = "linux")]

use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use quince_link::{fakephone, receiver};
use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use spa::param::video::VideoFormat;
use spa::param::ParamType;
use spa::pod::{object, property, Pod, Value};
use spa::utils::SpaTypes;

struct Read_ {
    buffers: u64,
    /// Buffers whose time stamp moved on from the one before: readers that time their display
    /// freeze on a stream whose stamps stand still.
    advancing: u64,
    loud_samples: u64,
}

/// Opens the node named `target` as a reader would and returns what it read, once `enough`
/// buffers came or `within` passed.
fn read_node(target: &str, video: bool, enough: u64, within: Duration) -> Read_ {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).unwrap();
    let context = pw::context::ContextRc::new(&mainloop, None).unwrap();
    let core = context.connect_rc(None).unwrap();
    let props = if video {
        properties! { *pw::keys::MEDIA_TYPE => "Video", *pw::keys::MEDIA_CATEGORY => "Capture", *pw::keys::MEDIA_ROLE => "Camera", *pw::keys::TARGET_OBJECT => target }
    } else {
        properties! { *pw::keys::MEDIA_TYPE => "Audio", *pw::keys::MEDIA_CATEGORY => "Capture", *pw::keys::MEDIA_ROLE => "Communication", *pw::keys::TARGET_OBJECT => target, *pw::keys::NODE_LATENCY => "480/48000" }
    };
    let stream = pw::stream::StreamRc::new(core, "skykeep-test-reader", props).unwrap();
    let got = Arc::new(Mutex::new(Read_ {
        buffers: 0,
        advancing: 0,
        loud_samples: 0,
    }));
    let lt = Arc::new(Mutex::new(i64::MIN));
    let (g, quit) = (got.clone(), mainloop.clone());
    let _l = stream
        .add_local_listener_with_user_data(())
        .process(move |s, _| {
            let Some(mut b) = s.dequeue_buffer() else {
                return;
            };
            let time = b
                .find_meta::<spa::buffer::meta::MetaHeader>()
                .map(|h| h.pts());
            let datas = b.datas_mut();
            let size = datas[0].chunk().size() as usize;
            let mut r = g.lock().unwrap();
            if size > 0 {
                r.buffers += 1;
                if let Some(t) = time {
                    let mut last = lt.lock().unwrap();
                    if t > *last {
                        r.advancing += 1;
                    }
                    *last = t;
                }
            }
            if !video {
                if let Some(d) = datas[0].data() {
                    r.loud_samples += d[..size.min(d.len())]
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .filter(|x| i16::from_le_bytes([x[0], x[1]]).abs() > 1000)
                        .count() as u64;
                }
            }
            if r.buffers >= enough {
                quit.quit();
            }
        })
        .register()
        .unwrap();
    let format = if video {
        object! {
            SpaTypes::ObjectParamFormat, ParamType::EnumFormat,
            property!(FormatProperties::MediaType, Id, MediaType::Video),
            property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
            property!(FormatProperties::VideoFormat, Choice, Enum, Id, VideoFormat::NV12, VideoFormat::NV12, VideoFormat::I420),
        }
    } else {
        let mut info = spa::param::audio::AudioInfoRaw::new();
        info.set_format(spa::param::audio::AudioFormat::S16LE);
        info.set_rate(48_000);
        info.set_channels(1);
        spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }
    };
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(format),
    )
    .unwrap()
    .0
    .into_inner();
    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut [Pod::from_bytes(&bytes).unwrap()],
        )
        .unwrap();
    let q = mainloop.clone();
    let timer = mainloop.loop_().add_timer(move |_| q.quit());
    timer
        .update_timer(Some(within), None)
        .into_result()
        .unwrap();
    mainloop.run();
    let r = got.lock().unwrap();
    Read_ {
        buffers: r.buffers,
        advancing: r.advancing,
        loud_samples: r.loud_samples,
    }
}

/// Both tests publish devices under the same names, so they take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn pipewire_running() -> bool {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    std::path::Path::new(&dir).join("pipewire-0").exists()
}

#[test]
fn programs_see_the_phones_camera_and_microphone_with_little_delay() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    if !pipewire_running() {
        eprintln!("no PipeWire on this machine: skipped");
        return;
    }
    let phone = fakephone::start(1280, 720, 60).unwrap();
    let link = TcpStream::connect(("127.0.0.1", phone.port)).unwrap();
    link.set_nodelay(true).unwrap();
    let report = Arc::new(Mutex::new(receiver::Report::default()));
    {
        let (r, w, report) = (link.try_clone().unwrap(), link, report.clone());
        std::thread::spawn(move || {
            receiver::serve(
                r,
                w,
                report,
                Arc::new(receiver::Devices::pipewire_only()),
                &|_: receiver::CameraState| {},
            )
        });
    }

    // Wait for the camera to appear: its first picture, or give up.
    for _ in 0..200 {
        if report.lock().unwrap().pictures > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let camera = read_node("skykeep-iphone-camera", true, 120, Duration::from_secs(10));
    let mic = read_node(
        "skykeep-iphone-microphone",
        false,
        60,
        Duration::from_secs(10),
    );

    let r = report.lock().unwrap();
    let pictures = r.picture_delays.as_ref().unwrap().lock().unwrap();
    let sound = r.sound_delays.as_ref().unwrap().lock().unwrap();
    let ms = |v: Option<i64>| v.unwrap_or(-1) as f64 / 1000.0;
    eprintln!(
        "hardware decoding: {}; mic underruns {}; camera read {} pictures, capture-to-camera p50 {:.1} ms p95 {:.1} ms; mic read {} blocks ({} loud samples), capture-to-microphone p50 {:.1} ms p95 {:.1} ms; keyframes asked {}",
        r.hardware_decoding,
        r.mic_underruns.as_ref().map(|u| u.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0),
        camera.buffers,
        ms(pictures.percentile(0.5)),
        ms(pictures.percentile(0.95)),
        mic.buffers,
        mic.loud_samples,
        ms(sound.percentile(0.5)),
        ms(sound.percentile(0.95)),
        r.keyframes_asked,
    );
    assert!(
        camera.buffers >= 120,
        "the camera gave {} pictures",
        camera.buffers
    );
    assert!(
        mic.buffers >= 60 && mic.loud_samples > 48_000,
        "the microphone gave {} blocks",
        mic.buffers
    );
    // Encoding the test picture is part of the phone's time. Decoding and handing over must
    // stay well inside one 60 fps frame: with the sequence header's room raised (sps.rs) no
    // decoder holds a picture until the next, which alone added about 17 ms.
    assert!(
        pictures.percentile(0.95).unwrap() < 15_000,
        "pictures took too long"
    );
    // The computer's own audio block (43 ms on a machine set up for heavy noise removal) is
    // there for every microphone; what this side adds on top must stay small.
    let block_us = r
        .mic_block
        .as_ref()
        .unwrap()
        .load(std::sync::atomic::Ordering::Relaxed) as i64
        * 1_000_000
        / 48_000;
    let added = sound.percentile(0.95).unwrap() - block_us;
    eprintln!(
        "audio block {:.1} ms; added by the link {:.1} ms",
        block_us as f64 / 1000.0,
        added as f64 / 1000.0
    );
    assert!(added < 15_000, "the link added {added} µs of sound delay");
}

/// A program keeps the camera it chose while the phone is unplugged and plugged back in: the
/// same camera goes black, then shows the phone again, and the program never notices a gap.
#[test]
fn a_program_keeps_the_camera_across_an_unplug() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    if !pipewire_running() {
        eprintln!("no PipeWire on this machine: skipped");
        return;
    }
    let devices = Arc::new(receiver::Devices::pipewire_only());
    let report = Arc::new(Mutex::new(receiver::Report::default()));
    let connect = |devices: &Arc<receiver::Devices>| {
        let phone = fakephone::start(640, 360, 60).unwrap();
        let link = TcpStream::connect(("127.0.0.1", phone.port)).unwrap();
        let (r, w, report, devices) = (
            link.try_clone().unwrap(),
            link,
            report.clone(),
            devices.clone(),
        );
        let session = std::thread::spawn(move || {
            receiver::serve(r, w, report, devices, &|_: receiver::CameraState| {})
        });
        (phone, session)
    };
    let wait_for = |n: u64| {
        for _ in 0..400 {
            if report.lock().unwrap().pictures >= n {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!(
            "the phone's pictures stopped at {}",
            report.lock().unwrap().pictures
        );
    };

    let (phone, session) = connect(&devices);
    wait_for(1);
    // The reader opens the camera now and reads on through the unplug and the replug.
    let reader = std::thread::spawn(|| {
        read_node("skykeep-iphone-camera", true, 240, Duration::from_secs(20))
    });
    wait_for(120);
    drop(phone);
    session.join().unwrap().unwrap();
    let before = report.lock().unwrap().pictures;
    let (_phone, _session) = connect(&devices);
    wait_for(before + 150);
    let read = reader.join().unwrap();
    assert!(
        read.buffers >= 240,
        "the reader got {} pictures across the replug",
        read.buffers
    );
}
