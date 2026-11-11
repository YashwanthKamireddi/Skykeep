//! The local library: SQLite, and the only thing the UI ever reads.
//!
//! iCloud.com is slow because every scroll is a network round trip. Quince never renders
//! from the network — the sync engine reconciles into this database, and the interface reads
//! it. That is the whole reason the app can be instant and work offline.

pub mod alarms;
pub mod backup;
pub mod calendar;
pub mod changes;
pub mod contacts;
pub mod dav;
pub mod drive;
pub mod export;
pub mod files;
pub mod fold;
pub mod health;
pub mod notes;
pub mod outbox;
pub mod photos;
pub mod reminders;
pub mod schema;
pub mod search;
pub mod shared;

use rusqlite::Connection;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Any SQLite error, on opening or on any read or write after: "couldn't be opened" was
    /// said for a full disk on a write and for a broken constraint alike, and misled.
    #[error("the local library couldn't be read or saved: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The library was last opened by a newer Skykeep, whose schema this one does not know:
    /// written to by this one, its tables and triggers could break for both.
    #[error(
        "this library was made by a newer version of Skykeep (library version {found}; this \
         version knows up to {known}). Install the newer version again to open it"
    )]
    Newer { found: u32, known: u32 },
}

/// What stands in the way of a library that will not open, where it is the computer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trouble {
    /// No space left on the disk.
    Full,
    /// The folder, or the disk it is on, cannot be written to.
    ReadOnly,
    /// Another program holds the library's lock.
    Busy,
}

impl Error {
    /// The computer's part in this error, if it is one a person can fix there.
    #[must_use]
    pub fn trouble(&self) -> Option<Trouble> {
        use rusqlite::ErrorCode;
        match self {
            Error::Sqlite(rusqlite::Error::SqliteFailure(f, _)) => match f.code {
                ErrorCode::DiskFull => Some(Trouble::Full),
                ErrorCode::ReadOnly | ErrorCode::PermissionDenied | ErrorCode::CannotOpen => {
                    Some(Trouble::ReadOnly)
                }
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => Some(Trouble::Busy),
                _ => None,
            },
            Error::Io(e) => io_trouble(e),
            Error::Sqlite(_) | Error::Newer { .. } => None,
        }
    }

