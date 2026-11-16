//! Where the iPhone's pictures go: the classic webcam device when the person made it (Chrome
//! with default settings, Zoom, Discord and Teams look only there), and Skykeep's own PipeWire
//! camera for GNOME's apps, OBS, Firefox and the desktop portal, unless PipeWire already lists
//! the webcam device as a camera itself. Each kind of program sees one iPhone camera.

use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::camera::Camera;
use super::decode::Picture;
use super::webcam::{self, Webcam};
use crate::stats::Delays;

pub struct Output {
    /// None when PipeWire already lists the webcam device as a camera.
    pipewire: Option<Camera>,
    webcam: Option<Mutex<Webcam>>,
    delays: Arc<Mutex<Delays>>,
    /// Black is being shown (the phone's camera off, paused, or the phone gone): sent again
    /// and again until a picture comes, as a webcam with its lens covered keeps sending. One
    /// black picture and then nothing was a camera that had stopped: Chromium and Meet showed
    /// it as broken, frozen, and Snapshot would not start on it.
    blank: std::sync::atomic::AtomicBool,
}

impl Output {
    pub fn start(
        pipewire_only: bool,
        name: &str,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Output> {
        let device = webcam::find().filter(|_| !pipewire_only);
        let webcam =
            device
                .as_ref()
                .and_then(|path| match Webcam::open(path, width, height, fps) {
                    Ok(w) => Some(Mutex::new(w)),
                    Err(e) => {
                        eprintln!("iPhone camera: the webcam device can't be used ({e:#})");
                        None
                    }
                });
        let listed = webcam.is_some() && device.as_deref().is_some_and(super::listed::lists);
        let pipewire = if listed {
            None
        } else {
            Some(Camera::start(name, width, height, fps)?)
        };
        let delays = pipewire
            .as_ref()
            .map(|c| c.delays.clone())
            .unwrap_or_default();
        Ok(Output {
            pipewire,
            webcam,
            delays,
            blank: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub fn show(&self, picture: Picture, captured_us: i64) {
        if captured_us != i64::MIN {
            self.blank
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(w) = &self.webcam {
            let wrote = w.lock().unwrap().show(&picture).is_ok();
            if wrote && self.pipewire.is_none() && captured_us != i64::MIN {
                self.delays.lock().unwrap().record(captured_us);
            }
        }
        if let Some(c) = &self.pipewire {
            c.show(picture, captured_us);
        }
    }

    /// Black from now until the phone's next picture, sent ten times a second by
    /// [`Output::keep_alive`].
    pub fn black(&self) {
        self.blank.store(true, std::sync::atomic::Ordering::Relaxed);
        self.one_black();
    }

    /// While black is being shown, it is sent again ten times a second, so programs reading
    /// the camera keep a live picture; the thread ends with the camera.
    pub fn keep_alive(camera: &Arc<Output>) {
        let weak = Arc::downgrade(camera);
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let Some(camera) = weak.upgrade() else { return };
            if camera.blank.load(std::sync::atomic::Ordering::Relaxed) {
                camera.one_black();
            }
        });
    }

    /// One black picture.
    fn one_black(&self) {
        if let Some(c) = &self.pipewire {
            c.black();
        }
        if let Some(w) = &self.webcam {
            let mut w = w.lock().unwrap();
            let (width, height) = w.size();
            let mut nv12 = vec![16u8; width * height * 3 / 2];
            nv12[width * height..].fill(128);
            let _ = w.show(&Picture {
                width: width as u32,
                height: height as u32,
                nv12,
            });
        }
    }

    pub fn delays(&self) -> Arc<Mutex<Delays>> {
        self.delays.clone()
    }
}
