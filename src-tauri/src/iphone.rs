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

/// The module's settings, in files of their own, and the module loaded (again, if it was loaded
/// with too few buffers): run as root. The settings and why they are what they are are in the
/// script. The packages install it as HELPER; without it, this same text runs as `sh -c`.
#[cfg(all(target_os = "linux", feature = "iphone"))]
const SET_UP: &str = include_str!("../../packaging/linux/iphone-camera-setup");

/// Where the .deb, .rpm and Arch packages install SET_UP, and the polkit action that pkexec
/// matches to it by that path (packaging/linux/*.iphone-camera.policy): its message, naming
/// Skykeep and what the setup does, is what the admin prompt says.
#[cfg(all(target_os = "linux", feature = "iphone"))]
const HELPER: &str = "/usr/lib/skykeep/iphone-camera-setup";
#[cfg(all(target_os = "linux", feature = "iphone"))]
const POLICY: &str =
    "/usr/share/polkit-1/actions/io.github.yashwanthkamireddi.Skykeep.iphone-camera.policy";

/// What pkexec runs: the installed helper when it and its polkit action are both there, so the
/// prompt says what it is for; otherwise the same script as `/bin/sh -c`, whose prompt names
/// /bin/sh (the Snap and the AppImage, which can't install either, and a build of one's own).
/// A helper anyone but root could change is never run as root.
#[cfg(all(target_os = "linux", feature = "iphone"))]
fn set_up_args(helper: Option<&std::fs::Metadata>, policy: bool) -> Vec<&'static str> {
    use std::os::unix::fs::MetadataExt;
    let trusted = helper.is_some_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0);
    if trusted && policy {
        vec![HELPER]
    } else {
        vec!["/bin/sh", "-c", SET_UP]
    }
}

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
            let helper = std::fs::metadata(HELPER).ok();
            let policy = std::path::Path::new(POLICY).is_file();
            std::process::Command::new("pkexec")
                .args(set_up_args(helper.as_ref(), policy))
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

#[cfg(all(test, target_os = "linux", feature = "iphone"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    fn fallback() -> Vec<&'static str> {
        vec!["/bin/sh", "-c", SET_UP]
    }

    #[test]
    fn nothing_installed_runs_the_script_through_sh() {
        // The Snap, the AppImage, a build of one's own: no helper, or no polkit action for it.
        assert_eq!(set_up_args(None, false), fallback());
        assert_eq!(set_up_args(None, true), fallback());
        let sh = std::fs::metadata("/bin/sh").unwrap();
        assert_eq!(
            set_up_args(Some(&sh), false),
            fallback(),
            "no action, so no message"
        );
    }

    #[test]
    fn a_root_owned_helper_with_its_policy_is_run() {
        // /bin/sh stands in for the installed helper: a root-owned program only root can change.
        let sh = std::fs::metadata("/bin/sh").unwrap();
        assert_eq!((sh.uid(), sh.mode() & 0o022), (0, 0));
        assert_eq!(set_up_args(Some(&sh), true), vec![HELPER]);
    }

    #[test]
    fn a_helper_root_does_not_own_is_never_run() {
        // A directory is not a program, even root's.
        let usr = std::fs::metadata("/usr").unwrap();
        assert_eq!(set_up_args(Some(&usr), true), fallback());
        // One the person owns could be changed by anything they run.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iphone-camera-setup");
        std::fs::write(&path, SET_UP).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mine = std::fs::metadata(&path).unwrap();
        if mine.uid() != 0 {
            assert_eq!(set_up_args(Some(&mine), true), fallback());
        }
    }

    #[test]
    fn the_script_takes_no_arguments() {
        // Refused before it touches anything: $0 is the name, then one argument.
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", SET_UP, "iphone-camera-setup", "--anything"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
}