    /// The library file is damaged or not a database at all: set aside and made again
    /// ([`Library::open_or_recover`]), never written over.
    #[must_use]
    pub fn is_corrupt(&self) -> bool {
        use rusqlite::ErrorCode::{DatabaseCorrupt, NotADatabase};
        matches!(
            self,
            Error::Sqlite(rusqlite::Error::SqliteFailure(e, _))
                if matches!(e.code, DatabaseCorrupt | NotADatabase)
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The `app_state` keys that are this computer's, not an account's. The photo cache's size is
/// chosen for this computer's disk, in Settings, as the others are.
const COMPUTER_KEYS: &str = "key LIKE 'settings.%' OR key IN \
     ('drive_folder_on', 'drive_folder_path', 'photos_folder_on', 'photos_folder_path', \
      'photos.cacheCap')";

/// Which account a library holds (the app's `ACCOUNT_KEY`): carried over with what waits to be
/// sent when a damaged library is made again, so those changes stay that account's alone.
const ACCOUNT_KEY: &str = "account";

/// `app_state` keys a library makes for itself the moment it opens, whoever it is for: the
/// id this installation signs note edits with (the app's `documents.writer`). Not a sign that
/// any account's records are here.
const MADE_ON_OPEN: &str = "key IN ('documents.writer')";

/// Every table holding what an account brought: all but `app_state`, which is sorted key by key,
/// the search index's, which are not rows to delete one by one (`forget_account` empties it),
/// and the photo counts (migration 50), which are the photos' own triggers' to keep: one row a
/// view, always there, counted down to nothing as the photos go.
fn account_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name NOT LIKE 'sqlite_%' AND name != 'app_state'
         AND name != 'photo_view_counts'
         AND name NOT LIKE 'search\\_%' ESCAPE '\\'",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Ids as one JSON array, for `x NOT IN (SELECT value FROM json_each(?))`: one parameter
/// however many there are. A parameter each passed SQLite's limit of 32,766 on a Drive folder
/// or an address book that large, and its store failed on every pass.
pub(crate) fn id_list(ids: &[&str]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

/// A migration's part that SQL cannot do, run in the same transaction as the migration whose
/// number it carries.
pub type Fixup = (u32, fn(&rusqlite::Transaction<'_>) -> Result<()>);

/// Run every migration past the library's version, each with its version bump as one
/// transaction: a migration that fails part way, or a crash in the middle of one, leaves the
/// library at the version before it with none of it applied — never half a schema change
/// that the next open would try to make again over itself, nor a change made whose version
/// was never written.
fn migrate_with(conn: &mut Connection, migrations: &[&str], fixups: &[Fixup]) -> Result<()> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let known = u32::try_from(migrations.len()).unwrap_or(u32::MAX);
    if current > known {
        return Err(Error::Newer {
            found: current,
            known,
        });
    }
    for (version, sql) in migrations.iter().enumerate() {
        let target = u32::try_from(version).unwrap_or(u32::MAX) + 1;
        if current < target {
            let tx = conn.transaction()?;
            tx.execute_batch(sql)?;
            for (_, fixup) in fixups.iter().filter(|(at, _)| *at == target) {
                fixup(&tx)?;
            }
            // `user_version` lives in the database header, written inside the transaction.
            tx.pragma_update(None, "user_version", i64::from(target))?;
            tx.commit()?;
        }
    }
    Ok(())
}

/// [`Error::trouble`] for a failure of the file system itself.
#[must_use]
pub fn io_trouble(e: &std::io::Error) -> Option<Trouble> {
    use std::io::ErrorKind;
    match e.kind() {
        ErrorKind::StorageFull | ErrorKind::QuotaExceeded => Some(Trouble::Full),
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => Some(Trouble::ReadOnly),
        _ => None,
    }
}

/// Fold a library's log into the library itself, so the one file holds every commit: before
/// the file is moved. Answers whether that was done in full.
#[must_use]
pub fn checkpoint(db: &Path) -> bool {
    let Ok(conn) = Connection::open(db) else {
        return false;
    };
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
        r.get::<_, i64>(0)
    })
    .is_ok_and(|busy| busy == 0)
}

/// A library opened by [`Library::open_or_recover`]: where a damaged one was set aside, and
/// how many of its rows were carried over.
pub struct Opened {
    pub library: Library,
    pub set_aside: Option<std::path::PathBuf>,
    pub salvaged: usize,
}

/// Move a damaged `library.db`, and its `-wal` and `-shm` beside it, to
/// `library.corrupt-<time>.db…`: out of the way of a new one, and kept for whatever can be
/// got out of it. Answers the set-aside database's path.
fn set_aside(dir: &Path) -> Result<std::path::PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut name = format!("library.corrupt-{stamp}.db");
    let mut n = 1;
    while dir.join(&name).exists() {
        name = format!("library.corrupt-{stamp}-{n}.db");
        n += 1;
    }
    let aside = dir.join(&name);
    // The log and its index first: a database moved without them would be read, under its
    // old name, by the next open as a new library's.
    for suffix in ["-wal", "-shm"] {
        let from = dir.join(format!("library.db{suffix}"));
        if from.exists() {
            std::fs::rename(&from, dir.join(format!("{name}{suffix}")))?;
        }
    }
    std::fs::rename(dir.join("library.db"), &aside)?;
    Ok(aside)
}

/// A handle to the local library.
pub struct Library {
    conn: Connection,
    /// The zone all-day and floating events are read and stored in: this computer's, asked
    /// each time (`None`), so a change of zone is seen at once — or one given, for tests.
    zone: Option<jiff::tz::TimeZone>,
    /// What a change being watched has written so far, row by row (`changing_items`).
    watch: changes::Watch,
    /// The Years and Months views' months, as last worked out, and the version of the library
    /// and the zone they were worked out for (see `photo_months`).
    months: std::sync::Mutex<Option<photos::MonthsKept>>,
}

impl Library {
    /// Open (creating if needed) the library under `dir`, applying any pending migrations.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Self::from_connection(Connection::open(dir.join("library.db"))?)
    }

    /// [`Library::open`], and when the file is damaged — a power cut on a cheap disk, a bad
    /// sector, a sync tool that touched the folder — set it aside under a new name (kept,
    /// never deleted), make the library again, and carry over what exists nowhere else: the
    /// changes waiting for Apple, the uploads waiting to go, and this computer's settings, as
    /// far as they can still be read. Everything else is Apple's and comes back with the next
    /// pass. Any other failure — a full disk, a folder that can't be written, a library from
    /// a newer Skykeep — is returned as it is, for the person to be told.
    pub fn open_or_recover(dir: &Path) -> Result<Opened> {
        match Self::open(dir) {
            Ok(library) => Ok(Opened {
                library,
                set_aside: None,
                salvaged: 0,
            }),
            Err(e) if e.is_corrupt() => {
                let aside = set_aside(dir)?;
                let mut library = Self::open(dir)?;
                let salvaged = library.salvage(&aside);
                Ok(Opened {
                    library,
                    set_aside: Some(aside),
                    salvaged,
                })
            }
            Err(e) => Err(e),
        }
    }

    /// Copy what only this computer holds out of a set-aside library: every row that can
    /// still be read, stopping at the first that can't. Answers how many came over.
    fn salvage(&mut self, from: &Path) -> usize {
        use rusqlite::OpenFlags;
        let Ok(old) = Connection::open_with_flags(from, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return 0;
        };
        // The account's name goes with its changes: without it, a library holding one
        // account's changes named no owner, and was given to whoever signed in next.
        let computer =
            format!("SELECT * FROM app_state WHERE {COMPUTER_KEYS} OR key = '{ACCOUNT_KEY}'");
        let mut moved = 0;
        for (table, select) in [
            ("outbox", "SELECT * FROM outbox ORDER BY id".to_string()),
            ("photo_uploads", "SELECT * FROM photo_uploads".to_string()),
            ("app_state", computer),
        ] {
            moved += self.carry_over(&old, table, &select);
        }
        moved
    }

    /// Rows of `select` on `old` into this library's `table`, by the columns both have.
    fn carry_over(&mut self, old: &Connection, table: &str, select: &str) -> usize {
        let Ok(mut stmt) = old.prepare(select) else {
            return 0;
        };
        let theirs: Vec<String> = stmt.column_names().map(str::to_owned).collect();
        let ours: Vec<String> = self
            .conn
            .prepare(&format!("SELECT * FROM {table} LIMIT 0"))
            .map(|s| s.column_names().map(str::to_owned).collect())
            .unwrap_or_default();
        let shared: Vec<(usize, &String)> = theirs
            .iter()
            .enumerate()
            .filter(|(_, c)| ours.contains(c))
            .collect();
        if shared.is_empty() {
            return 0;
        }
        let names: Vec<&str> = shared.iter().map(|(_, c)| c.as_str()).collect();
        let marks = vec!["?"; names.len()].join(", ");
        let insert = format!(
            "INSERT OR REPLACE INTO {table} ({}) VALUES ({marks})",
            names.join(", ")
        );
        let Ok(mut rows) = stmt.query([]) else {
            return 0;
        };
        let mut moved = 0;
        // Each row as it is read: a damaged page part way keeps every row before it.
        while let Ok(Some(row)) = rows.next() {
            let values: rusqlite::Result<Vec<rusqlite::types::Value>> =
                shared.iter().map(|(i, _)| row.get(*i)).collect();
            let Ok(values) = values else { break };
            if self
                .conn
                .execute(&insert, rusqlite::params_from_iter(values))
                .is_ok()
            {
                moved += 1;
            }
        }
        moved
    }

    /// A second, read-only handle on the library under `dir`, which must already be open (and
    /// migrated) through [`Library::open`]: WAL lets it read while the one that writes holds its
    /// lock through a sync pass, so a search never waits behind one. It sees what was
    /// committed, never half a pass.
    pub fn open_reader(dir: &Path) -> Result<Self> {
        use rusqlite::OpenFlags;
        let conn = Connection::open_with_flags(
            dir.join("library.db"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "query_only", true)?;
        conn.busy_timeout(std::time::Duration::from_millis(250))?;
        Ok(Self {
            conn,
            zone: None,
            watch: changes::Watch::default(),
            months: std::sync::Mutex::new(None),
        })
    }

    /// An in-memory library. Tests, and nothing else.
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // Another handle holding the write lock — a reader checkpointing, a second Skykeep
        // starting as the first ends — is waited out rather than failed on at once: with no
        // wait, a launch that met it failed to open the library at all.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL lets the sync engine write while the interface reads, which is the normal
        // state of this app. FULL, not NORMAL: most of the library is a cache Apple can give
        // again, but the outbox and the uploads waiting are changes made here that exist
        // nowhere else, and under NORMAL the last commits before a power cut can be rolled
        // back. In WAL, FULL is one sync per commit — nothing at the pace edits are made.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut library = Self {
            conn,
            zone: None,
            watch: changes::Watch::default(),
            months: std::sync::Mutex::new(None),
        };
        library.migrate()?;
        changes::register(&library.conn, &library.watch)?;
        Ok(library)
    }

