//! The iPhone as this computer's camera and microphone (crates/quince-link). Off until the
//! person turns it on in Settings (nothing runs for those who don't use it); on, a phone on the
//! cable with Skykeep open on it is used at once, with nothing to press. The choice belongs to
//! this computer (a marker file in Skykeep's folder), not the Apple Account, so signing out
//! keeps it. Linux for now.

use serde::Serialize;

#[cfg(all(target_os = "linux", feature = "iphone"))]
static LINK: std::sync::Mutex<Option<quince_link::service::Service>> = std::sync::Mutex::new(None);

#[cfg(all(target_os = "linux", feature = "iphone"))]
static MARKER: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Starts watching for the phone if the person turned it on. Only the one Skykeep that holds
/// the library gets here, so two never publish the same camera.
pub fn start(app: &tauri::AppHandle, dir: &std::path::Path) {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    {
        let marker = MARKER.get_or_init(|| dir.join("iphone-link-on"));
        if marker.exists() {
            run(app);
        }
    }
    #[cfg(not(all(target_os = "linux", feature = "iphone")))]
    let _ = (app, dir);
}

/// Each change (plugged in, opened, in use, gone) reaches the windows as `quince://iphone`, so
/// Settings shows it as it happens.
#[cfg(all(target_os = "linux", feature = "iphone"))]
fn run(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let mut link = LINK.lock().unwrap();
    if link.is_none() {
        let app = app.clone();
        *link = Some(quince_link::service::start(move |_| {
            let _ = app.emit("quince://iphone", ());
        }));
    }
}

/// Turns the iPhone link on or off, kept for next time. Off stops everything at once: no
/// watching, the camera and microphone gone from the computer.
#[tauri::command]
pub fn iphone_set(on: bool, app: tauri::AppHandle) -> Result<IphoneStatus, String> {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    {
        use tauri::Emitter;
        let marker = MARKER.get().ok_or("not started")?;
        if on {
            std::fs::write(marker, b"").map_err(|e| e.to_string())?;
            run(&app);
        } else {
            let _ = std::fs::remove_file(marker);
            drop(LINK.lock().unwrap().take());
        }
        let _ = app.emit("quince://iphone", ());
    }
    #[cfg(not(all(target_os = "linux", feature = "iphone")))]
    let _ = (on, app);
    Ok(iphone_status())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IphoneStatus {
    /// "unavailable" (this build or system has no iPhone link), "off" (not turned on in
    /// Settings), "none" (no iPhone on a cable),
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
/// with four buffers: the fewest every program accepts. With two (chosen so no picture would
/// wait behind another), v4l2loopback 0.15 refused any reader asking for more — Chromium and
/// Meet ask for four — with "Device or resource busy", and the picture came at 21 fps.
/// A module already loaded with fewer is loaded again (Skykeep lets go of the device first).
#[cfg(all(target_os = "linux", feature = "iphone"))]
const SET_UP: &str = r#"set -e
printf 'v4l2loopback\n' > /etc/modules-load.d/skykeep-iphone-camera.conf
printf 'options v4l2loopback devices=1 exclusive_caps=1 max_buffers=4 card_label="iPhone Camera"\n' > /etc/modprobe.d/skykeep-iphone-camera.conf
if grep -q '^v4l2loopback ' /proc/modules && [ "$(cat /sys/module/v4l2loopback/parameters/max_buffers)" -lt 4 ]; then
  for _ in 1 2 3 4 5 6 7 8 9 10; do modprobe -r v4l2loopback && break; sleep 0.5; done
fi
if ! grep -q '^v4l2loopback ' /proc/modules; then modprobe v4l2loopback; fi
"#;

/// Buffers fewer than every program accepts (see SET_UP): the module wants loading again.
#[cfg(all(target_os = "linux", feature = "iphone"))]
fn too_few_buffers() -> bool {
    std::fs::read_to_string("/sys/module/v4l2loopback/parameters/max_buffers")
        .ok()
        .and_then(|n| n.trim().parse::<u32>().ok())
        .is_some_and(|n| n < 4)
}

#[cfg(all(target_os = "linux", feature = "iphone"))]
fn every_app() -> &'static str {
    if quince_link::linux::webcam::find().is_some() {
        if too_few_buffers() {
            "setUp"
        } else {
            "ready"
        }
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
pub async fn iphone_every_app(app: tauri::AppHandle) -> Result<&'static str, String> {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    {
        if every_app() != "setUp" {
            return Ok(every_app());
        }
        // A module to load again: the link lets go of the device for it, and comes back after.
        let restart = too_few_buffers() && LINK.lock().unwrap().take().is_some();
        if restart {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let done = tauri::async_runtime::spawn_blocking(|| {
            std::process::Command::new("pkexec")
                .args(["/bin/sh", "-c", SET_UP])
                .status()
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("couldn't ask for permission: {e}"))?;
        if restart {
            run(&app);
        }
        if !done.success() {
            // Cancelled at the prompt, or refused: nothing changed.
            return Ok(every_app());
        }
        if let Some(link) = LINK.lock().unwrap().as_ref() {
            link.start_camera_again();
        }
        Ok(every_app())
    }
    #[cfg(not(all(target_os = "linux", feature = "iphone")))]
    {
        let _ = app;
        Err("unavailable".into())
    }
}

#[tauri::command]
pub fn iphone_status() -> IphoneStatus {
    #[cfg(all(target_os = "linux", feature = "iphone"))]
    if let Some(status) = linked_status() {
        return status;
    }
    IphoneStatus {
        state: "unavailable",
        camera_ms: None,
        microphone_ms: None,
        every_app: "install",
    }
}

#[cfg(all(target_os = "linux", feature = "iphone"))]
fn linked_status() -> Option<IphoneStatus> {
    if MARKER.get().is_some_and(|m| !m.exists()) {
        return Some(IphoneStatus {
            state: "off",
            camera_ms: None,
            microphone_ms: None,
            every_app: every_app(),
        });
    }
    if let Some(link) = LINK.lock().unwrap().as_ref() {
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
        return Some(IphoneStatus {
            state,
            camera_ms: ms(report.picture_delays.as_ref()),
            microphone_ms: ms(report.sound_delays.as_ref()),
            every_app: every_app(),
        });
    }
    None
}
