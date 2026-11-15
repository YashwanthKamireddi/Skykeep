//! Where the iPhone's pictures go: the classic webcam device when the person made it (every
//! program sees that, and PipeWire lists it for the rest), Skykeep's own PipeWire camera
//! otherwise. Never both, so programs list one iPhone camera.

use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::camera::Camera;
use super::decode::Picture;
use super::webcam::{self, Webcam};
use crate::stats::Delays;

pub enum Output {
    PipeWire(Camera),
    Webcam {
        device: Mutex<Webcam>,
        size: (u32, u32),
        delays: Arc<Mutex<Delays>>,
    },
}

impl Output {
    pub fn start(
        pipewire_only: bool,
        name: &str,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<Output> {
        if let Some(path) = webcam::find().filter(|_| !pipewire_only) {
            match Webcam::open(&path, width, height, fps) {
                Ok(device) => {
                    return Ok(Output::Webcam {
                        device: Mutex::new(device),
                        size: (width, height),
                        delays: Arc::new(Mutex::new(Delays::default())),
                    })
                }
                Err(e) => eprintln!(
                    "iPhone camera: the webcam device can't be used ({e:#}); PipeWire only"
                ),
            }
        }
        Ok(Output::PipeWire(Camera::start(name, width, height, fps)?))
    }

    pub fn show(&self, picture: Picture, captured_us: i64) {
        match self {
            Output::PipeWire(c) => c.show(picture, captured_us),
            Output::Webcam { device, delays, .. } => {
                if device.lock().unwrap().show(&picture).is_ok() && captured_us != i64::MIN {
                    delays.lock().unwrap().record(captured_us);
                }
            }
        }
    }

    /// One black picture: what programs keep showing while the phone is away.
    pub fn black(&self) {
        match self {
            Output::PipeWire(c) => c.black(),
            Output::Webcam { size: (w, h), .. } => {
                let (w, h) = (*w as usize, *h as usize);
                let mut nv12 = vec![16u8; w * h * 3 / 2];
                nv12[w * h..].fill(128);
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
    }

    pub fn delays(&self) -> Arc<Mutex<Delays>> {
        match self {
            Output::PipeWire(c) => c.delays.clone(),
            Output::Webcam { delays, .. } => delays.clone(),
        }
    }
}
