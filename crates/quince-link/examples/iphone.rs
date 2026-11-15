//! Uses the iPhone on the cable as this computer's camera and microphone, printing the delays
//! every two seconds: `cargo run -p quince-link --release --example iphone`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use quince_link::{receiver, usbmux};

fn main() -> anyhow::Result<()> {
    let phone = *usbmux::phones()?
        .first()
        .ok_or_else(|| anyhow::anyhow!("no iPhone on a cable"))?;
    let link = usbmux::connect(phone, 47820)?;
    let report = Arc::new(Mutex::new(receiver::Report::default()));
    let r = report.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let r = r.lock().unwrap();
        let ms = |d: &Option<Arc<Mutex<quince_link::stats::Delays>>>, p: f64| {
            d.as_ref()
                .and_then(|d| d.lock().unwrap().percentile(p))
                .map(|v| format!("{:.1}", v as f64 / 1000.0))
                .unwrap_or("-".into())
        };
        println!(
            "{} pictures (hardware {}), arrival p50 {} p95 {}, camera p50 {} p95 {} ms; microphone p50 {} p95 {} ms; keyframes asked {}",
            r.pictures,
            r.hardware_decoding,
            ms(&Some(r.arrival_delays.clone()), 0.5),
            ms(&Some(r.arrival_delays.clone()), 0.95),
            ms(&r.picture_delays, 0.5),
            ms(&r.picture_delays, 0.95),
            ms(&r.sound_delays, 0.5),
            ms(&r.sound_delays, 0.95),
            r.keyframes_asked
        );
    });
    let reader = link.try_clone()?;
    receiver::serve(reader, link, report)?;
    println!("the iPhone disconnected");
    Ok(())
}
