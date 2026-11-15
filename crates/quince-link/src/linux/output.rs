//! Where the iPhone's pictures go: always Skykeep's own PipeWire camera (GNOME's apps, OBS,
//! Firefox and the desktop portal look there), and also the classic webcam device when the
//! person made it (Chrome with default settings, Zoom, Discord and Teams look only there).
//!
//! PipeWire doesn't list the webcam device itself: it is made before a picture format is set,
//! so the session manager takes it for an output and leaves it out. Each kind of program
//! therefore sees one iPhone camera, the one it looks for.

use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::camera::Camera;
use super::decode::Picture;
use super::webcam::{self, Webcam};
use crate::stats::Delays;

pub struct Output {
    pipewire: Camera,
    webcam: Option<Mutex<Webcam>>,
}

impl Output {
    pub fn start(
        pipewire_only: bool,
        name: &str,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Output> {
        let webcam = webcam::find()
            .filter(|_| !pipewire_only)
            .and_then(|path| match Webcam::open(&path, width, height, fps) {
                Ok(w) => Some(Mutex::new(w)),
                Err(e) => {
                    eprintln!("iPhone camera: the webcam device can't be used ({e:#})");
                    None
                }
            });
        Ok(Output {
            pipewire: Camera::start(name, width, height, fps)?,
            webcam,
        })
    }

    pub fn show(&self, picture: Picture, captured_us: i64) {
        if let Some(w) = &self.webcam {
            let _ = w.lock().unwrap().show(&picture);
        }
        self.pipewire.show(picture, captured_us);
    }

    /// One black picture: what programs keep showing while the phone is away.
    pub fn black(&self) {
        self.pipewire.black();
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
        self.pipewire.delays.clone()
    }
}
