//! Uses the iPhone on the cable as this computer's camera and microphone, as the app does
//! (plug and go), printing the status and delays every two seconds:
//! `cargo run -p quince-link --release --example iphone`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use quince_link::service;
use quince_link::stats::Delays;

fn main() {
    let s = service::start(|status| println!("now: {status:?}"));
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let r = s.report.lock().unwrap();
        let ms = |d: Option<&Arc<Mutex<Delays>>>, p: f64| {
            d.and_then(|d| d.lock().unwrap().percentile(p))
                .map(|v| format!("{:.1}", v as f64 / 1000.0))
                .unwrap_or("-".into())
        };
        println!(
            "{:?}: {} pictures (hardware {}), arrival p50 {} p95 {}, camera p50 {} p95 {} ms; microphone p50 {} p95 {} ms; keyframes asked {}",
            *s.status.lock().unwrap(),
            r.pictures,
            r.hardware_decoding,
            ms(Some(&r.arrival_delays), 0.5),
            ms(Some(&r.arrival_delays), 0.95),
            ms(r.picture_delays.as_ref(), 0.5),
            ms(r.picture_delays.as_ref(), 0.95),
            ms(r.sound_delays.as_ref(), 0.5),
            ms(r.sound_delays.as_ref(), 0.95),
            r.keyframes_asked
        );
    }
}
