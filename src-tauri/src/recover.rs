//! A window whose page has died is given its page again.
//!
//! Each window's page runs in a process of its own (WebKitWebProcess on Linux, a WebView2
//! renderer on Windows). When that process crashes, or the system kills it for its memory —
//! WebKitGTK 2.54 has runaways on some GPUs that end in the kernel's OOM killer — the window
//! stays on screen with nothing in it. Until this, it stayed blank until Skykeep was restarted.
//! Now the page is loaded again: at once the first time, after a short wait the next, and after
//! a few in a row it stops trying, says so plainly and lets the window go, so opening it again
//! starts afresh instead of looping on a page that cannot stay up.
//!
//! What a reload loses. Edits are not held by the page: a note, a reminder, an event or a card
//! is handed to the backend's outbox when it is saved, and the outbox keeps it on disk and sends
//! it whatever the window does. What lives only in the page is lost with its process, as it
//! would be with any reload: typing in the last moment before a save (a note saves after a
//! 900 ms pause in typing, and on losing the focus — a process killed mid-word takes the words
//! since the last pause), an editor not yet saved (a new event's or card's sheet before Done, a
//! reminder still being typed), and where the window was — the note open, the selection, the
//! scroll. Settings kept in the page's localStorage are on disk and come back.
//!
//! Nothing private is noted: the window's label and why its page ended.

use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// Deaths of a window's page counted together: further apart, each is a first.
const WITHIN: Duration = Duration::from_secs(10 * 60);

/// How long to wait before each reload, in turn: at once, as a single crash is most often just
/// that; then after a pause, so a page that dies as it loads does not spin the processor. Past
/// the last, it is given up.
const WAITS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(2),
    Duration::from_secs(10),
];

/// When each window's page last died, by window label.
static DIED: Mutex<Vec<(String, Instant)>> = Mutex::new(Vec::new());

/// Why a page's process ended, as each platform says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Crashed,
    OutOfMemory,
    /// WebView2's browser process, which every window's page hangs from, ended: the web view
    /// itself is gone and cannot load anything again.
    Gone,
}

impl Why {
    fn words(self) -> &'static str {
        match self {
            Why::Crashed => "crashed",
            Why::OutOfMemory => "was ended for its memory",
            Why::Gone => "lost its browser process",
        }
    }
}

/// What to do about a death of a window's page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// The page loaded again after this wait; the death is the nth (from 1) in a row.
    Reload(Duration, usize),
    /// Too many in a row, or nothing to load into: the window is let go.
    GiveUp(usize),
}

/// The step for a death, given how many of the window's earlier deaths are still counted.
fn step(earlier: usize, why: Why) -> Step {
    let nth = earlier + 1;
    if why == Why::Gone {
        return Step::GiveUp(nth);
    }
    match WAITS.get(earlier) {
        Some(wait) => Step::Reload(*wait, nth),
        None => Step::GiveUp(nth),
    }
}

/// Counts this death against the window, forgetting deaths too long ago, and says what to do.
/// A window given up on starts again from nothing.
fn record(label: &str, why: Why, now: Instant) -> Step {
    let mut died = DIED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    died.retain(|(_, at)| now.duration_since(*at) < WITHIN);
    let earlier = died.iter().filter(|(l, _)| l == label).count();
    let next = step(earlier, why);
    match next {
        Step::Reload(..) => died.push((label.to_string(), now)),
        Step::GiveUp(_) => died.retain(|(l, _)| l != label),
    }
    next
}

/// A window's page has died: loaded again, or, past a few times, the window let go with a word.
/// Called on the main thread, from the web view's own signal.
pub fn died(app: &AppHandle, label: &str, why: Why) {
    let step = record(label, why, Instant::now());
    quiet!("the {label} window's page {}: {step:?}", why.words());
    crate::timing::mark(format_args!("{label} page {} ({step:?})", why.words()));
    match step {
        Step::Reload(wait, nth) => {
            crate::diagnostics::note(&format!(
                "window: the {label} window's page {}; loaded again ({nth} of {})",
                why.words(),
                WAITS.len()
            ));
            if wait.is_zero() {
                reload(app, label);
                return;
            }
            let (app, label) = (app.clone(), label.to_string());
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(wait).await;
                let handle = app.clone();
                app.run_on_main_thread(move || reload(&handle, &label)).ok();
            });
        }
        Step::GiveUp(nth) => {
            crate::diagnostics::note(&format!(
                "window: the {label} window's page {} ({nth} in a row); window let go",
                why.words()
            ));
            give_up(app, label);
        }
    }
}

/// The window's page loaded again, if the window is still there (closed meanwhile, there is
/// nothing to restore).
fn reload(app: &AppHandle, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    if let Err(e) = window.reload() {
        quiet!("the {label} window's page could not be loaded again: {e}");
        crate::diagnostics::note(&format!(
            "window: the {label} window's page could not be loaded again: {e}"
        ));
        give_up(app, label);
    }
}

