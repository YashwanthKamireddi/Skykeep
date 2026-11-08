//! The photo cache held to the size chosen in Settings (2 GB unless changed), as Photos on a Mac
//! with "Optimize Storage" keeps what was looked at last and lets the rest go.
//!
//! What it holds, by folder under `photos/`:
//!
//! - `thumb/`, and `fit/<step>/` up to [`THUMB_STEP`]: the grid's pictures. Small, drawn by every
//!   view, and kept — the library scrolls from disk — but only while the disk has room: below
//!   [`GRID_FLOOR`] free, no more are kept and the longest unseen are let go of until there is
//!   room again ([`trim_grid`]). A library of a hundred thousand photos makes gigabytes of
//!   them, and nothing else held them to anything.
//! - `original/`, `full/`, `medium/`, `video/`, `live/`, the larger `fit/` steps, and `open/`
//!   (a video's name for the system's player, a hard link to its original): originals and
//!   previews. These are what the cap holds, the longest unseen let go first.
//!
//! The disk is the only record of what is cached: a picture is cached exactly when its file is
//! in place (`cached`, the Photos folder and the export all ask the disk), and no table anywhere
//! says otherwise. Every file arrives whole — written to a `.part` beside it and renamed — and
//! leaves in one unlink, so a crash at any moment leaves each picture either whole and cached or
//! gone and fetched again when next wanted. Never half of one. A crash mid-download leaves only
//! a `.part`, which is never taken for the picture and is swept once it is clearly abandoned.
//!
//! Never let go of: anything handed to a reader in the last few minutes (see [`hold_if_cached`]
//! — the photo on screen, a film playing, a file the Photos folder is reading), anything waiting
//! to be uploaded (its source may sit here), and any download still landing. Photos has no "Keep
//! on This Computer" of its own — that is the iCloud Drive folder's, which keeps its files in its
//! own cache (`drive-cache`), never walked here.
//!
//! Eviction runs on a blocking thread, one pass at a time, never on the window's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

pub const MB: u64 = 1 << 20;
pub const GB: u64 = 1 << 30;
/// The sizes Settings offers.
pub const STEPS: [u64; 7] = [512 * MB, GB, 2 * GB, 5 * GB, 10 * GB, 20 * GB, 50 * GB];
/// The cap until one is chosen.
pub const DEFAULT_CAP: u64 = 2 * GB;
/// Where the chosen cap is kept in the library's state.
const CAP_KEY: &str = "photos.cacheCap";
/// The largest `fit/` step that is a grid picture (made from the 512-pixel thumbnail): kept with
/// the thumbnails. Larger steps are the viewer's, and previews.
pub const THUMB_STEP: u32 = 512;
/// Seen (or handed to a reader) this recently: on screen, or being read — never let go of by a
/// trim. A trim that could not reach the cap for them is tried again once this has passed.
pub const RECENT: Duration = Duration::from_secs(5 * 60);
/// A download's `.part` untouched for this long belongs to a download that died with the app.
/// One still landing is written to every moment.
pub const STALE_PART: Duration = Duration::from_secs(60 * 60);
/// Free room on the disk below which the grid's pictures stop being kept, and the longest
/// unseen of them let go of: a full disk is never the price of scrolling.
pub const GRID_FLOOR: u64 = 2 * GB;
/// How far over [`GRID_FLOOR`] a trim of the grid's pictures brings the disk back, so the next
/// picture made does not trim again at once.
const GRID_SLACK: u64 = GRID_FLOOR / 4;
/// The folders of originals and previews.
const PREVIEWS: [&str; 6] = ["original", "full", "medium", "video", "live", "open"];

// ---------------------------------------------------------------------------------------------
// Readers' holds.

/// Files handed to a reader, and when. Eviction checks this under the same lock as it unlinks,
/// and a reader is handed a file only under it after seeing the file in place: a file is either
/// let go of before it is handed out (and the reader fetches it again) or after its hold ends.
static HELD: Mutex<Option<HashMap<PathBuf, Instant>>> = Mutex::new(None);

fn held() -> std::sync::MutexGuard<'static, Option<HashMap<PathBuf, Instant>>> {
    HELD.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether `path` is in the cache; when it is, it is held for [`RECENT`] so no trim lets it go
/// while it is being read.
pub fn hold_if_cached(path: &Path) -> bool {
    let mut map = held();
    let there = std::fs::metadata(path).is_ok_and(|m| m.is_file());
    if there {
        note(&mut map, path);
    }
    there
}

/// A file just put in place for a reader, held as [`hold_if_cached`] holds one.
pub fn hold(path: &Path) {
    note(&mut held(), path);
}

fn note(map: &mut Option<HashMap<PathBuf, Instant>>, path: &Path) {
    let map = map.get_or_insert_with(HashMap::new);
    if map.len() > 4096 {
        map.retain(|_, at| at.elapsed() < RECENT);
    }
    map.insert(path.to_path_buf(), Instant::now());
}

