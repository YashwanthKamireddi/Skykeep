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
}

struct State {
    latest: Option<(Picture, i64)>,
    format: VideoFormat,
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
        })
    }

    /// Hands a picture on. Only the newest one waits: a picture still waiting when the next
    /// arrives is replaced, so a slow reader never builds a queue of old pictures.
    pub fn show(&self, picture: Picture, captured_us: i64) {
        let _ = self.to_loop.send(Message::Picture(picture, captured_us));
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
        },
    )?;

    let state = Rc::new(RefCell::new(State {
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
            let _ = stream.update_params(&mut [Pod::from_bytes(&buffers).unwrap()]);
        })
        .process(|stream, state| {
            let mut st = state.borrow_mut();
            let Some((picture, captured)) = st.latest.take() else {
                return;
            };
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let data = &mut datas[0];
            let format = st.format;
            let Some(out) = data.data() else { return };
            let n = picture.nv12.len().min(out.len());
            let stride = match format {
                VideoFormat::I420 => {
                    write_i420(&picture, out);
                    picture.width
                }
                _ => {
                    out[..n].copy_from_slice(&picture.nv12[..n]);
                    picture.width
                }
            };
            let chunk = data.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = stride as i32;
            *chunk.size_mut() = n as u32;
            st.delays.lock().unwrap().record(captured);
        })
        .register()?;

    let format = format_pod(width, height, fps);
    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::DRIVER | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut [Pod::from_bytes(&format).unwrap()],
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

fn format_pod(width: u32, height: u32, fps: u32) -> Vec<u8> {
    let _ = VideoInfoRaw::new();
    let obj = object! {
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        property!(FormatProperties::MediaType, Id, MediaType::Video),
        property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        property!(FormatProperties::VideoFormat, Choice, Enum, Id, VideoFormat::NV12, VideoFormat::NV12, VideoFormat::I420),
        property!(FormatProperties::VideoSize, Rectangle, Rectangle { width, height }),
        property!(FormatProperties::VideoFramerate, Fraction, Fraction { num: fps, denom: 1 }),
    };
    serialize(obj)
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
            int(spa::sys::SPA_PARAM_BUFFERS_buffers, 4),
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