/// Said plainly, and the window let go once the person has read it: the next time it is asked
/// for it is made new, with a fresh web view, rather than shown again with a dead one. The
/// whole-library window let go is what closing it does: behind the tray, or quit without one.
fn give_up(app: &AppHandle, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    let name = window.title().unwrap_or_else(|_| "Skykeep".into());
    let text = format!(
        "{name} stopped working several times in a row, so its window was closed. \
         Skykeep is still running, and everything you saved is kept. \
         Open {name} again to carry on."
    );
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    app.dialog()
        .message(text)
        .title(name)
        .kind(MessageDialogKind::Warning)
        .show(move |_| {
            #[cfg(windows)]
            crate::taskbar::unname_window(&window);
            window.destroy().ok();
        });
}

/// Watches the web view's page process, so its death is heard of. Every web view, as it is made.
#[cfg(target_os = "linux")]
pub fn watch(webview: &tauri::Webview) {
    let (app, label) = (webview.app_handle().clone(), webview.label().to_string());
    let done = webview.with_webview(move |platform| {
        use webkit2gtk::{WebProcessTerminationReason as Reason, WebViewExt};
        platform
            .inner()
            .connect_web_process_terminated(move |_, reason| {
                let why = match reason {
                    // Ended by WebKit on our asking: a window closed for good.
                    Reason::TerminatedByApi => return,
                    Reason::ExceededMemoryLimit => Why::OutOfMemory,
                    // A crash, or a SIGKILL from the kernel's OOM killer: WebKit says crashed.
                    _ => Why::Crashed,
                };
                died(&app, &label, why);
            });
    });
    if let Err(e) = done {
        quiet!("the web view's page process could not be watched: {e}");
    }
}

/// Watches the web view's processes, so a renderer's death is heard of. WebView2 says so with
/// ProcessFailed on the web view whose page it was.
#[cfg(windows)]
pub fn watch(webview: &tauri::Webview) {
    let (app, label) = (webview.app_handle().clone(), webview.label().to_string());
    let done = webview.with_webview(move |platform| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2ProcessFailedEventArgs2, COREWEBVIEW2_PROCESS_FAILED_KIND,
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_REASON, COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY,
        };
        use webview2_com::ProcessFailedEventHandler;
        use windows::core::Interface;
        let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
            let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
            // Why it ended, where the runtime is new enough to say (ProcessFailedEventArgs2), as
            // WebKit says it on Linux.
            let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
            if let Some(args) = args {
                unsafe { args.ProcessFailedKind(&mut kind)? };
                if let Ok(args) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                    let _ = unsafe { args.Reason(&mut reason) };
                }
            }
            let why = match kind {
                COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                    if reason == COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY =>
                {
                    Why::OutOfMemory
                }
                COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => Why::Crashed,
                COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => Why::Gone,
                // An unresponsive renderer recovers by itself or exits, which is heard of then;
                // a frame's or a helper's (GPU, utility) process is restarted by WebView2.
                _ => return Ok(()),
            };
            died(&app, &label, why);
            Ok(())
        }));
        let mut token = 0i64;
        // SAFETY: the controller is the web view's own, alive for this call on its thread.
        let added = unsafe {
            platform
                .controller()
                .CoreWebView2()
                .and_then(|core| core.add_ProcessFailed(&handler, &mut token))
        };
        if let Err(e) = added {
            quiet!("the web view's processes could not be watched: {e}");
        }
    });
    if let Err(e) = done {
        quiet!("the web view's processes could not be watched: {e}");
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn watch(_webview: &tauri::Webview) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// At once, then after a pause, then a longer one; the fourth in a row is given up.
    #[test]
    fn a_page_is_loaded_again_a_few_times_then_given_up() {
        assert_eq!(step(0, Why::Crashed), Step::Reload(Duration::ZERO, 1));
        assert_eq!(
            step(1, Why::OutOfMemory),
            Step::Reload(Duration::from_secs(2), 2)
        );
        assert_eq!(
            step(2, Why::Crashed),
            Step::Reload(Duration::from_secs(10), 3)
        );
        assert_eq!(step(3, Why::Crashed), Step::GiveUp(4));
    }

    /// A web view whose browser process is gone cannot load anything: given up at once.
    #[test]
    fn a_web_view_gone_is_given_up_at_once() {
        assert_eq!(step(0, Why::Gone), Step::GiveUp(1));
    }

    /// Counted per window, forgotten after a while, and from nothing again once given up.
    #[test]
    fn deaths_are_counted_per_window_and_forgotten() {
        let t = Instant::now();
        let a = "test-recover-a";
        let b = "test-recover-b";
        assert_eq!(record(a, Why::Crashed, t), Step::Reload(Duration::ZERO, 1));
        assert_eq!(record(b, Why::Crashed, t), Step::Reload(Duration::ZERO, 1));
        assert_eq!(record(a, Why::Crashed, t), Step::Reload(WAITS[1], 2));
        // Long after, a death is a first again.
        let later = t + WITHIN + Duration::from_secs(1);
        assert_eq!(
            record(a, Why::Crashed, later),
            Step::Reload(Duration::ZERO, 1)
        );
        assert_eq!(record(a, Why::Crashed, later), Step::Reload(WAITS[1], 2));
        assert_eq!(record(a, Why::Crashed, later), Step::Reload(WAITS[2], 3));
        assert_eq!(record(a, Why::Crashed, later), Step::GiveUp(4));
        assert_eq!(
            record(a, Why::Crashed, later),
            Step::Reload(Duration::ZERO, 1)
        );
    }
}