fn is_held(map: &Option<HashMap<PathBuf, Instant>>, path: &Path) -> bool {
    map.as_ref()
        .and_then(|m| m.get(path))
        .is_some_and(|at| at.elapsed() < RECENT)
}

// ---------------------------------------------------------------------------------------------
// What is on disk.

/// Room a file takes on disk: its blocks, so a sparse or tail-packed file counts what it uses.
fn on_disk(meta: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        meta.len()
    }
}

/// A file's identity: one file under two names (a video's `open/` link and its original) is
/// one picture, counted once and let go of whole.
fn identity(meta: &std::fs::Metadata, path: &Path) -> (u64, u64, Option<PathBuf>) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        (meta.dev(), meta.ino(), None)
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        (0, 0, Some(path.to_path_buf()))
    }
}

/// One picture that can be let go of: every name it has here, the room it takes, and when it
/// was last seen (its modified time, set again each time it is read: `photos::cached`).
#[derive(Debug, Clone)]
struct Picture {
    paths: Vec<PathBuf>,
    bytes: u64,
    seen: SystemTime,
    id: (u64, u64, Option<PathBuf>),
}

/// The originals and previews on disk, and the `.part` files beside them.
#[derive(Debug, Default)]
struct Scan {
    pictures: Vec<Picture>,
    parts: Vec<(PathBuf, SystemTime)>,
    used: u64,
}

/// Whether a `fit/` folder holds the grid's pictures (kept) or the viewer's (previews).
fn fit_is_preview(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .and_then(|s| s.parse::<u32>().ok())
        .is_some_and(|step| step > THUMB_STEP)
}

/// Every folder of originals and previews under `root`.
fn preview_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = PREVIEWS.iter().map(|d| root.join(d)).collect();
    for e in std::fs::read_dir(root.join("fit"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if fit_is_preview(&e.file_name()) {
            dirs.push(e.path());
        }
    }
    dirs
}

/// Files under `dir`, two levels at most (`open/<id>/<name>`), never following a link out.
fn walk(dir: &Path, depth: u8, out: &mut Vec<(PathBuf, std::fs::Metadata)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
            continue;
        };
        if meta.is_file() {
            out.push((e.path(), meta));
        } else if meta.is_dir() && depth > 0 {
            walk(&e.path(), depth - 1, out);
        }
    }
}

fn is_part(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "part")
}

fn scan(root: &Path) -> Scan {
    let mut files = Vec::new();
    for dir in preview_dirs(root) {
        walk(&dir, 1, &mut files);
    }
    let mut scan = Scan::default();
    let mut by_id: HashMap<(u64, u64, Option<PathBuf>), usize> = HashMap::new();
    for (path, meta) in files {
        let seen = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if is_part(&path) {
            scan.parts.push((path, seen));
            continue;
        }
        let id = identity(&meta, &path);
        match by_id.get(&id) {
            Some(&i) => scan.pictures[i].paths.push(path),
            None => {
                let bytes = on_disk(&meta);
                scan.used += bytes;
                by_id.insert(id.clone(), scan.pictures.len());
                scan.pictures.push(Picture {
                    paths: vec![path],
                    bytes,
                    seen,
                    id,
                });
            }
        }
    }
    scan
}

