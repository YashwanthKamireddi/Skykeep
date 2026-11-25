//! The move from identifiers the app had before (`PREVIOUS`) to the one it has now
//! (tauri.conf.json's `identifier`), once, at launch, before anything opens the library.
//!
//! The identifier names every folder the app keeps: the library and everything beside it
//! (`~/.local/share/<id>`, `%APPDATA%\<id>`), WebKitGTK's storage inside it (localStorage, its
//! caches), the window-state file (`~/.config/<id>`), the cache folder, and on Windows
//! WebView2's (`%LOCALAPPDATA%\<id>\EBWebView`). Changed without a move, an existing install
//! would open on an empty library, its unsent Drive files and signed-in session left behind.
//! The keyring entries move on their own, on first read (quince-auth `read_moving`).
//!
//! Crash safety. Each folder moves with one `rename` where it can: atomic, so a crash leaves it
//! either all old or all new. Where it cannot (another filesystem), it is copied into a staging
//! folder beside the new one, compared byte for byte, recorded as complete in a journal, and
//! only then put in place and the old one deleted. A journal left by a crash says which side
//! is whole: `copying` — the old one, and the staging copy is thrown away and redone; `copied` —
//! the copy, and the move is finished. Nothing is ever half in both places and read as whole.
//!
//! Never empty. If the move fails, this launch runs under the old identifier, from the old
//! folders as they were, and the next launch tries again. Both folders present (a build with
//! the new identifier ran before this one): a new folder with no library in it is set aside —
//! renamed, never deleted — and the old one moved in; if both hold a library, the larger one
//! is kept in place and the other set aside beside it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Identifiers this app was published under before, newest first (kept in quince-auth, which
/// moves the keyring entries by the same list). The current one is tauri.conf.json's.
pub use quince_auth::session::PREVIOUS_IDENTIFIERS as PREVIOUS;

/// One move: the folders of an identifier the app had (`old`) to the one it has (`new`).
#[derive(Clone, Copy, Debug)]
pub struct Ids<'a> {
    pub old: &'a str,
    pub new: &'a str,
}

/// Left in a folder that came from an earlier identifier (`.migrated-from-<old>`): it says the
/// folder is the established one, so an old build run later (which makes a fresh old folder)
/// is not mistaken for it.
const MARKER_PREFIX: &str = ".migrated-from-";

impl Ids<'_> {
    fn marker(&self) -> String {
        format!("{MARKER_PREFIX}{}", self.old)
    }
    /// Beside the folders, while a move is under way: the lock, and the phase of a copy.
    fn journal(&self) -> String {
        format!(".{}.migration", self.new)
    }
    /// Beside the folders, while a copy is under way.
    fn staging(&self) -> String {
        format!(".{}.partial", self.new)
    }
}

/// The folders the identifiers live in.
pub struct Roots {
    /// Where the library is: the move that decides which identifier this launch runs as.
    pub primary: PathBuf,
    /// The others (config, cache; Windows' local folder). They follow, and a failure there
    /// is logged and tried again next launch — nothing in them is the person's data.
    pub secondary: Vec<PathBuf>,
    /// The XDG autostart folder, for the Open at Login entry.
    pub autostart: Option<PathBuf>,
}

impl Roots {
    /// The folders Tauri's path resolver uses (the `dirs` crate's), from the environment, as
    /// it reads them: before the app exists there is no resolver to ask.
    pub fn system() -> Option<Self> {
        #[cfg(windows)]
        {
            let roaming = std::env::var_os("APPDATA").map(PathBuf::from)?;
            let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
            Some(Self {
                primary: roaming,
                secondary: local.into_iter().collect(),
                autostart: None,
            })
        }
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME").map(PathBuf::from)?;
            Some(Self {
                primary: home.join("Library/Application Support"),
                secondary: vec![home.join("Library/Caches")],
                autostart: None,
            })
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let home = std::env::var_os("HOME").map(PathBuf::from)?;
            // Only an absolute XDG path counts, as the spec (and `dirs`) say.
            let xdg = |var: &str, fallback: &str| {
                std::env::var_os(var)
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| home.join(fallback))
            };
            let config = xdg("XDG_CONFIG_HOME", ".config");
            Some(Self {
                primary: xdg("XDG_DATA_HOME", ".local/share"),
                secondary: vec![config.clone(), xdg("XDG_CACHE_HOME", ".cache")],
                autostart: Some(config.join("autostart")),
            })
        }
    }
}

/// How the launch goes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing under either identifier: a first run.
    Fresh,
    /// Moved now (or a move left half-done by a crash finished now).
    Moved,
    /// Already under the new identifier.
    Current,
    /// The move failed: this launch runs from the old folders, and the next one tries again.
    Fallback,
}

impl Outcome {
    /// The identifier this launch runs as.
    pub fn identifier(self, ids: Ids<'_>) -> &str {
        match self {
            Outcome::Fallback => ids.old,
            _ => ids.new,
        }
    }
}

/// A point in a move where a crash can land. Tests stop there; production never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Marked,
    SetAside,
    Renamed,
    Copying,
    CopiedOne,
    Verified,
    InPlace,
    Removing,
}

/// How a move is made. `Mover::default()` in production.
pub struct Mover<'a> {
    /// Copy even where a rename would do: what a move across filesystems takes.
    pub across: bool,
    /// Called at each `Step`; an error there stops the move as a crash would.
    pub checkpoint: &'a dyn Fn(Step) -> io::Result<()>,
}

impl Default for Mover<'_> {
    fn default() -> Self {
        Mover {
            across: false,
            checkpoint: &|_| Ok(()),
        }
    }
}

