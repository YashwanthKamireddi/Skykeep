//! Read commands for other programs: `skykeep --reminders-today` and the rest (docs/cli.md).
//!
//! Answered from the library on disk and nothing else: no window, no WebView, no hand-off to a
//! Skykeep already running, no network. A launcher, a status bar or a script gets its answer in
//! milliseconds and Skykeep's state is never touched (the library is opened read-only, as a
//! search's reader is, so a sync pass in a running Skykeep is neither waited on nor disturbed).
//!
//! The JSON written is a contract, not a dump of the library's own types: the field names here
//! are what other programs rely on, and the library's types can change underneath them freely.
//! Fields are only ever added.

use std::path::{Path, PathBuf};

use quince_core::Library;
use serde::Serialize;

/// The read commands this module answers, for the usage line and the check that `args` asks one.
const COMMANDS: &[&str] = &[
    "--reminder-lists",
    "--reminders-today",
    "--reminders",
    "--calendars",
    "--note-folders",
];

/// Exit codes: 0 answered; 1 asked wrongly or the library couldn't be read; 2 no library yet.
const ASKED_WRONGLY: i32 = 1;
const NO_LIBRARY: i32 = 2;

/// The exit code, when `args` asked a read command; `None` for every other launch. A read
/// command is the first argument and only the first, so nothing later on a launch's line (a
/// title, a file name) is ever taken for one.
pub fn answer(args: &[String]) -> Option<i32> {
    let asked = args.get(1).filter(|a| is_read(a))?;
    let Some(dir) = library_dir() else {
        eprintln!(
            "skykeep: Skykeep has no library on this computer yet. Open it and sign in first."
        );
        return Some(NO_LIBRARY);
    };
    let library = match Library::open_reader(&dir) {
        Ok(library) => library,
        Err(e) => {
            eprintln!("skykeep: the library couldn't be read: {e}");
            return Some(ASKED_WRONGLY);
        }
    };
    let now = jiff::Timestamp::now().as_second();
    let read = read(&library, asked, args, now);
    Some(match read {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(why) => {
            eprintln!("skykeep: {why}");
            ASKED_WRONGLY
        }
    })
}

fn is_read(arg: &str) -> bool {
    COMMANDS
        .iter()
        .any(|c| arg == *c || arg.strip_prefix(c).is_some_and(|r| r.starts_with('=')))
}

/// One command's answer as JSON, from `library` at `now`.
fn read(library: &Library, asked: &str, args: &[String], now: i64) -> Result<String, String> {
    let command = asked.split('=').next().unwrap_or(asked);
    let json = match command {
        "--reminder-lists" => to_json(
            library
                .reminder_lists()
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|l| l.enabled)
                .map(|l| ListOut {
                    id: l.guid,
                    title: l.title,
                })
                .collect::<Vec<_>>(),
        ),
        "--reminders-today" => {
            let until = end_of_day(now).ok_or("This computer's clock can't be read.")?;
            reminders_out(
                library
                    .reminders_smart(quince_core::reminders::SmartList::Today, until)
                    .map_err(|e| e.to_string())?,
            )
        }
        "--reminders" => {
            let list = value(args, "--reminders")
                .ok_or("--reminders takes a list id (see --reminder-lists).")?;
            let lists = library.reminder_lists().map_err(|e| e.to_string())?;
            if !lists.iter().any(|l| l.enabled && l.guid == list) {
                return Err(format!("No list in use has the id {list}."));
            }
            reminders_out(
                library
                    .reminders_in(&list, false)
                    .map_err(|e| e.to_string())?,
            )
        }
        "--calendars" => to_json(
            library
                .calendars()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|c| CalendarOut {
                    id: c.guid,
                    title: c.title,
                    writable: c.writable,
                    default: c.is_default,
                })
                .collect::<Vec<_>>(),
        ),
        "--note-folders" => to_json(
            library
                .note_folders()
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|f| !f.is_trash())
                .map(|f| FolderOut {
                    id: f.guid,
                    title: f.title,
                    parent: f.parent_guid,
                })
                .collect::<Vec<_>>(),
        ),
        other => return Err(format!("{other} isn't a read command.")),
    };
    json.map_err(|e| e.to_string())
}

fn to_json<T: Serialize>(value: T) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&value)
}

fn reminders_out(reminders: Vec<quince_core::reminders::Reminder>) -> serde_json::Result<String> {
    to_json(
        reminders
            .into_iter()
            .map(|r| ReminderOut {
                id: r.guid,
                list: r.list_guid,
                title: r.title,
                notes: r.notes.filter(|n| !n.trim().is_empty()),
                due: r.due_at,
                all_day: r.due_at.is_some() && r.all_day,
                flagged: r.flagged,
                priority: r.priority,
                parent: r.parent_guid,
            })
            .collect::<Vec<_>>(),
    )
}

/// The value after `flag` (`flag v` or `flag=v`).
fn value(args: &[String], flag: &str) -> Option<String> {
    let with = format!("{flag}=");
    args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix(&with)
            .map(ToString::to_string)
            .or_else(|| (a == flag).then(|| args.get(i + 1).cloned()).flatten())
            .filter(|v| !v.is_empty() && !v.starts_with("--"))
    })
}