    /// Bring the schema up to date. Idempotent, and safe to call on every open.
    fn migrate(&mut self) -> Result<()> {
        migrate_with(&mut self.conn, schema::MIGRATIONS, schema::FIXUPS)
    }

    /// Run `apply` as one change to the library: everything it stores lands, or — if it
    /// fails, or the process dies part way — none of it does, and the library is as it was.
    ///
    /// A sync pass stores a service's records, tombstones, order and position in many calls;
    /// each was its own transaction, so a failure or a crash between them left half a pass —
    /// rows pruned and their replacements never stored, or records stored under a position
    /// that said they were not. Nests: every write in this crate is a savepoint, so one call
    /// inside another is part of the outer one.
    pub fn atomically<T, E>(
        &mut self,
        apply: impl FnOnce(&mut Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<Error>,
    {
        const NAME: &str = "quince_atomically";
        self.conn
            .execute_batch(&format!("SAVEPOINT {NAME}"))
            .map_err(|e| E::from(Error::from(e)))?;
        let undo = |conn: &Connection| {
            let _ = conn.execute_batch(&format!("ROLLBACK TO {NAME}; RELEASE {NAME}"));
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(&mut *self)));
        match outcome {
            Ok(Ok(value)) => match self.conn.execute_batch(&format!("RELEASE {NAME}")) {
                Ok(()) => Ok(value),
                Err(e) => {
                    undo(&self.conn);
                    Err(E::from(Error::from(e)))
                }
            },
            Ok(Err(e)) => {
                undo(&self.conn);
                Err(e)
            }
            // Left open, every later write would sit inside this savepoint and never commit.
            Err(panic) => {
                undo(&self.conn);
                std::panic::resume_unwind(panic)
            }
        }
    }

    /// Run `apply`, and say whether it changed any row outside the tables in `besides` — the
    /// bookkeeping a pass writes every time (where it got to, and when), whatever came back.
    ///
    /// A pass that brought nothing new still stores its position, so "rows were written" is
    /// true of every pass; "rows other than the position were written" is what tells a screen
    /// it has something to read again. Errs one way only: a change it cannot place is counted
    /// as a change.
    pub fn changing<T>(
        &mut self,
        besides: &'static [&'static str],
        apply: impl FnOnce(&mut Self) -> T,
    ) -> (T, bool) {
        use std::sync::atomic::{AtomicU64, Ordering};
        let aside = std::sync::Arc::new(AtomicU64::new(0));
        let counted = std::sync::Arc::clone(&aside);
        // A hook that could not be set counts nothing aside, so every row written counts as
        // a change: the one way this may err.
        let _ = self.conn.update_hook(Some(
            move |_: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                if besides.contains(&table) {
                    counted.fetch_add(1, Ordering::Relaxed);
                }
            },
        ));
        let before = self.conn.total_changes();
        // Not borrowed across `apply`: it takes the library whole. The hook comes off however
        // `apply` ends, a panic included; one left on would only go on counting into `aside`.
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(self)));
        let _ = self
            .conn
            .update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
        let out = match out {
            Ok(out) => out,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        let written = self.conn.total_changes().saturating_sub(before);
        (out, written > aside.load(Ordering::Relaxed))
    }

    /// [`Library::changing`], saying besides which of each app's items the change really
    /// touched — added, changed, removed — once rows stored again as they were cancel out
    /// (`changes`). A screen redraws those rows rather than reading its whole list again.
    pub fn changing_items<T>(
        &mut self,
        besides: &'static [&'static str],
        apply: impl FnOnce(&mut Self) -> T,
    ) -> (T, changes::Changed) {
        /// The watch ended however `apply` ends, a panic included.
        struct Unwatch(changes::Watch);
        impl Drop for Unwatch {
            fn drop(&mut self) {
                self.0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take();
            }
        }
        // Made where missing on each change, so a table made since (a walk's own) is watched
        // too. A library whose triggers cannot be made places nothing, and is read whole.
        let watching = changes::install(&self.conn).is_ok();
        if watching {
            *self
                .watch
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(changes::Tally::default());
        }
        let unwatch = Unwatch(std::sync::Arc::clone(&self.watch));
        let (out, written) = self.changing(besides, apply);
        let tally = unwatch
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let changed = match tally {
            Some(tally) => changes::place(tally, written),
            None => changes::Changed {
                written,
                items: None,
                arranged: std::collections::BTreeSet::new(),
            },
        };
        (out, changed)
    }

    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

