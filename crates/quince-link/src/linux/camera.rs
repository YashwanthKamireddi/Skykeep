//! The iPhone as a camera every Linux program can choose: a PipeWire video source.
//!
//! The source drives its own timing: each picture is handed on the moment it is decoded
//! (`trigger_process`), never held for a clock tick, so it adds no delay of its own. OBS, the
//! desktop portal and browsers with PipeWire cameras on read it directly.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{anyhow, Result};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use spa::param::video::{VideoFormat, VideoInfoRaw};
use spa::param::ParamType;
use spa::pod::{object, property, Pod, Property, PropertyFlags, Value};
use spa::utils::{Fraction, Rectangle, SpaTypes};

use super::decode::Picture;
use crate::stats::Delays;

enum Message {
    Picture(Picture, i64),
    Stop,
}

pub struct Camera {
    to_loop: pw::channel::Sender<Message>,
    thread: Option<JoinHandle<()>>,
    /// Phone capture to picture handed to PipeWire, per picture.
    pub delays: Arc<Mutex<Delays>>,
    size: Mutex<Option<(u32, u32)>>,
}

struct State {
    latest: Option<(Picture, i64)>,
    format: VideoFormat,
    sequence: u64,
    delays: Arc<Mutex<Delays>>,
}

impl Camera {
    /// Publishes the camera, named after the phone. Pictures must then be `width`×`height`.
    pub fn start(name: &str, width: u32, height: u32, fps: u32) -> Result<Camera> {
        let (to_loop, from_others) = pw::channel::channel::<Message>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let delays = Arc::new(Mutex::new(Delays::default()));
        let shared = delays.clone();
        let name = name.to_owned();
        let thread = std::thread::Builder::new()
            .name("iphone-camera".into())
            .spawn(move || {
                if let Err(e) = run(&name, width, height, fps, from_others, shared, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
        ready_rx
            .recv()
            .map_err(|_| anyhow!("the camera thread ended before it started"))??;
        Ok(Camera {
            to_loop,
            thread: Some(thread),
            delays,
            size: Mutex::new(Some((width, height))),
        })
    }

    /// Hands a picture on. Only the newest one waits: a picture still waiting when the next
    /// arrives is replaced, so a slow reader never builds a queue of old pictures.
    pub fn show(&self, picture: Picture, captured_us: i64) {
        let _ = self.to_loop.send(Message::Picture(picture, captured_us));
    }
}

impl Camera {
    /// One black picture: what programs keep showing while the phone is away.
    pub fn black(&self) {
        let Some((w, h)) = *self.size.lock().unwrap() else {
            return;
        };
        let (w, h) = (w as usize, h as usize);
        let mut nv12 = vec![16u8; w * h * 3 / 2];
        nv12[w * h..].fill(128);
        // No capture time: nothing to count in the delays.
        self.show(
            Picture {
                width: w as u32,
                height: h as u32,
                nv12,
            },
            i64::MIN,
        );
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.to_loop.send(Message::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    name: &str,
    width: u32,
    height: u32,
    fps: u32,
    from_others: pw::channel::Receiver<Message>,
    delays: Arc<Mutex<Delays>>,
    ready: &std::sync::mpsc::Sender<Result<()>>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;
    let stream = pw::stream::StreamRc::new(
        core,
        "skykeep-iphone-camera",
        properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Camera",
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::NODE_NAME => "skykeep-iphone-camera",
            *pw::keys::NODE_DESCRIPTION => name,
            // Lower than any built-in device's, so the session manager never makes the iPhone
            // the default by itself: programs list it, the person picks it.
            *pw::keys::PRIORITY_SESSION => "100",
        },
    )?;

    let state = Rc::new(RefCell::new(State {
        sequence: 0,
        latest: None,
        format: VideoFormat::NV12,
        delays,
    }));

    let _listener = stream
        .add_local_listener_with_user_data(state.clone())
        .param_changed(move |stream, state, id, param| {
            let Some(param) = param else { return };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let mut info = VideoInfoRaw::new();
            if info.parse(param).is_err() {
                return;
            }
            state.borrow_mut().format = info.format();
            let size = (width * height * 3 / 2) as i32;
            let buffers = buffers_pod(size, width as i32);
            let header = header_pod();
            let _ = stream.update_params(&mut [
                Pod::from_bytes(&buffers).unwrap(),
                Pod::from_bytes(&header).unwrap(),
            ]);
        })
        .process(|stream, state| {
            let mut st = state.borrow_mut();
            let Some((picture, captured)) = st.latest.take() else {
                return;
            };
            let format = st.format;
            st.sequence += 1;
            let sequence = st.sequence;
            // SAFETY: the buffer is dequeued from this stream, written within its mapped size,
            // and queued back before returning, all on the stream's own thread.
            let written = unsafe { fill(stream.as_raw_ptr(), &picture, format, sequence) };
            if written && captured != i64::MIN {
                st.delays.lock().unwrap().record(captured);
            }
        })
        .register()?;

    let nv12 = format_pod(VideoFormat::NV12, width, height, fps);
    let i420 = format_pod(VideoFormat::I420, width, height, fps);
    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut [
            Pod::from_bytes(&nv12).unwrap(),
            Pod::from_bytes(&i420).unwrap(),
        ],
    )?;

    let loop_ = mainloop.clone();
    let for_pictures = stream.clone();
    let _receiver = from_others.attach(mainloop.loop_(), move |message| match message {
        Message::Picture(picture, captured) => {
            state.borrow_mut().latest = Some((picture, captured));
            let _ = for_pictures.trigger_process();
        }
        Message::Stop => loop_.quit(),
    });

    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}

/// Writes the picture into the next free buffer and hands it to the readers, stamped with the
/// time and a sequence number. The stamp matters: readers that time their display (video
/// windows, call apps) treat buffers with one unchanging time as one picture and freeze on it.
unsafe fn fill(
    stream: *mut pw::sys::pw_stream,
    picture: &Picture,
    format: VideoFormat,
    sequence: u64,
) -> bool {
    let raw = pw::sys::pw_stream_dequeue_buffer(stream);
    if raw.is_null() {
        // Every buffer is still with a reader: this picture is skipped, the next one goes.
        return false;
    }
    let buffer = (*raw).buffer;
    let data = &mut *(*buffer).datas;
    if data.data.is_null() {
        pw::sys::pw_stream_queue_buffer(stream, raw);
        return false;
    }
    let out = std::slice::from_raw_parts_mut(data.data as *mut u8, data.maxsize as usize);
    let n = picture.nv12.len().min(out.len());
    match format {
        VideoFormat::I420 => write_i420(picture, out),
        _ => out[..n].copy_from_slice(&picture.nv12[..n]),
    }
    let chunk = &mut *data.chunk;
    chunk.offset = 0;
    chunk.stride = picture.width as i32;
    chunk.size = n as u32;
    let header = spa::sys::spa_buffer_find_meta_data(
        buffer,
        spa::sys::SPA_META_Header,
        std::mem::size_of::<spa::sys::spa_meta_header>(),
    ) as *mut spa::sys::spa_meta_header;
    if !header.is_null() {
        (*header).pts = pw::sys::pw_stream_get_nsec(stream) as i64;
        (*header).flags = 0;
        (*header).seq = sequence;
        (*header).dts_offset = 0;
    }
    pw::sys::pw_stream_queue_buffer(stream, raw);
    true
}

/// NV12's interleaved colour plane split into I420's two, for readers that only take I420.
fn write_i420(picture: &Picture, out: &mut [u8]) {
    let (w, h) = (picture.width as usize, picture.height as usize);
    let luma = w * h;
    if out.len() < luma * 3 / 2 {
        return;
    }
    out[..luma].copy_from_slice(&picture.nv12[..luma]);
    let quarter = luma / 4;
    let (u, v) = out[luma..luma + 2 * quarter].split_at_mut(quarter);
    for (i, pair) in picture.nv12[luma..]
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .take(quarter)
    {
        u[i] = pair[0];
        v[i] = pair[1];
    }
}

/// One picture format the camera offers, listed on its own: browsers (Chromium's PipeWire
/// camera) read one plain format per entry, as webcams list them, and skip a camera whose
/// formats come as one choice.
fn format_pod(format: VideoFormat, width: u32, height: u32, fps: u32) -> Vec<u8> {
    let obj = object! {
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        property!(FormatProperties::MediaType, Id, MediaType::Video),
        property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        property!(FormatProperties::VideoFormat, Id, format),
        property!(FormatProperties::VideoSize, Rectangle, Rectangle { width, height }),
        // Any rate up to the phone's, as webcams offer several: programs that take 30 at most
        // (GNOME's Snapshot) skip a camera that offers only 60. Pictures still go out as the
        // phone sends them.
        property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: fps, denom: 1 },
            Fraction { num: 1, denom: 1 },
            Fraction { num: fps, denom: 1 }
        ),
    };
    serialize(obj)
}

/// Asks for a header on every buffer, to carry each picture's time and sequence number.
fn header_pod() -> Vec<u8> {
    serialize(spa::pod::Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamMeta,
        id: spa::sys::SPA_PARAM_Meta,
        properties: vec![
            Property {
                key: spa::sys::SPA_PARAM_META_type,
                flags: PropertyFlags::empty(),
                value: Value::Id(spa::utils::Id(spa::sys::SPA_META_Header)),
            },
            Property {
                key: spa::sys::SPA_PARAM_META_size,
                flags: PropertyFlags::empty(),
                value: Value::Int(std::mem::size_of::<spa::sys::spa_meta_header>() as i32),
            },
        ],
    })
}

fn buffers_pod(size: i32, stride: i32) -> Vec<u8> {
    let int = |key: u32, v: i32| Property {
        key,
        flags: PropertyFlags::empty(),
        value: Value::Int(v),
    };
    serialize(spa::pod::Object {
        type_: spa::sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa::sys::SPA_PARAM_Buffers,
        properties: vec![
            // Readers that show the picture hold several buffers at once (a video window keeps
            // the one on screen and the ones being uploaded); with four, pictures waited for a
            // free buffer and a window showed 20 of 60 a second. Ten keeps a full 60.
            int(spa::sys::SPA_PARAM_BUFFERS_buffers, 10),
            int(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(spa::sys::SPA_PARAM_BUFFERS_size, size),
            int(spa::sys::SPA_PARAM_BUFFERS_stride, stride),
        ],
    })
}

fn serialize(obj: spa::pod::Object) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(obj),
    )
    .expect("a format always serializes")
    .0
    .into_inner()
}