/// The grid's pictures on disk (thumbnails, and `fit/` steps up to [`THUMB_STEP`]).
fn grid_files(root: &Path) -> Vec<(PathBuf, std::fs::Metadata)> {
    let mut files = Vec::new();
    walk(&root.join("thumb"), 0, &mut files);
    for e in std::fs::read_dir(root.join("fit"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if !fit_is_preview(&e.file_name()) {
            walk(&e.path(), 0, &mut files);
        }
    }
    files.retain(|(p, _)| !is_part(p));
    files
}

/// Room the grid's pictures take.
fn thumbnails(root: &Path) -> u64 {
    grid_files(root).iter().map(|(_, m)| on_disk(m)).sum()
}

/// The grid's pictures let go of, the longest unseen first (last written, or read where the
/// disk keeps that), until the disk's `free` room has come back to `want` — none a reader
/// holds. Answers how many went.
pub fn trim_grid(root: &Path, free: u64, want: u64) -> usize {
    if free >= want {
        return 0;
    }
    let mut files = grid_files(root);
    files.sort_by_key(|(_, m)| last_seen(m));
    let mut freed = 0u64;
    let mut removed = 0;
    for (path, meta) in files {
        if free.saturating_add(freed) >= want {
            break;
        }
        if remove_unless_in_use(&path, &[]) {
            freed = freed.saturating_add(on_disk(&meta));
            removed += 1;
        }
    }
    removed
}

// ---------------------------------------------------------------------------------------------
// The other caches.

/// The caches beside Photos' own, each held to a size of its own, the longest unseen let go of
/// first: Drive's file thumbnails, contacts' faces, the pictures and files of notes and
/// reminders. Each is a picture or a file Apple still has, made again when next wanted — the
/// same ones "Clear Cached Pictures" lets go of. Before, nothing ever left them: every changed
/// face and every attachment opened stayed for good.
pub const SIDE: [(&str, u64); 5] = [
    ("thumbnails", 256 * MB),
    ("faces", 64 * MB),
    ("note-media", 512 * MB),
    ("note-files", 512 * MB),
    ("reminder-media", 256 * MB),
];

/// A grid picture of a photo the library no longer holds (deleted from iCloud), unseen this
/// long, is let go of: the grid's pictures are otherwise kept for good.
pub const GRID_ORPHAN: Duration = Duration::from_secs(30 * 24 * 3600);

/// When a file was last seen: read (where the disk keeps that) or written.
fn last_seen(meta: &std::fs::Metadata) -> SystemTime {
    let written = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
    meta.accessed().map_or(written, |read| read.max(written))
}

/// `dir` held to `cap` bytes, the longest unseen first; nothing seen in the last
/// [`RECENT`] (on screen, being opened), and a download's `.part` only once abandoned. Answers
/// how many files were let go of.
pub fn trim_lru(dir: &Path, cap: u64, now: SystemTime) -> usize {
    let mut files = Vec::new();
    walk(dir, 2, &mut files);
    let mut removed = 0;
    let mut kept: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for (path, meta) in files {
        let seen = last_seen(&meta);
        let age = now.duration_since(seen).unwrap_or_default();
        if is_part(&path) {
            if age > STALE_PART && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
            continue;
        }
        kept.push((seen, on_disk(&meta), path));
    }
    let mut used: u64 = kept.iter().map(|(_, b, _)| b).sum();
    kept.sort_by_key(|(seen, ..)| *seen);
    for (seen, bytes, path) in kept {
        if used <= cap {
            break;
        }
        if now.duration_since(seen).unwrap_or_default() < RECENT {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            used = used.saturating_sub(bytes);
            removed += 1;
        }
    }
    removed
}

/// Every cache in [`SIDE`] under the data folder `dir`, held to its size.
pub fn trim_side(dir: &Path, now: SystemTime) -> usize {
    SIDE.iter()
        .map(|(name, cap)| trim_lru(&dir.join(name), *cap, now))
        .sum()
}

/// Grid pictures (`thumb/`, small `fit/` steps) of photos not in `known`, unseen for
/// [`GRID_ORPHAN`]: a photo deleted from iCloud keeps no picture here for good.
pub fn prune_grid(
    root: &Path,
    known: &std::collections::HashSet<String>,
    now: SystemTime,
) -> usize {
    let mut dirs = vec![root.join("thumb")];
    for e in std::fs::read_dir(root.join("fit"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if !fit_is_preview(&e.file_name()) {
            dirs.push(e.path());
        }
    }
    let mut removed = 0;
    for dir in dirs {
        let mut files = Vec::new();
        walk(&dir, 0, &mut files);
        for (path, meta) in files {
            let Some(id) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if is_part(&path) || known.contains(id) {
                continue;
            }
            if now.duration_since(last_seen(&meta)).unwrap_or_default() > GRID_ORPHAN
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
    }
    removed
}

/// The other caches held to their sizes and the grid's orphans let go of: at start, then every
/// few hours for as long as Skykeep runs (a tray app can run for weeks). Off the window's
/// threads.
pub fn keep_side_caches(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        let dir = crate::state_dir(&app);
        let now = SystemTime::now();
        let side = trim_side(&dir, now);
        let known = {
            let data = app.state::<crate::Data>();
            let library = data.0.blocking_lock();
            library.photo_ids().ok()
        };
        // Only against a library that answered and holds photos: an empty one (signed out, a
        // first pass not in) says nothing about which pictures are gone.
        let grid = known
            .filter(|k| !k.is_empty())
            .map_or(0, |k| prune_grid(&root(&app), &k, now));
        if side + grid > 0 {
            quiet!(
                "caches: {side} files over their sizes and {grid} orphaned grid pictures let go of"
            );
        }
        std::thread::sleep(Duration::from_secs(6 * 3600));
    });
}

// ---------------------------------------------------------------------------------------------
// Letting go.

/// What a trim may not let go of, besides what readers hold.
pub struct Guard<'a> {
    /// Files never let go of, by identity: whatever is waiting to be uploaded.
    pub protected: &'a [(u64, u64, Option<PathBuf>)],
    /// Kept when seen within this long — on screen; `None` for an explicit Clear Cache.
    pub recent: Option<Duration>,
    pub now: SystemTime,
}

/// What a trim did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Originals and previews on disk before, and after, in bytes.
    pub before: u64,
    pub after: u64,
    pub removed: usize,
    /// It stopped short of the cap for pictures seen just now: worth trying again later.
    pub held_back: bool,
}

/// The identity of each file at `paths` that exists — what is waiting to be uploaded.
pub fn identities(paths: &[PathBuf]) -> Vec<(u64, u64, Option<PathBuf>)> {
    paths
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok().map(|m| identity(&m, p)))
        .collect()
}

