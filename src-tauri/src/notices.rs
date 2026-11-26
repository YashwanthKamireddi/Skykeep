//! Desktop notifications on Linux, sent and answered over one session-bus connection.
//!
//! Each alert used to be shown through notify-rust on a thread of its own, blocked in
//! `wait_for_action` — with a connection of its own — until the notification closed. A reminder
//! alert stays (resident, no timeout) and GNOME and KDE keep it in their list indefinitely; a
//! notification server that restarts never says it closed. A tray app left running for weeks
//! with daily reminders gathered a thread and a D-Bus connection for every alert it had ever
//! shown. Now there is one connection, one thread listening on it, and the answer to each
//! notification is looked up by its id. The answers are listened for on the connection that
//! sent the notifications: a server may say which button was pressed to the sender alone.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use zbus::zvariant::Value;

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// What a press on a notification does, given the action's key.
type Answer = Box<dyn FnOnce(&str) + Send>;

/// One notification to show.
pub struct Notice<'a> {
    pub summary: &'a str,
    pub body: &'a str,
    pub icon: &'a str,
    /// (key, label) pairs; "default" is a click on the notification itself.
    pub actions: &'a [(&'a str, &'a str)],
    pub category: Option<&'a str>,
    pub desktop_entry: Option<String>,
    pub sound: Sound,
    /// Kept in the server's list after a press, and never timed out (a reminder).
    pub resident: bool,
}

/// What an alert sounds like, and who plays it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// The server's own choice, or none.
    Default,
    /// The server plays this sound from the desktop's sound theme.
    Named(&'static str),
    /// Played here already: the server is asked to add none of its own. A server that says
    /// it plays no sounds may still play one for every notification (Quickshell's does), and
    /// the alert sounded twice.
    PlayedHere,
}

/// How long an answer is waited for: a notification still unanswered after this was dismissed
/// unseen, or lost with a server that restarted.
const KEPT: Duration = Duration::from_secs(7 * 24 * 3600);

/// The answers waiting, by notification id.
#[derive(Default)]
struct Waiting(HashMap<u32, (Instant, Answer)>);

impl Waiting {
    fn add(&mut self, id: u32, now: Instant, answer: Answer) {
        self.0.retain(|_, (at, _)| now.duration_since(*at) < KEPT);
        self.0.insert(id, (now, answer));
    }
    /// The answer for `id`, once: a second press finds nothing.
    fn take(&mut self, id: u32) -> Option<Answer> {
        self.0.remove(&id).map(|(_, answer)| answer)
    }
}

fn waiting() -> &'static Mutex<Waiting> {
    static WAITING: OnceLock<Mutex<Waiting>> = OnceLock::new();
    WAITING.get_or_init(Mutex::default)
}

/// The one connection, and its listener started the first time it is asked for.
fn bus() -> Option<&'static zbus::blocking::Connection> {
    static BUS: OnceLock<Option<zbus::blocking::Connection>> = OnceLock::new();
    BUS.get_or_init(|| {
        let conn = zbus::blocking::Connection::session().ok()?;
        let listening = conn.clone();
        std::thread::Builder::new()
            .name("notices".into())
            .spawn(move || listen(&listening))
            .ok()?;
        Some(conn)
    })
    .as_ref()
}

fn listen(conn: &zbus::blocking::Connection) {
    let rule = || -> zbus::Result<zbus::MatchRule<'static>> {
        Ok(zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(SERVICE)?
            .path(PATH)?
            .build())
    };
    let Ok(rule) = rule() else {
        return;
    };
    let Ok(messages) = zbus::blocking::MessageIterator::for_match_rule(rule, conn, Some(64)) else {
        return;
    };
    for message in messages.flatten() {
        let header = message.header();
        match header.member().map(zbus::names::MemberName::as_str) {
            Some("ActionInvoked") => {
                if let Ok((id, key)) = message.body().deserialize::<(u32, String)>() {
                    let answer = waiting()
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take(id);
                    // On a thread of its own while it works (a tick waits on the library): the
                    // next signal is not held up behind it.
                    if let Some(answer) = answer {
                        std::thread::spawn(move || answer(&key));
                    }
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _reason)) = message.body().deserialize::<(u32, u32)>() {
                    let _ = waiting()
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take(id);
                }
            }
            _ => {}
        }
    }
}

/// Show `notice`; `answer` is called with the action's key when one is pressed. Its id, to
/// take it back with `close`.
pub fn show(notice: &Notice<'_>, answer: impl FnOnce(&str) + Send + 'static) -> Option<u32> {
    let conn = bus()?;
    let actions: Vec<&str> = notice
        .actions
        .iter()
        .flat_map(|(key, label)| [*key, *label])
        .collect();
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    if let Some(category) = notice.category {
        hints.insert("category", Value::from(category));
    }
    if let Some(entry) = &notice.desktop_entry {
        hints.insert("desktop-entry", Value::from(entry.as_str()));
    }
    match notice.sound {
        Sound::Default => {}
        Sound::Named(name) => {
            hints.insert("sound-name", Value::from(name));
        }
        Sound::PlayedHere => {
            hints.insert("suppress-sound", Value::from(true));
        }
    }
    if notice.resident {
        hints.insert("resident", Value::from(true));
    }
    // 0: never times out. -1: the server's own choice.
    let timeout: i32 = if notice.resident { 0 } else { -1 };
    // Registered before the reply is read, under the lock the listener takes: a press can
    // never arrive for an id not yet known.
    let mut held = waiting()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let reply = conn.call_method(
        Some(SERVICE),
        PATH,
        Some(SERVICE),
        "Notify",
        &(
            "Skykeep",
            0u32,
            notice.icon,
            notice.summary,
            notice.body,
            actions,
            hints,
            timeout,
        ),
    );
    let id = reply.and_then(|r| r.body().deserialize::<u32>()).ok()?;
    held.add(id, Instant::now(), Box::new(answer));
    Some(id)
}

/// Take back a notification shown with `show`: gone from the screen and the server's list.
/// The server says it closed, and its answer is let go of then (`listen`).
pub fn close(id: u32) {
    if let Some(conn) = bus() {
        let _ = conn.call_method(Some(SERVICE), PATH, Some(SERVICE), "CloseNotification", &id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Each answer reaches its own notification once; one never answered is let go of after a
    /// week, so nothing gathers however long Skykeep runs.
    #[test]
    fn answers_go_to_their_own_notification_once_and_none_gather() {
        let mut w = Waiting::default();
        let pressed = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        for id in 1..=3 {
            let pressed = Arc::clone(&pressed);
            w.add(
                id,
                start,
                Box::new(move |key| {
                    assert_eq!(key, "complete");
                    pressed.fetch_add(id as usize, Ordering::SeqCst);
                }),
            );
        }
        w.take(2).unwrap()("complete");
        assert_eq!(pressed.load(Ordering::SeqCst), 2);
        assert!(w.take(2).is_none(), "a second press finds nothing");
        // A week on, a new alert finds the old unanswered ones gone.
        w.add(9, start + KEPT, Box::new(|_| {}));
        assert_eq!(w.0.len(), 1);
    }
}