impl Library {
    /// Remember something learned from the account, by name.
    pub fn set_state(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Everything learned from an account let go of — every reminder, note, event, contact,
    /// file and photo, every sync position — for another account to start clean. Kept: this
    /// computer's own choices (`settings.*`, which folders are on and where).
    ///
    /// Signed out and back in as someone else, the last account's data stayed and the new
    /// one's was laid over it, from the old account's sync positions.
    pub fn forget_account(&mut self) -> Result<()> {
        let tx = self.conn.savepoint()?;
        for table in account_tables(&tx)? {
            tx.execute(&format!("DELETE FROM \"{table}\""), [])?;
        }
        // The search index (migration 45) is not a table of records but words: its triggers
        // emptied it with the records, and this makes sure of it without reading it row by row.
        tx.execute_batch(
            "INSERT INTO search_text (search_text) VALUES ('delete-all'); DELETE FROM search_docs;",
        )?;
        tx.execute(
            &format!("DELETE FROM app_state WHERE NOT ({COMPUTER_KEYS})"),
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// This computer's own choices, which go with the computer, never with an account: carried
    /// over when one account's library is set aside for another's.
    pub fn computer_state(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT key, value FROM app_state WHERE {COMPUTER_KEYS}"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Whether anything of an account is here at all — a record, a sync position, a fact.
    pub fn holds_account(&self) -> Result<bool> {
        for table in account_tables(&self.conn)? {
            let any: bool = self.conn.query_row(
                &format!("SELECT EXISTS (SELECT 1 FROM \"{table}\")"),
                [],
                |r| r.get(0),
            )?;
            if any {
                return Ok(true);
            }
        }
        Ok(self.conn.query_row(
            &format!(
                "SELECT EXISTS (SELECT 1 FROM app_state \
                 WHERE NOT ({COMPUTER_KEYS}) AND NOT ({MADE_ON_OPEN}))"
            ),
            [],
            |r| r.get(0),
        )?)
    }

    /// What was remembered under that name, if anything.
    pub fn state(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM app_state WHERE key = ?1")?;
        let mut rows = stmt.query(rusqlite::params![key])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A damaged library is set aside — kept — and made again, with the changes still
    /// waiting for Apple and this computer's settings carried over. It used to stop the app
    /// at launch with no word at all.
    #[test]
    fn a_damaged_library_is_set_aside_and_its_waiting_changes_kept() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut lib = Library::open(dir.path()).unwrap();
            lib.outbox_push("reminders", "R1", "done", r#"{"done":true,"at":1}"#)
                .unwrap();
            lib.set_state("settings.alerts", "on").unwrap();
            lib.set_state("account", "someone").unwrap();
        }
        // Damaged after the header: SQLite's first read of the schema fails.
        let db = dir.path().join("library.db");
        let mut bytes = std::fs::read(&db).unwrap();
        let page = 4096.min(bytes.len());
        for b in &mut bytes[100..page] {
            *b = 0xA5;
        }
        std::fs::write(&db, &bytes).unwrap();
        assert!(Library::open(dir.path())
            .err()
            .is_some_and(|e| e.is_corrupt()));

        let opened = Library::open_or_recover(dir.path()).unwrap();
        let aside = opened.set_aside.expect("set aside");
        assert!(aside.exists(), "the damaged library is kept");
        assert_eq!(std::fs::read(&aside).unwrap(), bytes, "exactly as it was");
        let mut lib = opened.library;
        // Damage to the schema page leaves nothing readable here; what could be read is kept
        // (below). Either way the new library opens and works.
        lib.outbox_push("reminders", "R2", "done", "{}").unwrap();
    }

    #[test]
    fn what_can_still_be_read_is_carried_over() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.db");
        {
            let mut lib = Library::from_connection(Connection::open(&old).unwrap()).unwrap();
            lib.outbox_push("reminders", "R1", "done", r#"{"done":true,"at":1}"#)
                .unwrap();
            lib.set_state("settings.alerts", "on").unwrap();
            lib.set_state("account", "someone").unwrap();
            lib.conn
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
        }
        let mut fresh = Library::in_memory().unwrap();
        assert_eq!(
            fresh.salvage(&old),
            3,
            "the change waiting, the setting and whose they are"
        );
        assert_eq!(fresh.outbox_count().unwrap(), 1);
        assert_eq!(
            fresh.state("settings.alerts").unwrap().as_deref(),
            Some("on")
        );
        assert_eq!(
            fresh.state("account").unwrap().as_deref(),
            Some("someone"),
            "the change stays the account's that made it"
        );
    }

    /// A library from a newer Skykeep is refused, untouched, rather than written to by a
    /// version that does not know its tables.
    #[test]
    fn a_library_from_a_newer_version_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        drop(Library::open(dir.path()).unwrap());
        let newer = u32::try_from(schema::MIGRATIONS.len()).unwrap() + 2;
        Connection::open(dir.path().join("library.db"))
            .unwrap()
            .pragma_update(None, "user_version", newer)
            .unwrap();
        let err = Library::open_or_recover(dir.path()).err().unwrap();
        assert!(
            matches!(err, Error::Newer { found, .. } if found == newer),
            "{err}"
        );
        assert!(err.to_string().contains("newer version of Skykeep"));
        let still: u32 = Connection::open(dir.path().join("library.db"))
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(still, newer);
    }

    #[test]
    fn a_full_disk_and_a_folder_that_cannot_be_written_are_named() {
        let code = |c| {
            Error::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(c),
                None,
            ))
        };
        assert_eq!(
            code(rusqlite::ffi::SQLITE_FULL).trouble(),
            Some(Trouble::Full)
        );
        assert_eq!(
            code(rusqlite::ffi::SQLITE_READONLY).trouble(),
            Some(Trouble::ReadOnly)
        );
        assert_eq!(
            code(rusqlite::ffi::SQLITE_BUSY).trouble(),
            Some(Trouble::Busy)
        );
        let io = Error::Io(std::io::ErrorKind::ReadOnlyFilesystem.into());
        assert_eq!(io.trouble(), Some(Trouble::ReadOnly));
        assert!(code(rusqlite::ffi::SQLITE_CORRUPT).is_corrupt());
    }

    /// The writer waits for a lock another handle holds, rather than failing at once; and
    /// commits are synced in full, for the changes that exist only here.
    #[test]
    fn the_writer_waits_for_the_lock_and_syncs_in_full() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let wait: i64 = lib
            .conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert!(wait >= 5_000, "{wait}");
        let sync: i64 = lib
            .conn
            .query_row("PRAGMA synchronous", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sync, 2, "FULL");
    }

