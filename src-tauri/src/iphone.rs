//! The iPhone as this computer's camera and microphone (crates/quince-link): started with the
//! app, so a phone on the cable with Skykeep open on it is used at once, with nothing to press.
//! Linux for now; Windows follows with its own camera.

use serde::Serialize;

#[cfg(all(target_os = "linux", feature = "iphone"))]
static LINK: std::sync::OnceLock<quince_link::service::Service> = std::sync::OnceLock::new();

/// Starts watching for the phone. Only the one Skykeep that holds the library gets here, so
/// two never publish the same camera.
pub fn start() {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    {
        let _ = LINK.set(quince_link::service::start());
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IphoneStatus {
    /// "none" (no iPhone on a cable), "plugged" (Skykeep isn't open on it) or "inUse".
    state: &'static str,
    /// Capture on the phone to pictures reaching programs here, half of them within, in ms.
    camera_ms: Option<f64>,
    microphone_ms: Option<f64>,
}

#[tauri::command]
pub fn iphone_status() -> IphoneStatus {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    if let Some(link) = LINK.get() {
        use quince_link::service::Status;
        let state = match *link.status.lock().unwrap() {
            Status::NoPhone => "none",
            Status::Plugged => "plugged",
            Status::InUse => "inUse",
        };
        let report = link.report.lock().unwrap();
        let ms = |d: Option<&std::sync::Arc<std::sync::Mutex<quince_link::stats::Delays>>>| {
            d.and_then(|d| d.lock().unwrap().percentile(0.5))
                .map(|us| us as f64 / 1000.0)
        };
        return IphoneStatus {
            state,
            camera_ms: ms(report.picture_delays.as_ref()),
            microphone_ms: ms(report.sound_delays.as_ref()),
        };
    }
    IphoneStatus {
        state: "none",
        camera_ms: None,
        microphone_ms: None,
    }
}
