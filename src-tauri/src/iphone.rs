//! The iPhone as this computer's camera and microphone (crates/quince-link): started with the
//! app, so a phone on the cable with Skykeep open on it is used at once, with nothing to press.
//! Linux for now; Windows follows with its own camera.

use serde::Serialize;

#[cfg(all(target_os = "linux", feature = "iphone"))]
static LINK: std::sync::OnceLock<quince_link::service::Service> = std::sync::OnceLock::new();

/// Starts watching for the phone. Only the one Skykeep that holds the library gets here, so
/// two never publish the same camera. Each change (plugged in, opened, in use, gone) reaches
/// the windows as `quince://iphone`, so Settings shows it as it happens.
pub fn start(app: &tauri::AppHandle) {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    {
        use tauri::Emitter;
        let app = app.clone();
        let _ = LINK.set(quince_link::service::start(move |_| {
            let _ = app.emit("quince://iphone", ());
        }));
    }
    #[cfg(not(all(target_os = "linux", feature = "iphone")))]
    let _ = app;
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IphoneStatus {
    /// "unavailable" (this build or system has no iPhone link), "none" (no iPhone on a cable),
    /// "plugged" (Skykeep isn't open on it), "inUse", or "cameraPaused" (iOS stopped the
    /// camera: the phone locked or another app in front; the microphone goes on), or
    /// "micOnly" (the camera turned off on the phone).
    state: &'static str,
    /// Capture on the phone to pictures reaching programs here, half of them within, in ms.
    camera_ms: Option<f64>,
    microphone_ms: Option<f64>,
    /// The camera for every program (the webcam device): "ready", "setUp" (one admin prompt
    /// away) or "install" (the system has no v4l2loopback module to make it with).
    every_app: &'static str,
}

/// The module's settings, in files of their own: the device is made at every start-up, named
/// as Skykeep finds it, a capture device only while Skykeep sets a format (Chrome's need),
/// with two buffers so no picture waits behind another.
#[cfg(all(target_os = "linux", feature = "iphone"))]
const SET_UP: &str = r#"set -e
printf 'v4l2loopback\n' > /etc/modules-load.d/skykeep-iphone-camera.conf
printf 'options v4l2loopback devices=1 exclusive_caps=1 max_buffers=2 card_label="iPhone Camera"\n' > /etc/modprobe.d/skykeep-iphone-camera.conf
if ! grep -q '^v4l2loopback ' /proc/modules; then modprobe v4l2loopback; fi
"#;

#[cfg(all(target_os = "linux", feature = "iphone"))]
fn every_app() -> &'static str {
    if quince_link::linux::webcam::find().is_some() {
        "ready"
    } else if std::process::Command::new("modinfo")
        .args(["-n", "v4l2loopback"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        "setUp"
    } else {
        "install"
    }
}

/// Makes the iPhone camera a webcam device every program sees, from now on: one admin prompt
/// (polkit) writes the module's two settings files and loads it. Answers what's true after.
#[tauri::command]
pub async fn iphone_every_app() -> Result<&'static str, String> {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    {
        if every_app() != "setUp" {
            return Ok(every_app());
        }
        let done = tauri::async_runtime::spawn_blocking(|| {
            std::process::Command::new("pkexec")
                .args(["/bin/sh", "-c", SET_UP])
                .status()
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("couldn't ask for permission: {e}"))?;
        if !done.success() {
            // Cancelled at the prompt, or refused: nothing changed.
            return Ok(every_app());
        }
        if let Some(link) = LINK.get() {
            link.start_camera_again();
        }
        Ok(every_app())
    }
    #[cfg(not(all(target_os = "linux", feature = "iphone")))]
    Err("unavailable".into())
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
            Status::CameraPaused => "cameraPaused",
            Status::MicOnly => "micOnly",
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
            every_app: every_app(),
        };
    }
    IphoneStatus {
        state: "unavailable",
        camera_ms: None,
        microphone_ms: None,
        every_app: "install",
    }
}
