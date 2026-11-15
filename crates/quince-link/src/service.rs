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

use crate::receiver::{self, Devices, Report};
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
}

pub struct Service {
    pub status: Arc<Mutex<Status>>,
    pub report: Arc<Mutex<Report>>,
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
    let serving = Arc::new(Mutex::new(HashSet::<u64>::new()));
    let (s, r) = (status.clone(), report.clone());
    std::thread::Builder::new()
        .name("iphone-watch".into())
        .spawn(move || loop {
            let begin = |phone: u64| {
                if !serving.lock().unwrap().insert(phone) {
                    return;
                }
                let (s, r, d, serving, c) = (
                    s.clone(),
                    r.clone(),
                    devices.clone(),
                    serving.clone(),
                    changed.clone(),
                );
                let _ = std::thread::Builder::new()
                    .name("iphone-link".into())
                    .spawn(move || {
                        serve_while_plugged(phone, &s, &c, &r, &d);
                        serving.lock().unwrap().remove(&phone);
                        if serving.lock().unwrap().is_empty() {
                            set(&s, &c, Status::NoPhone);
                        }
                    });
            };
            if let Ok(phones) = usbmux::phones() {
                phones.into_iter().for_each(begin);
            }
            // Returns when usbmuxd stops (or isn't installed): look again in a while.
            let _ = usbmux::watch(begin);
            std::thread::sleep(Duration::from_secs(5));
        })
        .expect("a thread can start");
    Service { status, report }
}

fn serve_while_plugged(
    phone: u64,
    status: &Mutex<Status>,
    changed: &Changed,
    report: &Arc<Mutex<Report>>,
    devices: &Arc<Devices>,
) {
    loop {
        match usbmux::connect(phone, PORT) {
            Ok(link) => {
                let Ok(reader) = link.try_clone() else { return };
                set(status, changed, Status::InUse);
                if let Err(e) = receiver::serve(reader, link, report.clone(), devices.clone()) {
                    eprintln!("iPhone link closed: {e:#}");
                }
                set(status, changed, Status::Plugged);
            }
            Err(e) if e.is::<usbmux::AppNotOpen>() => {
                set(status, changed, Status::Plugged);
            }
            // Unplugged (or usbmuxd gone): this phone's loop ends; plugging in starts a new one.
            Err(_) => return,
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