/// One file let go of unless a reader holds it or it is waiting to be uploaded — checked and
/// removed under the readers' lock, as a trim does. Answers whether it is gone.
pub fn remove_unless_in_use(path: &Path, protected: &[(u64, u64, Option<PathBuf>)]) -> bool {
    let map = held();
    if is_held(&map, path)
        || std::fs::metadata(path).is_ok_and(|m| protected.contains(&identity(&m, path)))
    {
        return false;
    }
    std::fs::remove_file(path).is_ok()
}

/// One picture let go of, every name of it, unless a reader holds one. Its other names (the
/// player's link) go first and the cached name last: stopped between the two, the picture is
/// still whole and cached. Answers whether it is gone.
fn let_go(p: &Picture) -> bool {
    let map = held();
    if p.paths.iter().any(|path| is_held(&map, path)) {
        return false;
    }
    let mut order: Vec<&PathBuf> = p.paths.iter().collect();
    order.sort_by_key(|path| !path.components().any(|c| c.as_os_str() == "open"));
    for path in order {
        match std::fs::remove_file(path) {
            Ok(()) => {
                // A video's own folder for the player, empty now, goes with it.
                if let Some(dir) = path.parent().filter(|d| {
                    d.parent()
                        .and_then(Path::file_name)
                        .is_some_and(|n| n == "open")
                }) {
                    let _ = std::fs::remove_dir(dir);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            // Open elsewhere where that forbids it (Windows): it stays, whole.
            Err(_) => return false,
        }
    }
    true
}

/// `.part` files of downloads that died with the app, swept. One still landing is skipped.
fn sweep_parts(parts: &[(PathBuf, SystemTime)], now: SystemTime) {
    for (path, seen) in parts {
        if now.duration_since(*seen).is_ok_and(|age| age >= STALE_PART) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The originals and previews under `root` held to `cap`: when over it, the longest unseen go
/// first until they are back under nine tenths of it, so the next download does not trim again
/// at once. `stop_after` ends the pass after that many pictures, as a crash would (tests).
fn trim_with(root: &Path, cap: u64, guard: &Guard<'_>, stop_after: Option<usize>) -> Outcome {
    let scan = scan(root);
    sweep_parts(&scan.parts, guard.now);
    let mut out = Outcome {
        before: scan.used,
        after: scan.used,
        ..Outcome::default()
    };
    if scan.used <= cap {
        return out;
    }
    let low = cap / 10 * 9;
    let mut pictures = scan.pictures;
    pictures.sort_by_key(|p| p.seen);
    for p in &pictures {
        if out.after <= low || stop_after.is_some_and(|n| out.removed >= n) {
            break;
        }
        if guard.protected.contains(&p.id) {
            continue;
        }
        if let Some(window) = guard.recent {
            if guard
                .now
                .duration_since(p.seen)
                .map_or(true, |age| age < window)
            {
                out.held_back = true;
                continue;
            }
        }
        if let_go(p) {
            out.after = out.after.saturating_sub(p.bytes);
            out.removed += 1;
        } else {
            out.held_back = true;
        }
    }
    out
}

/// The cache under `root` held to `cap`. See [`trim_with`].
pub fn trim(root: &Path, cap: u64, guard: &Guard<'_>) -> Outcome {
    trim_with(root, cap, guard, None)
}

/// The most the cache may take on a disk: the cap chosen, or a fifth of the room it could have
/// (free space and its own) when that is less — a full disk is never the price of a cache.
pub fn limit(cap: u64, free: u64, used: u64) -> u64 {
    cap.min(free.saturating_add(used) / 5)
}

/// Free space on the disk holding `dir`, in bytes; `None` where it cannot be read.
fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: statvfs fills the struct it is given and reads only the path.
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut s) } == 0 {
            // 64-bit here already, narrower on some systems: the conversion is for those.
            #[allow(clippy::useless_conversion)]
            return Some(u64::from(s.f_bavail).saturating_mul(u64::from(s.f_frsize)));
        }
        None
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

/// The nearest size Settings offers to `bytes`.
pub fn snap(bytes: u64) -> u64 {
    STEPS
        .iter()
        .copied()
        .min_by_key(|s| s.abs_diff(bytes))
        .unwrap_or(DEFAULT_CAP)
}

// ---------------------------------------------------------------------------------------------
// In the app.

use crate::Data;
use tauri::{AppHandle, Emitter, Manager};

/// The cap chosen, once read from the library (0: not read yet).
static CAP: AtomicU64 = AtomicU64::new(0);
/// One pass at a time: a trim and Settings' Clear never run over each other.
static PASS: Mutex<()> = Mutex::new(());

/// What Settings shows of the cache.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Originals and previews on disk, in bytes.
    pub used: u64,
    /// The cap chosen.
    pub cap: u64,
    /// What it is held to now: the cap, or less while the disk is short of space.
    pub limit: u64,
    /// The grid's pictures, which always stay.
    pub thumbnails: u64,
    pub steps: Vec<u64>,
}

fn root(app: &AppHandle) -> PathBuf {
    crate::state_dir(app).join("photos")
}

fn parse_cap(saved: Option<String>) -> u64 {
    saved
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|b| STEPS.contains(b))
        .unwrap_or(DEFAULT_CAP)
}

