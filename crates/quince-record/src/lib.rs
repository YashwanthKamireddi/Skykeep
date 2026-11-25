//! What Skykeep downloaded from Apple, and why.
//!
//! Twice a library of 57,000 photos downloaded tens of gigabytes a day with nothing opened
//! (D#167, D#258), and both times the cause was found only by reading the code: nothing said
//! what the bytes were for. This counts them, by reason, as they arrive.
//!
//! - **Where:** every body read from Apple goes through a few functions (quince-icloud's
//!   `download`, `bounded`, `drive_parts` and the Drive whole-file download); each calls
//!   [`landed`] with the bytes it just read. Bytes received are counted, not file sizes: a
//!   resumed download counts what arrived, a file sent twice counts twice.
//! - **Why:** a task-local reason, set where work starts ([`because`]): a sync pass, the grid
//!   being made ahead, a photo opened, a program reading the Drive folder. The innermost reason
//!   wins. `tokio::spawn` does not carry it: a spawn on a download path sets its own, or
//!   [`carried`] takes the spawner's along. Bytes with no reason land in [`Why::Other`], which
//!   should read nothing; if it doesn't, a counting site is missing its reason.
//! - **Kept:** bytes per reason for each of the last 30 days, and the programs that read the
//!   folders most, on this computer only (`save`/`load`). No file names, ever.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;

/// Why bytes were downloaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// No reason was set where the download started: a counting site to fix.
    Other,
    /// Pictures, videos and attachments the person opened.
    Opened,
    /// The grid's pictures and thumbnails.
    Grid,
    /// The newest photos kept ready ahead of being opened.
    KeptAhead,
    /// A video's own picture, when Apple's is blank.
    Posters,
    /// Drive and Photos folder files the person chose to keep on this computer.
    KeptOnComputer,
    /// A program reading a file in the Drive or Photos folder: its name.
    Folder(String),
    /// The backup.
    Backup,
    /// Downloads, exports and saves the person asked for.
    Saved,
    /// Staying up to date: sync passes, listings, sign-in.
    Sync,
    /// Skykeep's own updates.
    Updates,
}

/// The reasons, in the order they are kept (`Day::bytes` is indexed by `Why::code`).
pub const WHYS: usize = 11;

impl Why {
    /// Its place in a day's counts: stable, written to disk.
    #[must_use]
    pub fn code(&self) -> usize {
        match self {
            Why::Other => 0,
            Why::Opened => 1,
            Why::Grid => 2,
            Why::KeptAhead => 3,
            Why::Posters => 4,
            Why::KeptOnComputer => 5,
            Why::Folder(_) => 6,
            Why::Backup => 7,
            Why::Saved => 8,
            Why::Sync => 9,
            Why::Updates => 10,
        }
    }

    /// Its name as the record and the window say it.
    #[must_use]
    pub fn name(code: usize) -> &'static str {
        [
            "other",
            "opened",
            "grid",
            "keptAhead",
            "posters",
            "keptOnComputer",
            "folders",
            "backup",
            "saved",
            "sync",
            "updates",
        ]
        .get(code)
        .copied()
        .unwrap_or("other")
    }
}

tokio::task_local! {
    static WHY: Why;
}

/// `work`, with what it downloads counted under `why`.
pub async fn because<F: Future>(why: Why, work: F) -> F::Output {
    WHY.scope(why, work).await
}

/// `work`, a plain call (not a future: a file system's request on its own thread), with any
/// download it starts counted under `why`. A task it spawns takes the reason only if spawned
/// through [`carried`] while `work` runs.
pub fn because_now<R>(why: Why, work: impl FnOnce() -> R) -> R {
    WHY.sync_scope(why, work)
}

/// The reason the current task downloads for.
#[must_use]
pub fn current() -> Why {
    WHY.try_with(Clone::clone).unwrap_or(Why::Other)
}

/// `work`, carrying the current task's reason into a task about to be spawned for it.
pub fn carried<F: Future>(work: F) -> impl Future<Output = F::Output> {
    because(current(), work)
}

/// The most programs kept for a day; the rest are counted together.
pub const PROGRAMS: usize = 10;
/// Days kept.
pub const DAYS: usize = 30;
/// The name the programs past [`PROGRAMS`] are counted under.
pub const OTHER_PROGRAMS: &str = "other programs";

/// One day's downloads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// The local date, "2026-11-25".
    pub date: String,
    /// Bytes per reason, by `Why::code`.
    pub bytes: Vec<u64>,
    /// Bytes per program that read the folders.
    pub programs: BTreeMap<String, u64>,
}

impl Day {
    fn new(date: String) -> Day {
        Day {
            date,
            bytes: vec![0; WHYS],
            programs: BTreeMap::new(),
        }
    }

    fn add(&mut self, why: &Why, bytes: u64) {
        if self.bytes.len() < WHYS {
            self.bytes.resize(WHYS, 0);
        }
        self.bytes[why.code()] = self.bytes[why.code()].saturating_add(bytes);
        if let Why::Folder(program) = why {
            let known = self.programs.contains_key(program);
            let name = if known || self.programs.len() < PROGRAMS {
                program.as_str()
            } else {
                OTHER_PROGRAMS
            };
            let n = self.programs.entry(name.to_string()).or_default();
            *n = n.saturating_add(bytes);
        }
    }

    /// Every byte of the day.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.bytes.iter().sum()
    }
}

/// The last [`DAYS`] days, newest first.
#[derive(Default)]
pub struct Record {
    days: Mutex<VecDeque<Day>>,
    dirty: std::sync::atomic::AtomicBool,
}

