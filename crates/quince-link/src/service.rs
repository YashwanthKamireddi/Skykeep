//! Plug and go: the iPhone is used the moment it's on the cable with Skykeep open on it, and
//! again after every unplug, app switch or restart, with nothing to press on the computer.
//!
//! usbmuxd tells when a phone is plugged in. While it stays plugged in, the computer tries
//! the app's port every second until the app answers (the phone gives no signal when an app
//! opens), and serves it until the link closes. A phone unplugged makes `connect` fail with
//! "no such device", which ends its loop.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::receiver::{self, CameraState, Devices, Report};
use crate::usbmux;

pub const PORT: u16 = 47820;

/// What the person sees about the phone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// No iPhone on a cable.
    #[default]
    NoPhone,
    /// An iPhone is plugged in; Skykeep isn't open on it.
    Plugged,
    /// The computer is using the phone's camera and microphone.
    InUse,
    /// Connected, but iOS stopped the camera: the phone is locked, or another app is in front
    /// without picture in picture. The microphone goes on.
    CameraPaused,
    /// Connected, the camera turned off on the phone: the iPhone is only the microphone.
    MicOnly,
}

pub struct Service {
    pub status: Arc<Mutex<Status>>,
    pub report: Arc<Mutex<Report>>,
    devices: Arc<Devices>,
    /// The open link to each phone, so it can be started again.
    links: Arc<Mutex<std::collections::HashMap<u64, usbmux::Socket>>>,
    /// The usbmuxd connection that hears phones come and go.
    listening: Arc<Mutex<Option<usbmux::Socket>>>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Service {
    /// Stops everything: no more listening, every link closed, and the camera and microphone
    /// gone from the computer once their sessions end (moments later).
    fn drop(&mut self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(s) = self.listening.lock().unwrap().take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        for (_, link) in self.links.lock().unwrap().drain() {
            let _ = link.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Service {
    /// Starts the camera again: after the webcam device is made, the next picture goes there.
    /// The links close and reconnect at once (plug and go), and the phone sends its format and
    /// a keyframe as it does to any computer that connects.
    pub fn start_camera_again(&self) {
        self.devices.forget_camera();
        for (_, link) in self.links.lock().unwrap().drain() {
            let _ = link.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Told each new status.
type Changed = Arc<dyn Fn(Status) + Send + Sync>;

/// Sets the status, telling `changed` only when it really changes.
fn set(status: &Mutex<Status>, changed: &Changed, new: Status) {
    let mut s = status.lock().unwrap();
    if *s != new {
        *s = new.clone();
        drop(s);
        changed(new);
    }
}

/// Starts watching for phones, telling `changed` whenever the status changes. Runs as long as
/// the program does.
pub fn start(changed: impl Fn(Status) + Send + Sync + 'static) -> Service {
    let changed: Changed = Arc::new(changed);
    let status = Arc::new(Mutex::new(Status::NoPhone));
    let report = Arc::new(Mutex::new(Report::default()));
    let devices = Arc::new(Devices::default());
    let links = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let listening = Arc::new(Mutex::new(None));
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (li, st) = (listening.clone(), stopped.clone());
    let serving = Arc::new(Mutex::new(HashSet::<u64>::new()));
    let (s, r, d0, l0) = (
        status.clone(),
        report.clone(),
        devices.clone(),
        links.clone(),
    );
    std::thread::Builder::new()
        .name("iphone-watch".into())
        .spawn(move || loop {
            let begin = |phone: u64| {
                if !serving.lock().unwrap().insert(phone) {
                    return;
                }
                let (s, r, d, serving, c, l, stop) = (
                    s.clone(),
                    r.clone(),
                    d0.clone(),
                    serving.clone(),
                    changed.clone(),
                    l0.clone(),
                    st.clone(),
                );
                let _ = std::thread::Builder::new()
                    .name("iphone-link".into())
                    .spawn(move || {
                        serve_while_plugged(phone, &s, &c, &r, &d, &l, &stop);
                        serving.lock().unwrap().remove(&phone);
                        if serving.lock().unwrap().is_empty() {
                            set(&s, &c, Status::NoPhone);
                        }
                    });
            };
            if st.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            if let Ok(phones) = usbmux::phones() {
                phones.into_iter().for_each(begin);
            }
            // Returns when usbmuxd stops (or isn't installed), or when stopped: look again in a
            // while unless stopped.
            let _ = usbmux::watch(&li, begin);
            if st.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_secs(5));
        })
        .expect("a thread can start");
    Service {
        status,
        report,
        devices,
        links,
        listening,
        stopped,
    }
}

fn serve_while_plugged(
    phone: u64,
    status: &Mutex<Status>,
    changed: &Changed,
    report: &Arc<Mutex<Report>>,
    devices: &Arc<Devices>,
    links: &Mutex<std::collections::HashMap<u64, usbmux::Socket>>,
    stopped: &std::sync::atomic::AtomicBool,
) {
    loop {
        if stopped.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match usbmux::connect(phone, PORT) {
            Ok(link) => {
                let Ok(reader) = link.try_clone() else { return };
                if let Ok(handle) = link.try_clone() {
                    links.lock().unwrap().insert(phone, handle);
                }
                set(status, changed, Status::InUse);
                let paused = |c: CameraState| {
                    set(
                        status,
                        changed,
                        match c {
                            CameraState::Live => Status::InUse,
                            CameraState::Paused => Status::CameraPaused,
                            CameraState::Off => Status::MicOnly,
                        },
                    )
                };
                if let Err(e) =
                    receiver::serve(reader, link, report.clone(), devices.clone(), &paused)
                {
                    eprintln!("iPhone link closed: {e:#}");
                }
                set(status, changed, Status::Plugged);
            }
            Err(e) if e.is::<usbmux::AppNotOpen>() => {
                set(status, changed, Status::Plugged);
            }
            // Unplugged (or usbmuxd gone): this phone's loop ends; plugging in starts a new one.
            Err(_) => {
                links.lock().unwrap().remove(&phone);
                return;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