/// The cap chosen. Off the async threads only: it may wait for the library.
fn cap_blocking(app: &AppHandle) -> u64 {
    match CAP.load(Ordering::SeqCst) {
        0 => {
            let saved = app
                .state::<Data>()
                .0
                .blocking_read(|library| library.state(CAP_KEY))
                .ok()
                .flatten();
            let cap = parse_cap(saved);
            CAP.store(cap, Ordering::SeqCst);
            cap
        }
        cap => cap,
    }
}

/// What is waiting to be uploaded, by identity. Off the async threads only.
fn waiting_uploads(app: &AppHandle) -> Vec<(u64, u64, Option<PathBuf>)> {
    let paths: Vec<PathBuf> = app
        .state::<Data>()
        .0
        .blocking_read(quince_core::Library::unfinished_uploads)
        .unwrap_or_default()
        .into_iter()
        .map(|u| PathBuf::from(u.path))
        .collect();
    identities(&paths)
}

fn status_of(root: &Path, cap: u64, used: u64) -> Status {
    let free = free_space(root).unwrap_or(u64::MAX / 2);
    Status {
        used,
        cap,
        limit: limit(cap, free, used),
        thumbnails: thumbnails(root),
        steps: STEPS.to_vec(),
    }
}

/// The cache held to its limit now, on this (blocking) thread. Answers the outcome and the
/// limit it was held to; tells the window what is there now.
pub fn trim_now(app: &AppHandle) -> (Outcome, u64) {
    let _pass = PASS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = root(app);
    if std::fs::create_dir_all(&root).is_err() {
        return (Outcome::default(), 0);
    }
    let cap = cap_blocking(app);
    let protected = waiting_uploads(app);
    let used = scan(&root).used;
    let held_to = limit(cap, free_space(&root).unwrap_or(u64::MAX / 2), used);
    let out = trim(
        &root,
        held_to,
        &Guard {
            protected: &protected,
            recent: Some(RECENT),
            now: SystemTime::now(),
        },
    );
    // The grid's pictures too, once the disk is short of room.
    let grid = free_space(&root)
        .filter(|free| *free < GRID_FLOOR)
        .map_or(0, |free| trim_grid(&root, free, GRID_FLOOR + GRID_SLACK));
    if out.removed + grid > 0 {
        let _ = app.emit(EVENT, status_of(&root, cap, out.after));
    }
    // Short of the limit only for what was on screen: tried again once that has had its time.
    if out.held_back && out.after > held_to {
        later(app, RECENT + Duration::from_secs(5));
    }
    (out, held_to)
}

/// The window's word that the cache changed: its [`Status`] now.
pub const EVENT: &str = "quince://photo-cache";

static RUNNING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);
static LATER: AtomicBool = AtomicBool::new(false);

/// A picture landed: the cap is checked soon, once, however many land together.
pub fn grew(app: &AppHandle) {
    request(app, Duration::from_secs(2));
}

/// Whether a grid picture made now may be kept on disk: while the disk has [`GRID_FLOOR`]
/// free (or cannot say how much it has).
pub fn grid_room(app: &AppHandle) -> bool {
    free_space(&root(app)).is_none_or(|free| free >= GRID_FLOOR)
}

/// A grid picture landed: nothing to do while the disk has room; short of it, a trim soon.
pub fn grid_grew(app: &AppHandle) {
    if !grid_room(app) {
        grew(app);
    }
}

/// A trim in the background after `after`; requests that come while one waits or runs are one.
fn request(app: &AppHandle, after: Duration) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(after).await;
            AGAIN.store(false, Ordering::SeqCst);
            let a = app.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || trim_now(&a)).await;
            if !AGAIN.load(Ordering::SeqCst) {
                break;
            }
        }
        RUNNING.store(false, Ordering::SeqCst);
        // One that came in the moment it stopped.
        if AGAIN.swap(false, Ordering::SeqCst) {
            request(&app, after);
        }
    });
}

/// One trim, `after` from now — however many ask meanwhile.
fn later(app: &AppHandle, after: Duration) {
    if LATER.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(after).await;
        LATER.store(false, Ordering::SeqCst);
        request(&app, Duration::ZERO);
    });
}

/// What the cache holds now, and what it is held to.
#[tauri::command]
pub async fn photos_cache_status(app: AppHandle) -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = root(&app);
        let cap = cap_blocking(&app);
        status_of(&root, cap, scan(&root).used)
    })
    .await
    .map_err(|e| e.to_string())
}