/// At launch, before Tauri starts: hand off to an instance already running under the current
/// identifier or an earlier one, else move what earlier identifiers left, and answer the
/// identifier to run as — `current` (tauri.conf.json's), or an earlier one whose folders could
/// not be moved this time.
pub fn at_launch(current: &str) -> String {
    // One running under the current identifier: handed this launch's arguments, as the
    // single-instance plugin would hand them, and this launch ends — here, before GTK, WebKit
    // and Tauri are brought up only for the plugin to find the same instance and exit. Nothing
    // is moved under a running app.
    crate::timing::mark("asking for a running instance");
    if instance::running(current) {
        instance::hand_off(current);
        crate::timing::mark("handed off");
        std::process::exit(0);
    }
    crate::timing::mark("none under the current identifier");
    // One running under an earlier one (the build before this one, still open after an
    // update, or this build after a failed move): it has the library open, so nothing moves
    // under it. It is shown, as the single-instance plugin would show it, and this launch ends.
    for old in PREVIOUS {
        if instance::running(old) {
            instance::hand_off(old);
            std::process::exit(0);
        }
    }
    let Some(roots) = Roots::system() else {
        return current.to_string();
    };
    for old in PREVIOUS.iter().filter(|old| **old != current) {
        let ids = Ids { old, new: current };
        let outcome = run(&roots, ids, &Mover::default());
        quiet!("quince: identifier: {old} -> {current}: {outcome:?}");
        if outcome == Outcome::Fallback {
            return outcome.identifier(ids).to_string();
        }
    }
    #[cfg(windows)]
    if let (Some(roaming), Some(local)) = (
        std::env::var_os("APPDATA").map(PathBuf::from),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    ) {
        let moved = to_local(&roaming.join(current), &local.join(current), |db| {
            quince_core::checkpoint(db)
        });
        quiet!("quince: the library to local app data: {moved:?}");
    }
    current.to_string()
}

/// Windows: the library and everything beside it — the photo cache of gigabytes among it —
/// from Roaming app data (`from`) to Local (`to`), where a machine's own data belongs. Roaming
/// is copied to the server at every sign-in and sign-out on a domain or school PC, and swept by
/// backup and OneDrive folder tools. Answers whether the library is in `to` afterwards (the
/// folder to run from).
///
/// Never half in both: the library's log is folded into it first (`checkpoint`), so the one
/// file holds every commit; each other entry moves with one rename; anything already in the
/// way, or a rename that fails (another volume, a file held open), puts back what moved and
/// leaves it all in Roaming for this launch, to try again the next. `library.db` moves last,
/// so its place says whether the move is done.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn to_local(from: &Path, to: &Path, checkpoint: impl Fn(&Path) -> bool) -> io::Result<bool> {
    let db = "library.db";
    if to.join(db).exists() || !from.join(db).exists() {
        return Ok(to.join(db).exists() || !from.join(db).exists());
    }
    if !checkpoint(&from.join(db)) {
        return Ok(false);
    }
    // What a checkpoint leaves can be an empty log; one with anything in it is not moved over.
    if fs::metadata(from.join(format!("{db}-wal"))).is_ok_and(|m| m.len() > 0) {
        return Ok(false);
    }
    let mut entries: Vec<std::ffi::OsString> = fs::read_dir(from)?
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .filter(|n| n != db)
        .collect();
    entries.sort();
    if entries.iter().any(|n| to.join(n).exists()) {
        return Ok(false);
    }
    fs::create_dir_all(to)?;
    let mut moved: Vec<&std::ffi::OsString> = Vec::new();
    let put_back = |moved: &[&std::ffi::OsString]| {
        for n in moved.iter().rev() {
            let _ = fs::rename(to.join(n), from.join(n));
        }
    };
    for name in entries
        .iter()
        .chain(std::iter::once(&std::ffi::OsString::from(db)))
    {
        if let Err(e) = fs::rename(from.join(name), to.join(name)) {
            put_back(&moved);
            quiet!("quince: stays in roaming app data for now: {e}");
            return Ok(false);
        }
        // The last one (`library.db`) is not borrowed past the loop: it is the end.
        if name != db {
            moved.push(name);
        }
    }
    Ok(true)
}

/// Everything that moves, in order: the library's folder before the other folders, since it
/// decides which identifier this launch runs as; the others only once it has moved.
pub fn run(roots: &Roots, ids: Ids<'_>, mover: &Mover) -> Outcome {
    // The login entry first, whatever happens to the folders: it starts the app, under any
    // identifier, and Settings looks for it under the new name.
    if let Some(dir) = &roots.autostart {
        if let Err(e) = autostart(dir, ids) {
            quiet!("quince: identifier: the login entry did not move ({e})");
        }
    }
    let outcome = match primary(&roots.primary, ids, mover) {
        Ok(outcome) => outcome,
        Err(e) => {
            quiet!("quince: identifier: the library did not move ({e})");
            return if old_is_whole(&roots.primary, ids) {
                distrust_copy(&roots.primary, ids);
                Outcome::Fallback
            } else {
                Outcome::Current
            };
        }
    };
    for parent in roots.secondary.iter().filter(|p| **p != roots.primary) {
        if let Err(e) = secondary(parent, ids, mover) {
            quiet!("quince: identifier: a folder did not move ({e}); trying again next launch");
        }
    }
    outcome
}

/// Whether, after a failed move, the old folder is still the whole library: it is there, and
/// no completed copy has replaced it (a `copied` journal with the new folder in place means
/// the old one may be part-deleted — the new one is the library then).
fn old_is_whole(parent: &Path, ids: Ids<'_>) -> bool {
    let copied = fs::read_to_string(parent.join(ids.journal())).is_ok_and(|j| j.trim() == COPIED);
    present(&parent.join(ids.old)) && !(copied && present(&parent.join(ids.new)))
}

/// About to run from the old folder: any copy of it made so far is stale the moment this launch
/// changes something there, so it is no longer complete — the next launch copies again rather
/// than finishing with it.
fn distrust_copy(parent: &Path, ids: Ids<'_>) {
    let journal = parent.join(ids.journal());
    if present(&journal) {
        let _ = fs::write(&journal, COPYING);
    }
    let _ = fs::remove_dir_all(parent.join(ids.staging()));
}