impl Record {
    /// `bytes` downloaded on `date` (local, "YYYY-MM-DD") for `why`.
    pub fn add_on(&self, date: &str, why: &Why, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let mut days = self
            .days
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if days.front().is_none_or(|d| d.date != date) {
            days.push_front(Day::new(date.to_string()));
            days.truncate(DAYS);
        }
        if let Some(today) = days.front_mut() {
            today.add(why, bytes);
        }
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// The days kept, newest first.
    #[must_use]
    pub fn days(&self) -> Vec<Day> {
        self.days
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    /// Written to `path` if anything changed since the last time, whole or not at all.
    ///
    /// # Errors
    /// When the file can't be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if !self.dirty.swap(false, std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        let json = serde_json::to_vec(&self.days()).map_err(std::io::Error::other)?;
        let part = path.with_extension("json.part");
        std::fs::write(&part, json)?;
        std::fs::rename(&part, path).inspect_err(|_| {
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        })
    }

    /// What `path` kept, added to nothing counted yet (read once, at start). A missing or
    /// unreadable file is an empty record: it is a count, nothing in it is the person's.
    pub fn load(&self, path: &Path) {
        let Some(mut kept) = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<VecDeque<Day>>(&b).ok())
        else {
            return;
        };
        kept.truncate(DAYS);
        let mut days = self
            .days
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if days.is_empty() {
            *days = kept;
        }
    }
}

/// The record this process keeps.
pub static RECORD: std::sync::LazyLock<Record> = std::sync::LazyLock::new(Record::default);

/// Today's local date.
fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

/// `bytes` just read from Apple, counted under the current task's reason.
pub fn landed(bytes: u64) {
    RECORD.add_on(&today(), &current(), bytes);
}

/// `bytes` read by something that isn't one of ours (the updater's own client), under `why`.
pub fn landed_as(why: &Why, bytes: u64) {
    RECORD.add_on(&today(), why, bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes land under the reason their task was given, the innermost one when nested, and
    /// under Other with none; a spawned task carries its spawner's only when asked to.
    #[tokio::test]
    async fn bytes_land_under_the_reason_their_work_was_given() {
        let r = Record::default();
        let add = |n| r.add_on("2026-11-25", &current(), n);
        add(1);
        because(Why::Sync, async {
            add(10);
            because(Why::Posters, async { add(100) }).await;
        })
        .await;
        let (spawned, carried_along) = because(Why::Grid, async {
            let lost = tokio::spawn(async { current() }).await.unwrap();
            let kept = tokio::spawn(carried(async { current() })).await.unwrap();
            (lost, kept)
        })
        .await;
        assert_eq!(spawned, Why::Other, "a spawn carries nothing by itself");
        assert_eq!(carried_along, Why::Grid);
        let day = &r.days()[0];
        assert_eq!(day.bytes[Why::Other.code()], 1);
        assert_eq!(day.bytes[Why::Sync.code()], 10);
        assert_eq!(day.bytes[Why::Posters.code()], 100, "the innermost reason");
        assert_eq!(day.total(), 111);
    }

    /// A plain call (a file system request on its own thread) gives its reason to what it
    /// starts, and to a task it spawns carried while it runs.
    #[tokio::test]
    async fn a_plain_call_gives_its_reason_to_what_it_starts() {
        let (inside, spawned) = because_now(Why::Folder("nautilus".into()), || {
            (current(), tokio::spawn(carried(async { current() })))
        });
        assert_eq!(inside, Why::Folder("nautilus".into()));
        assert_eq!(spawned.await.unwrap(), Why::Folder("nautilus".into()));
        assert_eq!(current(), Why::Other, "only while it runs");
    }

    /// A day per date, newest first, never more than DAYS; programs past PROGRAMS share one
    /// line, so a day's record stays small however many programs read the folder.
    #[test]
    fn days_and_programs_are_bounded() {
        let r = Record::default();
        for d in 0..(DAYS + 5) {
            r.add_on(&format!("day-{d:03}"), &Why::Sync, 1);
        }
        let days = r.days();
        assert_eq!(days.len(), DAYS);
        assert_eq!(days[0].date, format!("day-{:03}", DAYS + 4), "newest first");
        for p in 0..(PROGRAMS + 7) {
            r.add_on("today", &Why::Folder(format!("prog{p}.exe")), 5);
        }
        r.add_on("today", &Why::Folder("prog0.exe".into()), 5);
        let today = &r.days()[0];
        assert_eq!(today.programs.len(), PROGRAMS + 1);
        assert_eq!(
            today.programs["prog0.exe"], 10,
            "a known program keeps its own line"
        );
        assert_eq!(today.programs[OTHER_PROGRAMS], 35);
        assert_eq!(today.bytes[Why::Folder(String::new()).code()], 90);
    }

    /// Saved only when something changed, read back as it was, and a damaged file reads as
    /// an empty record rather than failing.
    #[test]
    fn the_record_is_kept_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("download-record.json");
        let r = Record::default();
        r.add_on("2026-11-25", &Why::KeptAhead, 42);
        r.save(&path).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        r.save(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            before,
            "nothing changed, nothing written"
        );
        let again = Record::default();
        again.load(&path);
        assert_eq!(again.days(), r.days());
        std::fs::write(&path, b"{ damaged").unwrap();
        let fresh = Record::default();
        fresh.load(&path);
        assert!(fresh.days().is_empty());
    }
}