/// The cap set to one of the sizes offered (the nearest), kept, and held to in the background:
/// a lower one lets the longest unseen go at once. Answers at once with what is there now; the
/// trim's result follows as [`EVENT`].
#[tauri::command]
pub async fn photos_cache_set(bytes: u64, app: AppHandle) -> Result<Status, String> {
    let cap = snap(bytes);
    app.state::<Data>()
        .0
        .lock()
        .await
        .set_state(CAP_KEY, &cap.to_string())
        .map_err(|e| e.to_string())?;
    let lower = cap < CAP.swap(cap, Ordering::SeqCst);
    let status = {
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let root = root(&app);
            status_of(&root, cap, scan(&root).used)
        })
        .await
        .map_err(|e| e.to_string())?
    };
    if lower || status.used > status.limit {
        request(&app, Duration::ZERO);
    }
    Ok(status)
}

/// `clear` run as the one pass over the cache (no trim runs over it), off the window's thread;
/// the window is then told what the cache holds after. Settings' Clear goes through here.
pub fn clearing<T>(app: &AppHandle, clear: impl FnOnce() -> T) -> T {
    let out = {
        let _pass = PASS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear()
    };
    let root = root(app);
    let _ = app.emit(EVENT, status_of(&root, cap_blocking(app), scan(&root).used));
    out
}

#[cfg(test)]
mod side_tests {
    use super::*;