/// The library's folder.
fn primary(parent: &Path, ids: Ids<'_>, mover: &Mover) -> io::Result<Outcome> {
    let old = parent.join(ids.old);
    let new = parent.join(ids.new);
    let journal_path = parent.join(ids.journal());
    // The everyday case after the move, and a first run: two lookups and nothing written.
    if !present(&old) && !present(&journal_path) {
        return Ok(if present(&new) {
            Outcome::Current
        } else {
            Outcome::Fresh
        });
    }
    let mut journal = Journal::open(&journal_path)?;
    let outcome = primary_locked(parent, ids, &mut journal, mover)?;
    journal.finish()?;
    Ok(outcome)
}

fn primary_locked(
    parent: &Path,
    ids: Ids<'_>,
    journal: &mut Journal,
    mover: &Mover,
) -> io::Result<Outcome> {
    let (old, new) = (&parent.join(ids.old), &parent.join(ids.new));
    if resume(parent, ids, journal, mover)? {
        return Ok(Outcome::Moved);
    }
    if !present(old) {
        return Ok(if present(new) {
            Outcome::Current
        } else {
            Outcome::Fresh
        });
    }
    if present(new) {
        if holds_marker(new) {
            // Moved before; an old build has run since and made a fresh old folder. Not this
            // app's library: left exactly as it is.
            quiet!(
                "quince: identifier: {} left by an older build, not moved",
                ids.old
            );
            return Ok(Outcome::Current);
        }
        if holds_library(new) && weight(new) > weight(old) {
            // The new folder is the established library: the old one is set aside beside it.
            set_aside(old)?;
            write_marker(new, ids)?;
            return Ok(Outcome::Current);
        }
        // A stub (no library), or the smaller library: set aside, never written over.
        set_aside(new)?;
        (mover.checkpoint)(Step::SetAside)?;
    }
    move_dir(parent, ids, journal, mover, true)?;
    Ok(Outcome::Moved)
}

/// Any other folder: config, cache, Windows' local one.
fn secondary(parent: &Path, ids: Ids<'_>, mover: &Mover) -> io::Result<()> {
    let old = parent.join(ids.old);
    let new = parent.join(ids.new);
    let journal_path = parent.join(ids.journal());
    if !present(&old) && !present(&journal_path) {
        return Ok(());
    }
    let mut journal = Journal::open(&journal_path)?;
    if !resume(parent, ids, &mut journal, mover)? && present(&old) {
        if present(&new) {
            merge(&old, &new)?;
        } else {
            move_dir(parent, ids, &mut journal, mover, false)?;
        }
    }
    journal.finish()
}

/// A move a crash interrupted: finished if its copy was complete (`true`), its staging copy
/// thrown away if not, so it starts again from the old folder.
fn resume(parent: &Path, ids: Ids<'_>, journal: &mut Journal, mover: &Mover) -> io::Result<bool> {
    if journal.phase()? == COPIED {
        finish_copy(parent, ids, mover)?;
        return Ok(true);
    }
    let staging = parent.join(ids.staging());
    if present(&staging) {
        fs::remove_dir_all(&staging)?;
    }
    Ok(false)
}

/// `old` to `new`: renamed, or copied where a rename cannot cross.
fn move_dir(
    parent: &Path,
    ids: Ids<'_>,
    journal: &mut Journal,
    mover: &Mover,
    mark: bool,
) -> io::Result<()> {
    let (old, new) = (&parent.join(ids.old), &parent.join(ids.new));
    if mark {
        // Inside the old folder before it moves, so it arrives with it, whichever way.
        write_marker(old, ids)?;
        (mover.checkpoint)(Step::Marked)?;
    }
    if !mover.across {
        match rename(old, new) {
            Ok(()) => {
                sync_dir(parent);
                return (mover.checkpoint)(Step::Renamed);
            }
            Err(e) if crosses_devices(&e) => {}
            Err(e) => return Err(e),
        }
    }
    journal.set(COPYING)?;
    (mover.checkpoint)(Step::Copying)?;
    let staging = parent.join(ids.staging());
    copy_tree(old, &staging, mover)?;
    verify_tree(old, &staging)?;
    sync_dir(parent);
    // From here the copy is the whole library: a crash finishes the move, never redoes it.
    journal.set(COPIED)?;
    (mover.checkpoint)(Step::Verified)?;
    finish_copy(parent, ids, mover)
}

/// The verified copy put in place, then the old folder deleted.
fn finish_copy(parent: &Path, ids: Ids<'_>, mover: &Mover) -> io::Result<()> {
    let (old, new) = (&parent.join(ids.old), &parent.join(ids.new));
    let staging = parent.join(ids.staging());
    if present(&staging) {
        if present(new) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the new folder appeared while the copy was in progress",
            ));
        }
        rename(&staging, new)?;
        sync_dir(parent);
    }
    (mover.checkpoint)(Step::InPlace)?;
    if !present(old) {
        return Ok(());
    }
    for entry in fs::read_dir(old)? {
        let path = entry?.path();
        if fs::symlink_metadata(&path)?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
        (mover.checkpoint)(Step::Removing)?;
    }
    // A mount point cannot be removed; emptied, it no longer holds anything to move.
    if let Err(e) = fs::remove_dir(old) {
        quiet!("quince: identifier: the emptied old folder stays ({e})");
    }
    Ok(())
}

/// Whatever `old` holds that `new` does not, moved in; what both hold, `new`'s kept. For the
/// folders that are not the library: a window's size, caches.
fn merge(old: &Path, new: &Path) -> io::Result<()> {
    for entry in fs::read_dir(old)? {
        let entry = entry?;
        let to = new.join(entry.file_name());
        if present(&to) {
            continue;
        }
        match rename(&entry.path(), &to) {
            Ok(()) => {}
            Err(e) if crosses_devices(&e) => {
                quiet!("quince: identifier: left in the old folder: {e}");
            }
            Err(e) => return Err(e),
        }
    }
    // Gone only once empty: anything left is left where it was.
    let _ = fs::remove_dir(old);
    Ok(())
}

