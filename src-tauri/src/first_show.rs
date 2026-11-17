//! A window's first appearance, decided once.
//!
//! Every window is made hidden and shown once its page has drawn (`window_ready`), or after
//! FALLBACK if the page never says so. Both are only the window *becoming ready*: neither is the
//! person asking for it. Shown by whichever came, a window closed a moment later was shown again
//! by the other — the page drawing, a page reloaded, or the fallback seeing a hidden window —
//! so a window closed as it opened came straight back.
//!
//! So a made window owes one first show, and the first thing that decides it settles it: the
//! page drawing, the fallback, the window put away or gone, or the person asking for it. Every
//! later signal finds nothing owed and does nothing. A window made again gets a new turn, so a
//! fallback left from an earlier one never shows the new one before it has drawn.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// How long a page has to say it has drawn before its window is shown anyway: never a launch
/// that opens nothing, however slow the page or a keyring that does not answer.
pub const FALLBACK: Duration = Duration::from_secs(3);

/// The windows owing a first show, each with the turn it was made in.
static OWED: Mutex<Owed> = Mutex::new(Owed(Vec::new()));
static TURN: AtomicU64 = AtomicU64::new(1);

/// Labels owing a first show, by turn. Kept apart from the window system so its rules are tested.
struct Owed(Vec<(String, u64)>);

impl Owed {
    fn owe(&mut self, label: &str, turn: u64) {
        self.0.retain(|(l, _)| l != label);
        self.0.push((label.to_string(), turn));
    }
    /// The show owed to `label` (of `turn`, when given), taken: true once, then never again.
    fn take(&mut self, label: &str, turn: Option<u64>) -> bool {
        let before = self.0.len();
        self.0
            .retain(|(l, t)| !(l == label && turn.is_none_or(|turn| *t == turn)));
        self.0.len() != before
    }
    fn turn_of(&self, label: &str) -> Option<u64> {
        self.0.iter().find(|(l, _)| l == label).map(|(_, t)| *t)
    }
}

fn owed() -> std::sync::MutexGuard<'static, Owed> {
    OWED.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `window`, just made hidden, owes its first show.
pub fn owe(window: &tauri::WebviewWindow) {
    owed().owe(window.label(), TURN.fetch_add(1, Ordering::Relaxed));
}

/// Whether `label` still waits for its first show (made, not yet drawn or decided).
pub fn owing(label: &str) -> bool {
    owed().turn_of(label).is_some()
}

/// The page drew, or another signal of readiness came: true only for the first, and only while
/// the show is still owed — its caller then shows the window.
pub fn take(label: &str) -> bool {
    owed().take(label, None)
}

/// Decided without a show: the window put away, gone, or shown by the person asking for it.
pub fn settle(label: &str) {
    owed().take(label, None);
}

/// Ending: nothing still loading is shown while Skykeep winds down.
pub fn settle_all() {
    owed().0.clear();
}

/// Shown after FALLBACK if its show is still owed then, by this same turn of the window.
pub fn fallback(window: &tauri::WebviewWindow) {
    let Some(turn) = owed().turn_of(window.label()) else {
        return;
    };
    let w = window.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FALLBACK).await;
        if owed().take(w.label(), Some(turn)) {
            crate::timing::mark(format_args!("{} shown by its fallback", w.label()));
            crate::apps::show(&w);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::Owed;

    #[test]
    fn a_first_show_is_given_once() {
        let mut o = Owed(Vec::new());
        o.owe("app-notes", 1);
        assert!(o.take("app-notes", None), "the page drawing shows it");
        assert!(
            !o.take("app-notes", Some(1)),
            "the fallback after it does nothing"
        );
        assert!(!o.take("app-notes", None), "a page reloaded does nothing");
    }

    #[test]
    fn a_window_closed_owes_nothing() {
        let mut o = Owed(Vec::new());
        o.owe("app-reminders", 1);
        // Shown by the fallback before its page drew, then closed: the page drawing later
        // must not bring it back.
        assert!(o.take("app-reminders", Some(1)));
        assert!(!o.take("app-reminders", None));
    }

    #[test]
    fn an_old_fallback_never_shows_a_window_made_again() {
        let mut o = Owed(Vec::new());
        o.owe("app-photos", 1);
        o.take("app-photos", None);
        o.owe("app-photos", 2);
        assert!(
            !o.take("app-photos", Some(1)),
            "the first window's fallback"
        );
        assert_eq!(o.turn_of("app-photos"), Some(2));
        assert!(o.take("app-photos", Some(2)));
    }

    #[test]
    fn windows_owe_apart() {
        let mut o = Owed(Vec::new());
        o.owe("app-notes", 1);
        o.owe("main", 2);
        assert!(o.take("main", None));
        assert_eq!(o.turn_of("app-notes"), Some(1));
    }
}