    fn put(dir: &Path, name: &str, bytes: usize, age: Duration) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![7u8; bytes]).unwrap();
        let at = SystemTime::now() - age;
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_times(std::fs::FileTimes::new().set_accessed(at).set_modified(at))
            .unwrap();
        path
    }

    /// The grid's pictures, kept while the disk has room, are let go of once it is short: the
    /// longest unseen first, only until the room wanted is back, never a picture a reader holds
    /// and never a preview (those are the cap's).
    #[test]
    fn the_grid_gives_way_when_the_disk_is_short() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let hour = Duration::from_secs(3600);
        let oldest = put(root, "thumb/a", 40_000, hour * 30);
        let old_fit = put(root, "fit/256/b", 40_000, hour * 20);
        let held = put(root, "thumb/c", 40_000, hour * 10);
        let newest = put(root, "thumb/d", 40_000, hour);
        let preview = put(root, "fit/2048/e", 40_000, hour * 40);
        hold(&held);
        let each = on_disk(&std::fs::metadata(&oldest).unwrap());

        assert_eq!(trim_grid(root, GRID_FLOOR, GRID_FLOOR), 0, "room enough");
        assert!(oldest.exists());

        // Short by a little more than two pictures: three go, the held one passed over.
        let free = GRID_FLOOR - 2 * each - 1;
        assert_eq!(trim_grid(root, free, GRID_FLOOR), 2 + 1);
        assert!(!oldest.exists() && !old_fit.exists() && !newest.exists());
        assert!(held.exists(), "a picture being read stays");
        assert!(preview.exists(), "a preview is the cap's, not the grid's");
    }

    /// Over its size, a cache lets go of the longest unseen first, never one just seen, and
    /// sweeps an abandoned download.
    #[test]
    fn a_cache_over_its_size_lets_the_longest_unseen_go_first() {
        let dir = tempfile::tempdir().unwrap();
        let hour = Duration::from_secs(3600);
        let oldest = put(dir.path(), "a/old", 40_000, hour * 30);
        let older = put(dir.path(), "b", 40_000, hour * 20);
        let newer = put(dir.path(), "c", 40_000, hour * 2);
        let on_screen = put(dir.path(), "d", 40_000, Duration::from_secs(10));
        let dead = put(dir.path(), ".e.1234.part", 10, hour * 2);
        let landing = put(dir.path(), ".f.5678.part", 10, Duration::from_secs(5));
        let used: u64 = [&oldest, &older, &newer, &on_screen]
            .iter()
            .map(|p| on_disk(&std::fs::metadata(p).unwrap()))
            .sum();
        // Room for two of the four.
        let removed = trim_lru(dir.path(), used / 2, SystemTime::now());
        assert_eq!(removed, 3, "two pictures and the dead download");
        assert!(!oldest.exists() && !older.exists() && !dead.exists());
        assert!(newer.exists() && on_screen.exists() && landing.exists());
    }

    /// Grid pictures of photos the library no longer holds go once long unseen; every other
    /// grid picture stays, as the grid is drawn from them.
    #[test]
    fn a_deleted_photos_grid_pictures_go_and_the_rest_stay() {
        let root = tempfile::tempdir().unwrap();
        let day = Duration::from_secs(24 * 3600);
        let kept = put(root.path(), "thumb/KEPT", 100, day * 90);
        let gone = put(root.path(), "thumb/GONE", 100, day * 90);
        let gone_fit = put(root.path(), "fit/256/GONE", 100, day * 90);
        let just_seen = put(root.path(), "thumb/NEW", 100, day);
        let preview = put(root.path(), "fit/2048/GONE", 100, day * 90);
        let known: std::collections::HashSet<String> = ["KEPT".to_string()].into();
        assert_eq!(prune_grid(root.path(), &known, SystemTime::now()), 2);
        assert!(kept.exists() && just_seen.exists() && preview.exists());
        assert!(!gone.exists() && !gone_fit.exists());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every original and preview under `root` let go of, but what is held, protected or still
    /// landing. The grid's pictures stay.
    fn clear(root: &Path, guard: &Guard<'_>) -> Outcome {
        trim_with(
            root,
            0,
            &Guard {
                recent: None,
                ..*guard
            },
            None,
        )
    }

    /// A cached picture at `rel` of `kb` kilobytes, last seen `ago` ago.
    fn put(root: &Path, rel: &str, kb: usize, ago: Duration) -> PathBuf {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![7u8; kb * 1024]).unwrap();
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(SystemTime::now() - ago).unwrap();
        p
    }

    fn mins(n: u64) -> Duration {
        Duration::from_secs(n * 60)
    }

    fn guard(protected: &[(u64, u64, Option<PathBuf>)]) -> Guard<'_> {
        Guard {
            protected,
            recent: Some(RECENT),
            now: SystemTime::now(),
        }
    }

    /// Every file under `root` is whole (all its bytes the ones written) or not there at all.
    fn all_whole(root: &Path) {
        let mut files = Vec::new();
        for dir in [
            "thumb", "original", "full", "medium", "video", "fit/128", "fit/1536", "open",
        ] {
            walk(&root.join(dir), 1, &mut files);
        }
        for (p, m) in files {
            if is_part(&p) {
                continue;
            }
            let bytes = std::fs::read(&p).unwrap();
            assert!(
                bytes.len() as u64 == m.len() && bytes.iter().all(|&b| b == 7),
                "{p:?} is whole"
            );
        }
    }

    /// Over the cap, the longest unseen go first, down to nine tenths of it; the grid's
    /// pictures always stay, and so does anything seen in the last few minutes.
    #[test]
    fn the_longest_unseen_go_first_and_thumbnails_stay() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let thumb = put(r, "thumb/A", 400, mins(900));
        let grid = put(r, "fit/256/A", 400, mins(900));
        let oldest = put(r, "original/A", 400, mins(600));
        let old = put(r, "full/B", 400, mins(500));
        let newer = put(r, "medium/C", 400, mins(100));
        let viewer = put(r, "fit/1536/D", 400, mins(50));
        let on_screen = put(r, "original/E", 400, mins(1));
        let before = scan(r).used;
        assert!(
            before >= 5 * 400 * 1024,
            "the five previews count, not the grid's: {before}"
        );
        // Room for three of the five under nine tenths of the cap: two go, the two seen
        // longest ago.
        let cap = 1_400_000;
        let out = trim(r, cap, &guard(&[]));
        assert_eq!(out.removed, 2, "{out:?}");
        assert!(!oldest.exists() && !old.exists());
        for kept in [&thumb, &grid, &newer, &viewer, &on_screen] {
            assert!(kept.exists(), "{kept:?} stays");
        }
        assert!(out.after <= cap / 10 * 9 && out.after == scan(r).used);
        // Under the cap: nothing more goes.
        assert_eq!(trim(r, cap, &guard(&[])).removed, 0);
        all_whole(r);
    }

    /// The cap lowered: the cache goes down to it at once, oldest first — but never the picture
    /// on screen, which is tried again once it has had its time.
    #[test]
    fn a_lower_cap_is_held_to_at_once() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        for i in 0..10 {
            put(r, &format!("original/P{i}"), 100, mins(100 - i));
        }
        let shown = put(r, "full/SHOWN", 100, Duration::from_secs(10));
        let out = trim(r, 5 * 104 * 1024, &guard(&[]));
        assert!(out.after <= 5 * 104 * 1024, "{out:?}");
        assert!(!r.join("original/P0").exists() && r.join("original/P9").exists());
        assert!(shown.exists());
        // Down to nothing: all but the one on screen go, and it says it held one back.
        let out = trim(r, 0, &guard(&[]));
        assert!(out.held_back && shown.exists());
        assert_eq!(scan(r).pictures.len(), 1);
    }

    /// What waits to be uploaded, a picture a reader was just handed, and a download still
    /// landing are never let go of — not by a trim, not by Clear Cache.
    #[test]
    fn pending_held_and_landing_files_stay() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let pending = put(r, "open/X/IMG_1.MOV", 300, mins(900));
        let read_now = put(r, "original/R", 300, mins(900));
        let landing = put(r, "original/L.1a2b.part", 300, Duration::from_secs(2));
        let plain = put(r, "original/Q", 300, mins(900));
        let protected = identities(std::slice::from_ref(&pending));
        assert!(hold_if_cached(&read_now));
        assert!(!hold_if_cached(&r.join("original/NOT-THERE")));
        let out = clear(r, &guard(&protected));
        assert!(pending.exists() && read_now.exists() && landing.exists());
        assert!(!plain.exists());
        assert!(out.held_back && out.removed == 1);
        // The `.part` is never taken for a picture: not counted, never a cache hit.
        assert_eq!(scan(r).pictures.len(), 2);
    }

    /// A pending upload that is a hard link to a cached original keeps the original too: the
    /// two are one file, and letting one name go would free nothing.
    #[cfg(unix)]
    #[test]
    fn a_link_and_its_original_are_one_picture() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let original = put(r, "original/V", 500, mins(900));
        let link = r.join("open/V/clip.MOV");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::fs::hard_link(&original, &link).unwrap();
        let s = scan(r);
        assert_eq!(s.pictures.len(), 1, "one file under two names");
        assert!(s.used < 600 * 1024, "counted once: {}", s.used);
        // Protected by either name.
        let protected = identities(std::slice::from_ref(&link));
        assert_eq!(clear(r, &guard(&protected)).removed, 0);
        assert!(original.exists() && link.exists());
        // Unprotected, both names go, the player's folder with them.
        assert_eq!(clear(r, &guard(&[])).removed, 1);
        assert!(!original.exists() && !link.exists() && !r.join("open/V").exists());
    }

    /// A crash at any point of a trim leaves every picture whole or gone — never half — and the
    /// next start finishes the job; what the disk holds is all there is to agree with.
    #[test]
    fn a_crash_mid_trim_leaves_everything_whole_and_the_next_start_finishes() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        for i in 0..20 {
            put(r, &format!("original/P{i:02}"), 50, mins(200 - i));
            put(r, &format!("full/P{i:02}"), 50, mins(200 - i));
        }
        let cap = 10 * 52 * 1024;
        for stop in [0, 1, 7, 19] {
            let out = trim_with(r, cap, &guard(&[]), Some(stop));
            assert!(out.removed <= stop);
            all_whole(r);
            // What is counted is exactly what is on disk.
            assert_eq!(scan(r).used, out.after);
        }
        let out = trim(r, cap, &guard(&[]));
        assert!(out.after <= cap && out.after == scan(r).used);
        all_whole(r);
    }

    /// A download that died with the app leaves a `.part`: swept once clearly abandoned, never
    /// while one may still be landing, and never counted as the picture.
    #[test]
    fn abandoned_parts_are_swept() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let dead = put(r, "original/A.9f.part", 200, Duration::from_secs(2 * 3600));
        let live = put(r, "video/B.3c.part", 200, Duration::from_secs(30));
        let out = trim(r, 10 * GB, &guard(&[]));
        assert_eq!(out.before, 0, "parts are not pictures");
        assert!(!dead.exists() && live.exists());
    }

    /// Settings offers fixed sizes; anything else is taken as the nearest, and a saved value
    /// that is not one of them (or not a number) is the default.
    #[test]
    fn caps_are_the_sizes_offered() {
        assert_eq!(snap(3 * GB), 2 * GB);
        assert_eq!(snap(0), 512 * MB);
        assert_eq!(snap(u64::MAX), 50 * GB);
        assert_eq!(parse_cap(None), DEFAULT_CAP);
        assert_eq!(parse_cap(Some("banana".into())), DEFAULT_CAP);
        assert_eq!(parse_cap(Some("12345".into())), DEFAULT_CAP);
        assert_eq!(parse_cap(Some((5 * GB).to_string())), 5 * GB);
        // A fifth of the room it could have, when the disk is short of it.
        assert_eq!(limit(10 * GB, 100 * GB, 0), 10 * GB);
        assert_eq!(limit(10 * GB, 4 * GB, GB), GB);
    }

    /// A trim on a copy of a real cache (never the real one): `QUINCE_CACHE_COPY=<copy of
    /// photos/> QUINCE_CACHE_CAP=<bytes> cargo test -p quince-app on_a_copy -- --ignored`.
    #[test]
    #[ignore = "needs a copy of a real cache"]
    fn on_a_copy() {
        let root = PathBuf::from(std::env::var("QUINCE_CACHE_COPY").unwrap());
        let cap: u64 = std::env::var("QUINCE_CACHE_CAP").unwrap().parse().unwrap();
        let thumbs = thumbnails(&root);
        let t = Instant::now();
        let out = trim(&root, cap, &guard(&[]));
        println!(
            "{} pictures let go, {} → {} bytes against {cap}, in {:?}; thumbnails {} → {}",
            out.removed,
            out.before,
            out.after,
            t.elapsed(),
            thumbs,
            thumbnails(&root)
        );
        assert_eq!(thumbnails(&root), thumbs);
        assert!(out.after <= cap || out.held_back);
        assert_eq!(scan(&root).used, out.after);
    }

    /// Drive's Keep on This Computer lives in its own cache beside Photos': never walked here.
    #[test]
    fn other_caches_are_never_touched() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path();
        let kept = put(data, "drive-cache/FILE__x", 300, mins(900));
        let kept_list = put(data, "drive-cache/.kept.json", 1, mins(900));
        let waiting = put(
            data,
            "photos-folder-writes/outgoing/h/IMG.HEIC",
            300,
            mins(900),
        );
        let loose = put(data, "photos/LEGACY", 300, mins(900));
        put(data, "photos/original/A", 300, mins(900));
        clear(&data.join("photos"), &guard(&[]));
        for f in [kept, kept_list, waiting, loose] {
            assert!(f.exists(), "{f:?}");
        }
        assert!(!data.join("photos/original/A").exists());
    }
}