/// Open at Login (`~/.config/autostart`): the entry under the new name, then the old removed.
fn autostart(dir: &Path, ids: Ids<'_>) -> io::Result<()> {
    let old = dir.join(format!("{}.desktop", ids.old));
    let new = dir.join(format!("{}.desktop", ids.new));
    if !present(&old) {
        return Ok(());
    }
    if !present(&new) {
        let text = fs::read_to_string(&old)?;
        let mut moved = String::with_capacity(text.len());
        for line in text.lines() {
            // The Exec line stays as the person's install wrote it (an AppImage's own path).
            if line.trim_end() == format!("Icon={}", ids.old) {
                moved.push_str(&format!("Icon={}", ids.new));
            } else {
                moved.push_str(line);
            }
            moved.push('\n');
        }
        let part = dir.join(format!(".{}.desktop.part", ids.new));
        write_synced(&part, moved.as_bytes())?;
        rename(&part, &new)?;
        sync_dir(dir);
    }
    fs::remove_file(&old)
}

const COPYING: &str = "copying";
const COPIED: &str = "copied";

/// The journal beside the folders: held locked while a move runs (two launches at once —
/// login and a click — never move together), and saying how far a copy got.
struct Journal {
    file: fs::File,
    path: PathBuf,
}

impl Journal {
    fn open(path: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        loop {
            use std::os::unix::fs::MetadataExt;
            use std::os::unix::io::AsRawFd;
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            // SAFETY: flock on a descriptor this function owns.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(io::Error::last_os_error());
            }
            // The launch that held it may have finished and removed it while this one
            // waited: then this lock is on a file nobody else sees. Open it again.
            let held = file.metadata()?;
            match fs::metadata(path) {
                Ok(now) if now.ino() == held.ino() && now.dev() == held.dev() => {
                    return Ok(Self {
                        file,
                        path: path.to_path_buf(),
                    })
                }
                _ => continue,
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Opened unshared: a second launch is refused until the first lets go.
            let mut tries = 0;
            loop {
                match fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .share_mode(0)
                    .open(path)
                {
                    Ok(file) => {
                        return Ok(Self {
                            file,
                            path: path.to_path_buf(),
                        })
                    }
                    // ERROR_SHARING_VIOLATION: the other launch has it.
                    Err(e) if e.raw_os_error() == Some(32) && tries < 600 => {
                        tries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            Ok(Self {
                file,
                path: path.to_path_buf(),
            })
        }
    }

    fn phase(&mut self) -> io::Result<String> {
        use io::{Read, Seek};
        let mut text = String::new();
        self.file.seek(io::SeekFrom::Start(0))?;
        self.file.read_to_string(&mut text)?;
        Ok(text.trim().to_string())
    }

    fn set(&mut self, phase: &str) -> io::Result<()> {
        use io::{Seek, Write};
        self.file.set_len(0)?;
        self.file.seek(io::SeekFrom::Start(0))?;
        self.file.write_all(phase.as_bytes())?;
        self.file.sync_all()
    }

    /// The move done: the journal gone (on Windows, closed first — it is held unshared).
    fn finish(self) -> io::Result<()> {
        let Self { file, path } = self;
        #[cfg(windows)]
        drop(file);
        let removed = fs::remove_file(&path);
        #[cfg(not(windows))]
        drop(file);
        match removed {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// There, as itself: a symlink counts even if what it points at is gone.
fn present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn holds_library(dir: &Path) -> bool {
    present(&dir.join("library.db"))
}

/// How much library a folder holds: the database and what its WAL has not yet folded in.
fn weight(dir: &Path) -> u64 {
    ["library.db", "library.db-wal"]
        .iter()
        .filter_map(|f| fs::metadata(dir.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

/// `dir` renamed beside itself (`<name>.set-aside-<time>`), or removed if it is empty.
fn set_aside(dir: &Path) -> io::Result<()> {
    if fs::remove_dir(dir).is_ok() {
        return Ok(());
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut to = dir.with_file_name(format!("{name}.set-aside-{secs}"));
    let mut n = 1;
    while present(&to) {
        n += 1;
        to = dir.with_file_name(format!("{name}.set-aside-{secs}-{n}"));
    }
    quiet!(
        "quince: identifier: {} set aside as {}",
        dir.display(),
        to.display()
    );
    rename(dir, &to)
}

fn write_marker(dir: &Path, ids: Ids<'_>) -> io::Result<()> {
    write_synced(
        &dir.join(ids.marker()),
        format!(
            "This folder was {}, moved when the app's identifier changed.\n",
            ids.old
        )
        .as_bytes(),
    )
}

/// Whether `dir` came from an earlier identifier, whichever.
fn holds_marker(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(MARKER_PREFIX))
    })
}

/// `fs::rename`, made to hold on Windows: a file just written may be held for a moment by the
/// virus scanner or the search indexer (ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION), and
/// Windows will not rename over an existing empty folder as Linux does. Both are tried again
/// for up to two seconds; anything else is said at once.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let mut tries = 0;
        loop {
            match fs::rename(from, to) {
                Ok(()) => return Ok(()),
                Err(e) if matches!(e.raw_os_error(), Some(5 | 32)) && tries < 40 => {
                    if to.is_dir() && fs::read_dir(to).is_ok_and(|mut d| d.next().is_none()) {
                        let _ = fs::remove_dir(to);
                    }
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => return Err(e),
            }
        }
    }
    #[cfg(not(windows))]
    fs::rename(from, to)
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use io::Write;
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// A rename's own entries on the disk, not only the files' bytes. Unix only: Windows has no
/// way to sync a folder and its renames are journaled by NTFS.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

fn crosses_devices(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::CrossesDevices || (cfg!(unix) && e.raw_os_error() == Some(18))
}

/// `from` copied to `to` (which must not exist): files synced, folders with their
/// permissions, symlinks as symlinks.
fn copy_tree(from: &Path, to: &Path, mover: &Mover) -> io::Result<()> {
    fs::create_dir(to)?;
    fs::set_permissions(to, fs::metadata(from)?.permissions())?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let kind = fs::symlink_metadata(&src)?.file_type();
        if kind.is_symlink() {
            copy_link(&src, &dst)?;
        } else if kind.is_dir() {
            copy_tree(&src, &dst, mover)?;
        } else {
            fs::copy(&src, &dst)?;
            // Opened for writing: Windows refuses to flush a read-only handle (Access denied).
            fs::OpenOptions::new().write(true).open(&dst)?.sync_all()?;
            (mover.checkpoint)(Step::CopiedOne)?;
        }
    }
    sync_dir(to);
    Ok(())
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(src)?, dst)
}

#[cfg(not(unix))]
fn copy_link(src: &Path, dst: &Path) -> io::Result<()> {
    // Windows: what the link points at, copied as itself.
    if fs::metadata(src)?.is_dir() {
        copy_tree(src, dst, &Mover::default())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

/// `copy` holds exactly what `original` does: every entry, of the same kind, every file's
/// bytes the same.
fn verify_tree(original: &Path, copy: &Path) -> io::Result<()> {
    let differ = |what: &Path| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the copy differs at {}", what.display()),
        )
    };
    let mut seen = 0usize;
    for entry in fs::read_dir(original)? {
        let entry = entry?;
        seen += 1;
        let a = entry.path();
        let b = copy.join(entry.file_name());
        let (ka, kb) = (
            fs::symlink_metadata(&a)?.file_type(),
            fs::symlink_metadata(&b)
                .map_err(|_| differ(&a))?
                .file_type(),
        );
        if ka.is_symlink() && cfg!(unix) {
            if !kb.is_symlink() || fs::read_link(&a)? != fs::read_link(&b)? {
                return Err(differ(&a));
            }
        } else if ka.is_dir() {
            if !fs::metadata(&b)?.is_dir() {
                return Err(differ(&a));
            }
            verify_tree(&a, &b)?;
        } else if !same_bytes(&a, &b)? {
            return Err(differ(&a));
        }
    }
    if fs::read_dir(copy)?.count() != seen {
        return Err(differ(copy));
    }
    Ok(())
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    use io::Read;
    let (mut fa, mut fb) = (fs::File::open(a)?, fs::File::open(b)?);
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let (mut ba, mut bb) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let n = fa.read(&mut ba)?;
        if n == 0 {
            return Ok(true);
        }
        fb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

/// Whether a Skykeep is running under an identifier, and handing a launch to it: the same
/// channel tauri-plugin-single-instance listens on, so the running one opens what was asked
/// for (`--app notes`, a file) exactly as it would for a launch under its own identifier.
mod instance {
    /// The session bus, with a limit on every call: a running app that never answers must not
    /// keep this launch waiting for ever.
    #[cfg(target_os = "linux")]
    fn bus() -> Option<zbus::blocking::Connection> {
        zbus::blocking::connection::Builder::session()
            .ok()?
            .method_timeout(std::time::Duration::from_secs(5))
            .build()
            .ok()
    }

    #[cfg(target_os = "linux")]
    pub fn running(id: &str) -> bool {
        let Some(bus) = bus() else {
            return false;
        };
        bus.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &(format!("{id}.SingleInstance"),),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<bool>().ok())
        .unwrap_or(false)
    }

    #[cfg(target_os = "linux")]
    pub fn hand_off(id: &str) {
        let Some(bus) = bus() else {
            return;
        };
        let name = format!("{id}.SingleInstance");
        let path = format!("/{}", name.replace('.', "/").replace('-', "_"));
        let cwd = std::env::current_dir().unwrap_or_default();
        let _ = bus.call_method(
            Some(name.as_str()),
            path.as_str(),
            Some("org.SingleInstance.DBus"),
            "ExecuteCallback",
            &(
                std::env::args().collect::<Vec<String>>(),
                cwd.to_str().unwrap_or_default(),
            ),
        );
    }

    #[cfg(windows)]
    mod win {
        use std::ffi::c_void;

        #[repr(C)]
        pub struct CopyData {
            pub data: usize,
            pub len: u32,
            pub ptr: *const c_void,
        }

        #[link(name = "user32")]
        extern "system" {
            pub fn FindWindowW(class: *const u16, window: *const u16) -> *mut c_void;
            pub fn SendMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize)
                -> isize;
            pub fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
            pub fn AllowSetForegroundWindow(pid: u32) -> i32;
        }

        pub fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain(std::iter::once(0)).collect()
        }

        /// The plugin's hidden window for `id` (`<id>-sic` / `<id>-siw`).
        pub fn window(id: &str) -> *mut c_void {
            let class = wide(&format!("{id}-sic"));
            let name = wide(&format!("{id}-siw"));
            // SAFETY: two NUL-terminated UTF-16 strings that outlive the call.
            unsafe { FindWindowW(class.as_ptr(), name.as_ptr()) }
        }
    }

    #[cfg(windows)]
    pub fn running(id: &str) -> bool {
        !win::window(id).is_null()
    }

    #[cfg(windows)]
    pub fn hand_off(id: &str) {
        const WM_COPYDATA: u32 = 0x004A;
        /// tauri-plugin-single-instance's tag on the message.
        const SINGLE_INSTANCE_DATA: usize = 1542;
        let hwnd = win::window(id);
        if hwnd.is_null() {
            return;
        }
        let cwd = std::env::current_dir().unwrap_or_default();
        let args = std::env::args().collect::<Vec<String>>().join("|");
        let data = format!("{}|{args}\0", cwd.to_str().unwrap_or_default());
        let cds = win::CopyData {
            data: SINGLE_INSTANCE_DATA,
            len: u32::try_from(data.len()).unwrap_or(u32::MAX),
            ptr: data.as_ptr().cast(),
        };
        // SAFETY: a window handle just found; `cds` and `data` outlive the synchronous send.
        unsafe {
            let mut pid = 0;
            win::GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != 0 {
                win::AllowSetForegroundWindow(pid);
            }
            win::SendMessageW(hwnd, WM_COPYDATA, 0, std::ptr::addr_of!(cds) as isize);
        }
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    pub fn running(_id: &str) -> bool {
        false
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    pub fn hand_off(_id: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Any two identifiers: nothing here depends on which.
    const OLD_ID: &str = "dev.quince.client";
    const NEW_ID: &str = "org.example.Renamed";
    const IDS: Ids<'static> = Ids {
        old: OLD_ID,
        new: NEW_ID,
    };

    /// A temporary home: the three XDG roots and an autostart folder, nothing of the real one.
    struct Home {
        _dir: tempfile::TempDir,
        roots: Roots,
    }

    fn home() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("share");
        let config = dir.path().join("config");
        let cache = dir.path().join("cache");
        for d in [&data, &config, &cache] {
            fs::create_dir_all(d).unwrap();
        }
        let roots = Roots {
            primary: data,
            secondary: vec![config.clone(), cache],
            autostart: Some(config.join("autostart")),
        };
        Home { _dir: dir, roots }
    }

    impl Home {
        fn old(&self) -> PathBuf {
            self.roots.primary.join(OLD_ID)
        }
        fn current(&self) -> PathBuf {
            self.roots.primary.join(NEW_ID)
        }
        fn config(&self, id: &str) -> PathBuf {
            self.roots.secondary[0].join(id)
        }
    }

    /// What an install holds: a library whose last changes are only in its WAL, a Drive file
    /// waiting to upload, cookies, the cooldown, a photo, note media, WebKit's localStorage,
    /// and the window's size in the config folder.
    fn install(h: &Home) {
        let old = h.old();
        {
            // Ended as a crash ends it: no checkpoint, so the row lives in the WAL alone. Made
            // beside the folder and copied in, so this process holds nothing open inside it — a
            // crashed app holds nothing, and Windows will not rename a folder with a file open.
            let crashed = h.roots.primary.parent().unwrap().join("crashed-library");
            let mut library = quince_core::Library::open(&crashed).unwrap();
            library
                .set_state("remembered", "kept-through-the-move")
                .unwrap();
            std::mem::forget(library);
            fs::create_dir_all(&old).unwrap();
            for f in ["library.db", "library.db-wal", "library.db-shm"] {
                if crashed.join(f).exists() {
                    fs::copy(crashed.join(f), old.join(f)).unwrap();
                }
            }
        }
        assert!(
            fs::metadata(old.join("library.db-wal")).unwrap().len() > 0,
            "the test needs the change to be in the WAL"
        );
        let writes = quince_vfs::writes::Writes::new(&old.join("drive-writes")).unwrap();
        let file = writes.staging(7, "report.pdf");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"%PDF unsent").unwrap();
        let pending = quince_vfs::writes::Pending {
            folder: "FOLDER".into(),
            name: "report.pdf".into(),
            replaces: None,
            open: 0,
            dirty: true,
            generation: 0,
            modified: std::time::SystemTime::now(),
            sender: false,
            quiet_since: None,
            sending: false,
            settled: false,
            failure: None,
        };
        fs::write(
            old.join("drive-writes/pending.json"),
            serde_json::to_vec(&vec![(7u64, pending)]).unwrap(),
        )
        .unwrap();
        fs::write(old.join("cookies.json"), b"{\"cookies\":[]}").unwrap();
        fs::write(old.join("cooldown.json"), b"{\"consecutive_failures\":0}").unwrap();
        fs::create_dir_all(old.join("photos/full")).unwrap();
        fs::write(old.join("photos/full/a.heic"), vec![7u8; 200_000]).unwrap();
        fs::create_dir_all(old.join("note-media")).unwrap();
        fs::write(old.join("note-media/x.png"), b"png").unwrap();
        fs::create_dir_all(old.join("localstorage")).unwrap();
        fs::write(
            old.join("localstorage/tauri_localhost_0.localstorage"),
            b"screen=notes",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&old, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::create_dir_all(h.config(OLD_ID)).unwrap();
        fs::write(
            h.config(OLD_ID).join(".window-state.json"),
            b"{\"main\":{}}",
        )
        .unwrap();
    }

    /// Everything `install` put there, read back through the app's own code from `dir`.
    fn assert_whole(dir: &Path) {
        let library = quince_core::Library::open(dir).unwrap();
        assert_eq!(
            library.state("remembered").unwrap().as_deref(),
            Some("kept-through-the-move"),
            "the WAL's change survived"
        );
        drop(library);
        let waiting = quince_vfs::writes::Writes::new(&dir.join("drive-writes"))
            .unwrap()
            .load();
        assert_eq!(waiting.len(), 1, "the unsent Drive file is still waiting");
        assert_eq!(waiting[0].1.name, "report.pdf");
        assert_eq!(
            fs::read(dir.join("drive-writes/7/report.pdf")).unwrap(),
            b"%PDF unsent"
        );
        assert_eq!(
            fs::read(dir.join("photos/full/a.heic")).unwrap(),
            vec![7u8; 200_000]
        );
        assert!(dir.join("cookies.json").exists() && dir.join("cooldown.json").exists());
        assert_eq!(fs::read(dir.join("note-media/x.png")).unwrap(), b"png");
        assert_eq!(
            fs::read(dir.join("localstorage/tauri_localhost_0.localstorage")).unwrap(),
            b"screen=notes"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    fn no_leftovers(h: &Home) {
        for parent in std::iter::once(&h.roots.primary).chain(&h.roots.secondary) {
            assert!(
                !parent.join(IDS.journal()).exists(),
                "journal left in {}",
                parent.display()
            );
            assert!(
                !parent.join(IDS.staging()).exists(),
                "staging left in {}",
                parent.display()
            );
        }
    }

    #[test]
    fn a_fresh_install_moves_nothing_and_writes_nothing() {
        let h = home();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Fresh);
        assert_eq!(fs::read_dir(&h.roots.primary).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&h.roots.secondary[0]).unwrap().count(), 0);
        assert_eq!(Outcome::Fresh.identifier(IDS), NEW_ID);
    }

    #[test]
    fn an_install_moves_whole_with_its_wal_and_unsent_files() {
        let h = home();
        install(&h);
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        assert!(!h.old().exists());
        assert_whole(&h.current());
        assert!(h.current().join(IDS.marker()).exists());
        assert!(!h.config(OLD_ID).exists());
        assert!(h.config(NEW_ID).join(".window-state.json").exists());
        no_leftovers(&h);
        // And the next launch has nothing to do.
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Current);
    }

    #[test]
    fn across_filesystems_it_is_copied_verified_and_the_old_removed() {
        let h = home();
        install(&h);
        let across = Mover {
            across: true,
            ..Mover::default()
        };
        assert_eq!(run(&h.roots, IDS, &across), Outcome::Moved);
        assert!(!h.old().exists());
        assert_whole(&h.current());
        no_leftovers(&h);
    }

    /// A crash at every point of either kind of move: the launch it happens in runs from the
    /// old folder whole (or the new one, once the copy is complete) — never an empty or a half
    /// library — and the next launch finishes the move.
    #[test]
    fn a_move_interrupted_anywhere_is_resumed_without_loss() {
        let steps = [
            Step::Marked,
            Step::Renamed,
            Step::Copying,
            Step::CopiedOne,
            Step::Verified,
            Step::InPlace,
            Step::Removing,
        ];
        for across in [false, true] {
            for stop in steps {
                // The nth time a step is reached, for steps reached more than once.
                for nth in [1, 3] {
                    let h = home();
                    install(&h);
                    let count = Cell::new(0);
                    let crash = |s: Step| {
                        if s == stop {
                            count.set(count.get() + 1);
                            if count.get() == nth {
                                return Err(io::Error::other("crashed"));
                            }
                        }
                        Ok(())
                    };
                    let mover = Mover {
                        across,
                        checkpoint: &crash,
                    };
                    let first = run(&h.roots, IDS, &mover);
                    let label = format!("across={across} stop={stop:?} nth={nth}");
                    match first {
                        Outcome::Fallback => {
                            assert_whole(&h.old());
                            // This launch runs from the old folder and changes it: the next
                            // move must carry that, never an earlier copy.
                            fs::write(h.old().join("written-in-fallback"), b"1").unwrap();
                        }
                        Outcome::Moved | Outcome::Current => assert_whole(&h.current()),
                        Outcome::Fresh => panic!("{label}: started empty"),
                    }
                    // The next launch, uninterrupted.
                    let second = run(&h.roots, IDS, &Mover::default());
                    assert_ne!(second, Outcome::Fallback, "{label}");
                    assert_whole(&h.current());
                    if first == Outcome::Fallback {
                        assert!(
                            h.current().join("written-in-fallback").exists(),
                            "{label}: stale copy"
                        );
                    }
                    assert!(!h.old().exists(), "{label}: old left behind");
                    no_leftovers(&h);
                }
            }
        }
    }

    #[test]
    fn a_crash_while_copying_never_counts_the_partial_copy() {
        let h = home();
        install(&h);
        // As a crash leaves it: the journal saying the copy was under way, half a copy.
        fs::write(h.roots.primary.join(IDS.journal()), COPYING).unwrap();
        fs::create_dir_all(h.roots.primary.join(IDS.staging()).join("photos")).unwrap();
        fs::write(
            h.roots.primary.join(IDS.staging()).join("library.db"),
            b"half",
        )
        .unwrap();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        assert_whole(&h.current());
        no_leftovers(&h);
    }

    #[test]
    fn a_new_folder_that_is_only_a_stub_is_set_aside_not_kept() {
        let h = home();
        install(&h);
        // What a build with the new identifier leaves before it has synced anything.
        fs::create_dir_all(h.current().join("localstorage")).unwrap();
        fs::write(
            h.current()
                .join("localstorage/tauri_localhost_0.localstorage"),
            b"",
        )
        .unwrap();
        fs::write(h.current().join("cooldown.json"), b"{}").unwrap();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        assert_whole(&h.current());
        let aside: Vec<_> = fs::read_dir(&h.roots.primary)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(&format!("{NEW_ID}.set-aside-")))
            .collect();
        assert_eq!(aside.len(), 1, "the stub kept, renamed, never deleted");
    }

    #[test]
    fn an_empty_new_folder_is_simply_replaced() {
        let h = home();
        install(&h);
        fs::create_dir_all(h.current()).unwrap();
        fs::create_dir_all(h.config(NEW_ID)).unwrap();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        assert_whole(&h.current());
        assert!(
            h.config(NEW_ID).join(".window-state.json").exists(),
            "merged in"
        );
        assert_eq!(fs::read_dir(&h.roots.primary).unwrap().count(), 1);
    }

    #[test]
    fn when_both_hold_a_library_the_larger_stays_and_the_other_is_set_aside() {
        let h = home();
        install(&h);
        // A smaller library under the new identifier: the old one wins.
        quince_core::Library::open(&h.current()).unwrap();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        assert_whole(&h.current());

        // The other way round: an old folder smaller than the established new one.
        let h = home();
        install(&h);
        fs::rename(h.old(), h.current()).unwrap();
        quince_core::Library::open(&h.old()).unwrap();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Current);
        assert_whole(&h.current());
        assert!(!h.old().exists(), "set aside, so it is not tried again");
        assert!(h.current().join(IDS.marker()).exists());
    }

    #[test]
    fn an_old_folder_made_by_an_older_build_after_the_move_is_left_alone() {
        let h = home();
        install(&h);
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        // The older build run again: a fresh library under the old identifier, bigger even.
        fs::create_dir_all(h.old()).unwrap();
        fs::write(h.old().join("library.db"), vec![0u8; 50_000_000 / 1000]).unwrap();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Current);
        assert_whole(&h.current());
        assert!(h.old().join("library.db").exists(), "not touched");
    }