/// The next local midnight after `now`: where Reminders' Today ends.
fn end_of_day(now: i64) -> Option<i64> {
    let zone = jiff::tz::TimeZone::system();
    let today = jiff::Timestamp::from_second(now)
        .ok()?
        .to_zoned(zone.clone())
        .date();
    Some(
        today
            .tomorrow()
            .ok()?
            .to_zoned(zone)
            .ok()?
            .timestamp()
            .as_second(),
    )
}

/// The folder Skykeep keeps its library in, found as the app finds it (`state_dir` in main.rs,
/// `migrate.rs`), before any app exists to ask: the current identifier's, then an earlier
/// identifier's still in use; on Windows the local folder before the roaming one.
fn library_dir() -> Option<PathBuf> {
    let roots = crate::migrate::Roots::system()?;
    let current = identifier()?;
    let ids = std::iter::once(current.as_str()).chain(crate::migrate::PREVIOUS.iter().copied());
    // Windows: Local app data once the library has moved there (migrate::to_local), else
    // Roaming. Elsewhere the one data folder.
    #[cfg(windows)]
    let bases: Vec<PathBuf> = roots.secondary.into_iter().chain([roots.primary]).collect();
    #[cfg(not(windows))]
    let bases = [roots.primary];
    ids.flat_map(|id| bases.iter().map(move |b| b.join(id)))
        .find(|dir| has_library(dir))
}

fn has_library(dir: &Path) -> bool {
    dir.join("library.db").is_file()
}

/// tauri.conf.json's identifier, as quince-auth reads it for the keyring.
fn identifier() -> Option<String> {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).ok()?;
    Some(conf.get("identifier")?.as_str()?.to_string())
}

#[derive(Serialize)]
struct ListOut {
    id: String,
    title: String,
}

#[derive(Serialize)]
struct CalendarOut {
    id: String,
    title: String,
    /// Events can be added to it (`--create-event --calendar`); false for a feed or a calendar
    /// shared read-only.
    writable: bool,
    /// The one `--create-event` uses when none is named.
    default: bool,
}

#[derive(Serialize)]
struct FolderOut {
    id: String,
    title: String,
    parent: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReminderOut {
    id: String,
    list: String,
    title: String,
    notes: Option<String>,
    /// Seconds since 1970; with `allDay`, that day's midnight UTC, as Apple keeps it.
    due: Option<i64>,
    all_day: bool,
    flagged: bool,
    /// Apple's: 0 none, 1 high, 5 medium, 9 low.
    priority: i64,
    /// The reminder this one is a subtask of.
    parent: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::reminders::{Reminder, ReminderList};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(ToString::to_string).collect()
    }

    /// Only a read command as the first argument is answered here; every other launch, a
    /// read command's name later on the line included, goes on as before.
    #[test]
    fn only_read_commands_are_answered_without_the_app() {
        for read in ["--reminders-today", "--reminders=L1", "--calendars"] {
            assert!(is_read(read), "{read}");
        }
        for launch in [
            "--app",
            "--reminders-todayish",
            "--create-reminder",
            "notes.md",
        ] {
            assert!(!is_read(launch), "{launch}");
        }
    }

    /// The JSON other programs read: ids, the lists in use, Today with overdue in it, a list's
    /// open reminders, and nothing about a list out of use or a folder of deleted notes.
    #[test]
    fn read_commands_answer_from_the_library() {
        let mut library = Library::in_memory().unwrap();
        library
            .put_reminder_lists(&[
                ReminderList {
                    guid: "L1".into(),
                    title: "Home".into(),
                    enabled: true,
                    ..Default::default()
                },
                ReminderList {
                    guid: "off".into(),
                    title: "Old".into(),
                    enabled: false,
                    ..Default::default()
                },
            ])
            .unwrap();
        let now = 1_795_000_000;
        let r = |guid: &str, due: Option<i64>, done: Option<i64>| Reminder {
            guid: guid.into(),
            list_guid: "L1".into(),
            title: guid.into(),
            due_at: due,
            completed_at: done,
            ..Default::default()
        };
        library
            .put_reminders(&[
                r("overdue", Some(now - 86_400), None),
                r("later", Some(now + 30 * 86_400), None),
                r("done", Some(now - 60), Some(now - 30)),
                r("someday", None, None),
            ])
            .unwrap();
        let ask = |a: &[&str]| {
            let a = args(a);
            let json = read(&library, &a[1], &a, now).unwrap();
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        };
        let ids = |v: serde_json::Value| -> Vec<String> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x["id"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(ids(ask(&["skykeep", "--reminder-lists"])), ["L1"]);
        assert_eq!(ids(ask(&["skykeep", "--reminders-today"])), ["overdue"]);
        let mut open = ids(ask(&["skykeep", "--reminders", "L1"]));
        open.sort();
        assert_eq!(open, ["later", "overdue", "someday"]);
        let today = ask(&["skykeep", "--reminders-today"]);
        assert_eq!(today[0]["due"], now - 86_400);
        assert_eq!(today[0]["allDay"], false);
        assert_eq!(today[0]["list"], "L1");
        assert!(read(
            &library,
            "--reminders",
            &args(&["skykeep", "--reminders", "off"]),
            now
        )
        .is_err());
        assert!(read(
            &library,
            "--reminders",
            &args(&["skykeep", "--reminders"]),
            now
        )
        .is_err());
    }
}
