//! What keeps a launch on Windows from ending in nothing: a Skykeep that cannot open its
//! window says why, in words, and offers the way out, rather than vanishing.
//!
//! The window is drawn by Microsoft Edge WebView2. Windows can list WebView2 as installed when
//! it is not usable: a half-removed runtime (Edge-removal and "debloat" tools, a failed
//! update) leaves its registry entry, often version 0.0.0.0, and nothing else. The installer's
//! check reads that entry, so it skipped the repair; the Store package can install nothing;
//! and the window then failed with WebView2's own "Could not find the WebView2 Runtime" and
//! Skykeep ended. Here WebView2 is asked the way the window asks it (`webview_version`, the
//! very call the window is gated on), before anything else starts, and when it answers no,
//! Skykeep offers to repair it with Microsoft's own installer and starts again once it has.
//!
//! A start that fails for any other reason (`failed`), or a window that cannot be made, is
//! said in a message too: on Windows there is no console, and a panic there was silence.

/// Microsoft's Evergreen WebView2 bootstrapper: the small installer that fetches and installs
/// (or repairs) the runtime for every user, asking for permission itself.
#[cfg(windows)]
const BOOTSTRAPPER: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

#[cfg(windows)]
const TITLE: &str = "Skykeep";

/// Before Tauri starts: WebView2 there and usable, or the person offered its repair. Returns
/// only when Skykeep may go on; otherwise this launch ends here, by the person's choice, or
/// hands over to a new one started after the repair.
#[cfg(windows)]
pub fn ensure_webview() {
    if tauri::webview_version().is_ok() {
        return;
    }
    let asked = ask(
        "Skykeep needs Microsoft Edge WebView2 to show its windows, and it isn't working on \
         this PC. It may be missing or damaged, even if Windows lists it as installed.\n\n\
         Repair it now? Microsoft's installer will download and run, and Skykeep will open \
         when it's done.",
    );
    if !asked {
        std::process::exit(0);
    }
    match repair() {
        Ok(()) if tauri::webview_version().is_ok() => restart(),
        // Installed, but this process may not see it yet (it reads the runtime once): a new
        // one does.
        Ok(()) => restart(),
        Err(why) => {
            tell(&format!(
                "WebView2 couldn't be repaired: {why}.\n\nYou can install it yourself from \
                 developer.microsoft.com/microsoft-edge/webview2 (Evergreen Standalone \
                 Installer, x64), then open Skykeep again."
            ));
            std::process::exit(1);
        }
    }
}

#[cfg(not(windows))]
pub fn ensure_webview() {}

/// Skykeep could not start at all: said, never a silent exit.
pub fn failed(error: &dyn std::fmt::Display) -> ! {
    let message = format!(
        "Skykeep couldn't start: {error}\n\nPlease restart your computer and try again. If it \
         keeps happening, report it at github.com/YashwanthKamireddi/Skykeep/issues with this \
         message."
    );
    #[cfg(windows)]
    tell(&message);
    #[cfg(not(windows))]
    eprintln!("{message}");
    std::process::exit(1);
}

/// The window could not be made: said, rather than a Skykeep that runs with nothing to see.
pub fn no_window(error: &dyn std::fmt::Display) {
    #[cfg(windows)]
    tell(&format!(
        "Skykeep couldn't open its window: {error}\n\nIf this keeps happening, repair Microsoft \
         Edge WebView2 (Settings, Apps, Installed apps, Microsoft Edge WebView2 Runtime, \
         Modify) and open Skykeep again."
    ));
    #[cfg(not(windows))]
    eprintln!("Skykeep couldn't open its window: {error}");
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A Yes/No question in Windows' own message box. Yes is true.
#[cfg(windows)]
fn ask(text: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONWARNING, MB_SETFOREGROUND, MB_YESNO,
    };
    let (text, title) = (wide(text), wide(TITLE));
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    let answer = unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND,
        )
    };
    answer == IDYES
}

/// A message in Windows' own message box, with OK.
#[cfg(windows)]
fn tell(text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };
    let (text, title) = (wide(text), wide(TITLE));
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND,
        );
    }
}

/// Microsoft's bootstrapper downloaded to a folder of this launch's own and run with its
/// window shown (it asks for permission and shows its progress), waited for.
#[cfg(windows)]
fn repair() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Com::Urlmon::URLDownloadToFileW;
    let dir = std::env::temp_dir().join(format!("skykeep-webview2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("no folder for the installer ({e})"))?;
    let file = dir.join("MicrosoftEdgeWebview2Setup.exe");
    let (url, path) = (wide(BOOTSTRAPPER), wide(&file.to_string_lossy()));
    // SAFETY: both strings are NUL-terminated and outlive the call; no callback.
    unsafe { URLDownloadToFileW(None, PCWSTR(url.as_ptr()), PCWSTR(path.as_ptr()), 0, None) }
        .map_err(|e| format!("the installer didn't download ({e}); check the connection"))?;
    let status = std::process::Command::new(&file)
        .arg("/install")
        .status()
        .map_err(|e| format!("the installer didn't start ({e})"))?;
    let _ = std::fs::remove_dir_all(&dir);
    if status.success() {
        Ok(())
    } else {
        Err(format!("the installer stopped ({status})"))
    }
}

/// This launch handed over to a new one, with the same arguments. A Store copy is not started
/// straight from its program file (it would run without its package's identity): it is asked to
/// be opened again instead, from Start, as Windows starts it.
#[cfg(windows)]
fn restart() -> ! {
    if crate::package::packaged() {
        tell_info("WebView2 is repaired. Open Skykeep again from Start.");
        std::process::exit(0);
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .args(std::env::args_os().skip(1))
            .spawn();
    }
    std::process::exit(0);
}

/// A message in Windows' own message box, with OK, as news rather than an error.
#[cfg(windows)]
fn tell_info(text: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
    };
    let (text, title) = (wide(text), wide(TITLE));
    // SAFETY: both strings are NUL-terminated and outlive the call; no owner window.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}