    #[cfg(unix)]
    #[test]
    fn a_move_that_fails_runs_from_the_old_folder_and_tries_again() {
        use std::os::unix::fs::PermissionsExt;
        let h = home();
        install(&h);
        // The folder above refuses the rename (and the journal).
        fs::set_permissions(&h.roots.primary, fs::Permissions::from_mode(0o555)).unwrap();
        let first = run(&h.roots, IDS, &Mover::default());
        fs::set_permissions(&h.roots.primary, fs::Permissions::from_mode(0o755)).unwrap();
        if first == Outcome::Moved {
            // Running as root: permissions do not stop it. Nothing to show here.
            return;
        }
        assert_eq!(first, Outcome::Fallback);
        assert_eq!(first.identifier(IDS), OLD_ID);
        assert_whole(&h.old());
        assert!(!h.current().exists());
        assert!(
            h.config(OLD_ID).exists(),
            "nothing else moved without the library"
        );
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        assert_whole(&h.current());
    }

    #[test]
    fn the_other_folders_merge_keeping_the_new_ones_files() {
        let h = home();
        fs::create_dir_all(h.config(OLD_ID)).unwrap();
        fs::write(h.config(OLD_ID).join(".window-state.json"), b"old").unwrap();
        fs::write(h.config(OLD_ID).join("other"), b"o").unwrap();
        fs::create_dir_all(h.config(NEW_ID)).unwrap();
        fs::write(h.config(NEW_ID).join(".window-state.json"), b"new").unwrap();
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Fresh);
        assert_eq!(
            fs::read(h.config(NEW_ID).join(".window-state.json")).unwrap(),
            b"new"
        );
        assert_eq!(fs::read(h.config(NEW_ID).join("other")).unwrap(), b"o");
        assert!(
            h.config(OLD_ID).join(".window-state.json").exists(),
            "left, not lost"
        );
    }

    #[test]
    fn the_login_entry_moves_to_the_new_name() {
        let h = home();
        let dir = h.roots.autostart.clone().unwrap();
        fs::create_dir_all(&dir).unwrap();
        let entry = "[Desktop Entry]\nType=Application\nName=Skykeep\nExec=\"/opt/Skykeep.AppImage\" --background\nIcon=dev.quince.client\nX-GNOME-Autostart-enabled=true\n";
        fs::write(dir.join(format!("{OLD_ID}.desktop")), entry).unwrap();
        run(&h.roots, IDS, &Mover::default());
        assert!(!dir.join(format!("{OLD_ID}.desktop")).exists());
        let moved = fs::read_to_string(dir.join(format!("{NEW_ID}.desktop"))).unwrap();
        assert!(moved.contains("Exec=\"/opt/Skykeep.AppImage\" --background\n"));
        assert!(moved.contains(&format!("Icon={NEW_ID}\n")));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no part file left");
    }

    /// Renamed twice: the folders of the first identifier reach the third, through the second
    /// or straight there, and an empty identifier in between changes nothing.
    #[test]
    fn a_second_rename_carries_the_first_ones_folders() {
        let h = home();
        install(&h);
        let third = "org.example.Third";
        assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
        let second = Ids {
            old: NEW_ID,
            new: third,
        };
        assert_eq!(run(&h.roots, second, &Mover::default()), Outcome::Moved);
        let first = Ids {
            old: OLD_ID,
            new: third,
        };
        assert_eq!(run(&h.roots, first, &Mover::default()), Outcome::Current);
        assert_whole(&h.roots.primary.join(third));
        assert_eq!(fs::read_dir(&h.roots.primary).unwrap().count(), 1);
    }

    #[test]
    fn a_symlinked_library_moves_as_the_link() {
        #[cfg(unix)]
        {
            let h = home();
            let elsewhere = tempfile::tempdir().unwrap();
            fs::write(elsewhere.path().join("library.db"), b"db").unwrap();
            std::os::unix::fs::symlink(elsewhere.path(), h.old()).unwrap();
            assert_eq!(run(&h.roots, IDS, &Mover::default()), Outcome::Moved);
            assert_eq!(fs::read_link(h.current()).unwrap(), elsewhere.path());
            assert_eq!(fs::read(h.current().join("library.db")).unwrap(), b"db");
        }
    }
}