    /// Another account signing in finds nothing of the last one's, and this computer's own
    /// choices still where they were.
    #[test]
    fn an_account_is_forgotten_whole() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(
            "root",
            &[crate::drive::DriveItem {
                drivewsid: "FILE::x::1".into(),
                parent_id: Some("root".into()),
                name: "Plan".into(),
                extension: Some("pdf".into()),
                kind: "FILE".into(),
                size: Some(1),
                child_count: None,
                created_at: None,
                modified_at: None,
                etag: None,
            }],
        )
        .unwrap();
        lib.set_state("contacts.me", "C1").unwrap();
        lib.set_state("settings.alerts.events", "1").unwrap();
        lib.set_state("drive_folder_on", "1").unwrap();
        lib.mark_synced("drive", Some("token")).unwrap();
        assert!(lib.holds_account().unwrap());
        assert_eq!(
            lib.computer_state().unwrap().len(),
            2,
            "the settings and the folder, nothing of the account's"
        );
        lib.forget_account().unwrap();
        assert!(!lib.holds_account().unwrap(), "nothing left to set aside");
        // The id a library makes for itself as it opens is no account's records.
        lib.set_state("documents.writer", "W").unwrap();
        lib.set_state("photos.cacheCap", "1073741824").unwrap();
        assert!(
            !lib.holds_account().unwrap(),
            "a fresh library is not taken for one"
        );
        assert!(
            lib.drive_children("root").unwrap().is_empty(),
            "the files gone"
        );
        assert_eq!(
            lib.state("contacts.me").unwrap(),
            None,
            "the account's own facts gone"
        );
        assert_eq!(
            lib.sync_token("drive").unwrap(),
            None,
            "and where its sync had got to"
        );
        assert_eq!(
            lib.state("settings.alerts.events").unwrap().as_deref(),
            Some("1"),
            "this computer's settings kept"
        );
        assert_eq!(lib.state("drive_folder_on").unwrap().as_deref(), Some("1"));
    }

    /// A migration that fails part way leaves nothing of itself and the version where it was;
    /// the ones before it stay applied. Run again (fixed), it applies whole.
    #[test]
    fn a_failed_migration_leaves_the_library_at_the_version_before_it() {
        let mut conn = Connection::open_in_memory().unwrap();
        let good = "CREATE TABLE a (x INTEGER);";
        // The first statement would succeed on its own; the second fails.
        let bad = "CREATE TABLE b (x INTEGER); INSERT INTO nowhere VALUES (1);";
        assert!(migrate_with(&mut conn, &[good, bad], &[]).is_err());
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1, "the version bump went with the migration");
        let has = |conn: &Connection, t: &str| -> bool {
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = ?1)",
                [t],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert!(has(&conn, "a"), "the migration before it stays");
        assert!(!has(&conn, "b"), "nothing of the failed one persists");
        let fixed = "CREATE TABLE b (x INTEGER);";
        migrate_with(&mut conn, &[good, fixed], &[]).unwrap();
        assert!(has(&conn, "b"));
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }

    /// The same on disk, through a real file: what a crash mid-migration would find on reopen.
    #[test]
    fn a_failed_migration_is_not_on_disk_either() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            let _ = migrate_with(
                &mut conn,
                &[
                    "CREATE TABLE a (x INTEGER);",
                    "ALTER TABLE a ADD COLUMN y INTEGER; SELECT RAISE(ABORT, 'boom');",
                ],
                &[],
            );
        }
        let conn = Connection::open(&path).unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let columns: i64 = conn
            .query_row("SELECT count(*) FROM pragma_table_info('a')", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            columns, 1,
            "the column of the failed migration is not there"
        );
    }

    /// One change made of many core writes — each its own savepoint — is all or nothing.
    #[test]
    fn an_atomic_change_that_fails_leaves_nothing_behind() {
        let mut lib = Library::in_memory().unwrap();
        lib.set_state("kept", "before").unwrap();
        let failed: std::result::Result<(), Error> = lib.atomically(|l| {
            l.set_state("kept", "after")?;
            l.mark_synced("notes", Some("new-token"))?;
            l.forget_account()?;
            // A failure after all of that: none of it may persist.
            Err(Error::Sqlite(rusqlite::Error::InvalidQuery))
        });
        assert!(failed.is_err());
        assert_eq!(lib.state("kept").unwrap().as_deref(), Some("before"));
        assert_eq!(lib.sync_token("notes").unwrap(), None);
        assert!(lib.conn().is_autocommit(), "nothing left open");

        lib.atomically(|l| -> Result<()> {
            l.set_state("kept", "after")?;
            // Nested: part of the outer change.
            l.atomically(|l| l.set_state("inner", "1"))
        })
        .unwrap();
        assert_eq!(lib.state("kept").unwrap().as_deref(), Some("after"));
        assert_eq!(lib.state("inner").unwrap().as_deref(), Some("1"));

        // A nested change that fails takes only itself back when the outer one carries on.
        lib.atomically(|l| -> Result<()> {
            l.set_state("outer", "1")?;
            let _ = l.atomically(|l| -> Result<()> {
                l.set_state("inner", "2")?;
                Err(Error::Sqlite(rusqlite::Error::InvalidQuery))
            });
            Ok(())
        })
        .unwrap();
        assert_eq!(lib.state("outer").unwrap().as_deref(), Some("1"));
        assert_eq!(lib.state("inner").unwrap().as_deref(), Some("1"));
    }

    /// A panic part way rolls back and leaves the connection usable, not stuck inside a
    /// savepoint that would swallow every later write.
    #[test]
    fn a_panic_in_an_atomic_change_rolls_it_back() {
        let mut lib = Library::in_memory().unwrap();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<()> = lib.atomically(|l| {
                l.set_state("k", "v").unwrap();
                panic!("mid-apply");
            });
        }));
        assert!(caught.is_err());
        assert!(lib.conn().is_autocommit());
        assert_eq!(lib.state("k").unwrap(), None);
    }

    #[test]
    fn a_new_library_is_migrated_to_the_latest_schema() {
        let library = Library::in_memory().unwrap();
        assert_eq!(
            library.schema_version().unwrap() as usize,
            schema::MIGRATIONS.len()
        );
    }

    /// A library from before lists kept their look reads every list again once, or lists no
    /// one had touched would never get their colour and group. Only Reminders is reset.
    #[test]
    fn an_upgraded_library_reads_its_reminders_lists_whole() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = rusqlite::Connection::open(dir.path().join("library.db")).unwrap();
            for migration in &schema::MIGRATIONS[..21] {
                conn.execute_batch(migration).unwrap();
            }
            conn.pragma_update(None, "user_version", 21).unwrap();
            conn.execute_batch(
                "INSERT INTO sync_state (service, token) VALUES ('reminders', 'old'), ('notes', 'kept');",
            )
            .unwrap();
        }
        let library = Library::open(dir.path()).unwrap();
        assert_eq!(library.sync_token("reminders").unwrap(), None);
        // Notes is read from the start once too, by a later migration of its own: every note
        // has to have its formatting read, and only a full pass reaches the untouched ones.
        assert_eq!(library.sync_token("notes").unwrap(), None);
    }

    #[test]
    fn opening_twice_does_not_re_run_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let first = Library::open(dir.path()).unwrap();
        let version = first.schema_version().unwrap();
        drop(first);

        // A second open must be a no-op, not an error and not a reset.
        let second = Library::open(dir.path()).unwrap();
        assert_eq!(second.schema_version().unwrap(), version);
    }

    #[test]
    fn deleting_a_calendar_takes_its_events_with_it() {
        let library = Library::in_memory().unwrap();
        let conn = library.conn();
        conn.execute(
            "INSERT INTO calendars (guid, title, updated_at) VALUES ('cal-1', 'Work', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (guid, calendar_guid, title, starts_at, ends_at, updated_at)
             VALUES ('ev-1', 'cal-1', 'Standup', 100, 200, 0)",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM calendars WHERE guid = 'cal-1'", [])
            .unwrap();

        let orphans: i64 = conn
            .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(orphans, 0, "events must not outlive their calendar");
    }

    /// Re-syncing the same event must update it, not duplicate it.
    #[test]
    fn upserting_an_event_replaces_it() {
        let library = Library::in_memory().unwrap();
        let conn = library.conn();
        conn.execute(
            "INSERT INTO calendars (guid, title, updated_at) VALUES ('cal-1', 'Work', 0)",
            [],
        )
        .unwrap();

        for title in ["Standup", "Standup (moved)"] {
            conn.execute(
                "INSERT INTO events (guid, calendar_guid, title, starts_at, ends_at, updated_at)
                 VALUES ('ev-1', 'cal-1', ?1, 100, 200, 0)
                 ON CONFLICT(guid) DO UPDATE SET title = excluded.title",
                [title],
            )
            .unwrap();
        }

        let (count, title): (i64, String) = conn
            .query_row("SELECT count(*), max(title) FROM events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(count, 1, "a re-sync must not duplicate rows");
        assert_eq!(title, "Standup (moved)");
    }

    /// A pass that only moved its position changed nothing a screen shows; one that stored a
    /// record did. Rolled back or not, a change it cannot place counts as one.
    #[test]
    fn only_rows_beyond_the_bookkeeping_count_as_a_change() {
        let mut library = Library::in_memory().unwrap();
        let ((), changed) = library.changing(&["sync_state"], |l| {
            l.mark_synced("notes", Some("token-2")).unwrap();
            l.mark_full_sync("notes").unwrap();
        });
        assert!(!changed, "a new position alone is not a change");

        let ((), changed) = library.changing(&["sync_state"], |l| {
            l.mark_synced("notes", Some("token-3")).unwrap();
            l.set_state("k", "v").unwrap();
        });
        assert!(changed, "a row stored is a change");

        // The hook is gone afterwards: nothing it counted leaks into the next call.
        let ((), changed) = library.changing(&[], |l| l.set_state("k", "w").unwrap());
        assert!(changed);
        let ((), changed) = library.changing(&["sync_state"], |_| {});
        assert!(!changed);
    }
}
