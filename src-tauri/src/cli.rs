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
    "--events",
    "--search-notes",
    "--search-contacts",
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

/// Exit 1 when `args` asks for something to be made or changed and asks it wrongly (no title,
/// a time that isn't a number): said on stderr before anything starts, rather than a Skykeep
/// started, or a running one handed the launch, only to refuse it. What needs the library to
/// judge (a list id) is judged where the change is made.
pub fn refused(args: &[String]) -> Option<i32> {
    use crate::apps::{event_asked, note_asked, reminder_asked, EventAsk, NoteAsk, ReminderAsk};
    let why = match (reminder_asked(args), note_asked(args), event_asked(args)) {
        (Some(ReminderAsk::Refused(why)), _, _)
        | (_, Some(NoteAsk::Refused(why)), _)
        | (_, _, Some(EventAsk::Refused(why))) => why,
        _ => return None,
    };
    eprintln!("skykeep: {why}");
    Some(ASKED_WRONGLY)
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
        "--events" => {
            let days = match value(args, "--days") {
                None => 7,
                Some(d) => d
                    .parse::<i64>()
                    .ok()
                    .filter(|d| (1..=366).contains(d))
                    .ok_or_else(|| {
                        format!("--days takes a number of days from 1 to 366, not \"{d}\".")
                    })?,
            };
            to_json(
                library
                    .events_between(now, now + days * 86_400)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|e| EventOut {
                        id: e.guid,
                        calendar: e.calendar_guid,
                        title: e.title,
                        location: e.location.filter(|l| !l.trim().is_empty()),
                        start: e.starts_at,
                        end: e.ends_at,
                        all_day: e.all_day,
                        timezone: e.timezone,
                    })
                    .collect::<Vec<_>>(),
            )
        }
        "--search-notes" => {
            let query =
                value(args, "--search-notes").ok_or("--search-notes takes the words to find.")?;
            to_json(
                library
                    .search_notes(&query)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|n| NoteOut {
                        id: n.guid,
                        folder: n.folder_guid,
                        title: n.title,
                        // A locked note's words stay locked: its title only, as the phone lists it.
                        snippet: n.snippet.filter(|_| !n.locked),
                        locked: n.locked,
                        pinned: n.pinned,
                        modified: n.modified_at,
                    })
                    .collect::<Vec<_>>(),
            )
        }
        "--search-contacts" => {
            let query = value(args, "--search-contacts")
                .ok_or("--search-contacts takes the words to find.")?;
            to_json(
                library
                    .contacts(Some(&query))
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|c| {
                        let of = |kind: &str| {
                            c.methods
                                .iter()
                                .filter(|m| m.kind == kind)
                                .map(|m| MethodOut {
                                    label: m.label.clone(),
                                    value: m.value.clone(),
                                })
                                .collect::<Vec<_>>()
                        };
                        ContactOut {
                            phones: of("phone"),
                            emails: of("email"),
                            name: c.display_name(),
                            company: c.company.clone().filter(|n| !n.trim().is_empty()),
                            id: c.guid,
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        }
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventOut {
    id: String,
    calendar: String,
    title: String,
    location: Option<String>,
    /// Seconds since 1970. All day: local midnight to the next local midnight after the last day.
    start: i64,
    end: i64,
    all_day: bool,
    timezone: Option<String>,
}

#[derive(Serialize)]
struct NoteOut {
    id: String,
    folder: Option<String>,
    title: String,
    snippet: Option<String>,
    locked: bool,
    pinned: bool,
    /// Seconds since 1970.
    modified: Option<i64>,
}

#[derive(Serialize)]
struct ContactOut {
    id: String,
    name: String,
    company: Option<String>,
    phones: Vec<MethodOut>,
    emails: Vec<MethodOut>,
}

#[derive(Serialize)]
struct MethodOut {
    label: Option<String>,
    value: String,
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

    /// Events from now for the days asked (a week unless said), the one still running included;
    /// searches need words; a days count out of range is refused.
    #[test]
    fn events_and_searches_answer_from_the_library() {
        use quince_core::calendar::{Calendar, Event};
        let mut library = Library::in_memory().unwrap();
        library
            .put_calendars(&[Calendar {
                guid: "C".into(),
                title: "Home".into(),
                enabled: true,
                writable: true,
                ..Default::default()
            }])
            .unwrap();
        let now = 1_795_000_000;
        let e = |guid: &str, start: i64, hours: i64| Event {
            guid: guid.into(),
            calendar_guid: "C".into(),
            title: guid.into(),
            starts_at: start,
            ends_at: start + hours * 3600,
            timezone: Some("UTC".into()),
            ..Default::default()
        };
        library
            .put_events(&[
                e("running", now - 3600, 2),
                e("past", now - 86_400, 1),
                e("in3days", now + 3 * 86_400, 1),
                e("in10days", now + 10 * 86_400, 1),
            ])
            .unwrap();
        let ids = |a: &[&str]| -> Vec<String> {
            let a = args(a);
            let v: serde_json::Value =
                serde_json::from_str(&read(&library, &a[1], &a, now).unwrap()).unwrap();
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x["id"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(ids(&["skykeep", "--events"]), ["running", "in3days"]);
        assert_eq!(ids(&["skykeep", "--events", "--days", "1"]), ["running"]);
        assert_eq!(
            ids(&["skykeep", "--events", "--days=30"]),
            ["running", "in3days", "in10days"]
        );
        for bad in [
            vec!["skykeep", "--events", "--days", "0"],
            vec!["skykeep", "--events", "--days", "soon"],
            vec!["skykeep", "--search-notes"],
            vec!["skykeep", "--search-contacts"],
        ] {
            let a = args(&bad);
            assert!(read(&library, &a[1], &a, now).is_err(), "{bad:?}");
        }
        assert!(ids(&["skykeep", "--search-notes", "nothing"]).is_empty());
        assert!(ids(&["skykeep", "--search-contacts", "nobody"]).is_empty());
        // Found by a word in its text; a locked note found by its title shows no words; one in
        // Recently Deleted is never found.
        let note = |guid: &str, title: &str, folder: Option<&str>, locked: bool| {
            quince_core::notes::Note {
                guid: guid.into(),
                folder_guid: folder.map(Into::into),
                title: title.into(),
                snippet: Some(format!("{title} snippet")),
                body: Some(format!("{title}\nmilk and bread")),
                locked,
                pinned: false,
                created_at: None,
                modified_at: Some(now),
                editable: true,
                etag: None,
                runs: None,
            }
        };
        library
            .put_note_folders(&[quince_core::notes::NoteFolder {
                guid: quince_core::notes::TRASH_FOLDER.into(),
                title: "Recently Deleted".into(),
                sort_order: 0,
                parent_guid: None,
            }])
            .unwrap();
        library
            .put_notes(&[
                note("open", "Groceries", None, false),
                note("secret", "Groceries locked", None, true),
                note(
                    "gone",
                    "Groceries old",
                    Some(quince_core::notes::TRASH_FOLDER),
                    false,
                ),
            ])
            .unwrap();
        let a = args(&["skykeep", "--search-notes", "groc"]);
        let found: serde_json::Value =
            serde_json::from_str(&read(&library, &a[1], &a, now).unwrap()).unwrap();
        let mut by_id: Vec<(String, bool)> = found
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                (
                    n["id"].as_str().unwrap().to_string(),
                    n["snippet"].is_null(),
                )
            })
            .collect();
        by_id.sort();
        assert_eq!(
            by_id,
            [("open".to_string(), false), ("secret".to_string(), true)]
        );
    }
}