#[cfg(test)]
mod local_tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn the_library_and_its_caches_move_to_local_whole() {
        let root = tempfile::tempdir().unwrap();
        let (from, to) = (root.path().join("Roaming/id"), root.path().join("Local/id"));
        write(&from.join("library.db"), "db");
        write(&from.join("library.db-wal"), "");
        write(&from.join("photos/thumb/a.jpg"), "a");
        write(&from.join("drive-writes/f"), "waiting");
        // WebView2 already keeps its folder in Local: left alone.
        write(&to.join("EBWebView/x"), "w");
        assert!(to_local(&from, &to, |_| true).unwrap());
        assert_eq!(fs::read_to_string(to.join("library.db")).unwrap(), "db");
        assert_eq!(
            fs::read_to_string(to.join("drive-writes/f")).unwrap(),
            "waiting"
        );
        assert!(to.join("photos/thumb/a.jpg").exists() && to.join("EBWebView/x").exists());
        assert!(!from.join("library.db").exists());
        // Done: asked again, it says so and moves nothing.
        assert!(to_local(&from, &to, |_| panic!("not asked")).unwrap());
    }

    #[test]
    fn anything_in_the_way_leaves_it_all_where_it_was() {
        let root = tempfile::tempdir().unwrap();
        let (from, to) = (root.path().join("Roaming/id"), root.path().join("Local/id"));
        write(&from.join("library.db"), "db");
        write(&from.join("photos/a"), "a");
        write(&to.join("photos/b"), "b");
        assert!(!to_local(&from, &to, |_| true).unwrap());
        assert!(from.join("library.db").exists() && from.join("photos/a").exists());
        // A log the checkpoint could not fold in: not moved either.
        let other = tempfile::tempdir().unwrap();
        let (from, to) = (other.path().join("R"), other.path().join("L"));
        write(&from.join("library.db"), "db");
        write(&from.join("library.db-wal"), "commits");
        assert!(!to_local(&from, &to, |_| true).unwrap());
        assert!(!to_local(&from, &to, |_| false).unwrap());
        assert!(from.join("library.db").exists());
    }
}
