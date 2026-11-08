//! A folder from iCloud Drive — or a selection of files and folders — saved into Downloads as
//! the same tree of folders and files it is in Drive, at any size: a terabyte, a hundred
//! thousand files, a tree hundreds of folders deep.
//!
//! - **Where it goes.** Every name in the tree is whoever shared the folder's to choose, so each
//!   is made one plain name before it touches a path ([`crate::export::file_name`]: no `..`, no
//!   separator, no drive, no stream, no Windows device name, no trailing dot or space), and the
//!   path it makes is checked to sit directly in the folder it was made for
//!   ([`crate::saving::directly_in`]). Nothing is written outside the folder made for the
//!   download in Downloads — or, for a file chosen on its own, anywhere but Downloads itself.
//! - **Names that clash.** Nothing that was in Downloads before is written over or into: a
//!   folder is made new and numbered ("Photos 2"), as Finder numbers one. Inside, two names
//!   that come out the same once cleaned, that differ only in case, or only in how an accent is
//!   encoded (a Mac's decomposed "é" beside a composed one), are numbered too — Windows, macOS
//!   and an exFAT drive would take them for one file. Names are saved composed (NFC).
//! - **Links.** None is followed: a name taken by anything at all, a link included, counts as
//!   taken, and every folder between the download's own and a file is checked to be a real
//!   folder before the file is written into it.
//! - **Two passes, bounded memory.** The tree is read once to count it — files, bytes, and on a
//!   resumed download what is already here — keeping only the folders still to read, so the
//!   room it needs is known before a byte is written. Then it is read again and fetched as it
//!   is read: the reading stays no more than [`AHEAD`] files ahead of the fetching, four files
//!   and four folders at a time. A folder of a hundred thousand files takes the memory of a few
//!   hundred. (`retrieveItemDetailsInFolders` answers with a folder whole: there is no page to
//!   ask for next.)
//! - **Whole files only.** Each file goes into a partial file of its own and is moved into
//!   place once every byte Apple promised is there (`quince_icloud::drive`), Windows' virus
//!   scanner waited out (`quince_core::files`). A dropped connection or a busy Drive is asked
//!   again ([`quince_icloud::drive::patiently`]).
//! - **Resumable.** What was chosen and where it went is kept in a small journal ([`Journal`])
//!   until the download finishes cleanly. Downloading the same thing again — after Cancel, a
//!   lost connection, a full disk, or a restart — goes on in the same folder: a file already
//!   there at iCloud's size is kept, any other is fetched again in its place, and partial files
//!   left by a crash are cleared.
//! - **When it can't.** A fresh sign-in needed, a disk that is full or would be (checked before
//!   it starts and as it goes), or the connection lost for good, stops the lot and says why —
//!   resumable. A file that cannot be saved — its path too long for this computer, say — is
//!   named with the reason, and the rest go on.
//! - **Stopping.** Cancel stops everything at once: what is arriving is dropped with its partial
//!   file, what has arrived stays, and a folder made that is left empty goes again.
//!
//! The window hears how far it has got (`DOWNLOAD`): a count while the tree is read, then each
//! file as it lands — and, for a file of a megabyte or more, its bytes as they arrive.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::future::{AbortHandle, Abortable};
use futures_util::StreamExt;
use quince_core::drive::DriveItem;
use quince_icloud::drive::Placing;
use quince_icloud::Error;

/// Where a download's progress is announced.
pub(crate) const DOWNLOAD: &str = "quince://download";

/// How many folders are read, and how many files fetched, at once.
pub(crate) const AT_ONCE: usize = 4;

/// How many files found may wait for their turn: how far the reading gets ahead of the fetching.
pub(crate) const AHEAD: usize = 256;

/// Folders deeper than this are not gone into: far past any real tree, and a guard against a
/// listing that names a folder inside itself under a new id.
pub(crate) const DEEPEST: usize = 256;

/// How many failures are named one by one; past this they are counted.
const NAMED_FAILURES: usize = 200;

/// Files this big or bigger tell their bytes as they arrive; smaller ones only that they landed.
const LARGE: u64 = 1 << 20;

/// How often the count of what has been found is told while the tree is read.
const FOUND_EVERY: Duration = Duration::from_millis(100);

/// How often the disk's free space is looked at again while files arrive, in the app.
pub(crate) const ROOM_EVERY: Duration = Duration::from_secs(2);

/// How many files in a row may fail on a dropped connection before the connection counts as
/// lost and the download pauses, rather than marking every file after them as failed.
const LOST_AFTER: usize = 8;

/// One step of a download, as the window hears it — the same fields as an upload's.
#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadEvent {
    /// Unique within the batch the window started.
    pub id: String,
    pub batch: String,
    pub name: String,
    /// The folders it sits in, inside Downloads, as they were saved.
    pub within: String,
    pub sent: u64,
    pub total: u64,
    /// For a file: `downloading`, `done`, `failed` or `cancelled`. For the batch: `found` while
    /// the tree is read and `planned` once all of it is known — their `sent` the files, their
    /// `total` the bytes.
    pub state: &'static str,
    /// A folder rather than a file: only ever one that could not be read or made.
    pub folder: bool,
    pub error: Option<String>,
}

/// What a download came to.
#[derive(serde::Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DownloadReport {
    /// What it is known by in the journal (see [`Journal::key`]).
    pub key: String,
    /// What is now in Downloads: one path for each thing chosen that is there, whole or in part.
    pub saved: Vec<String>,
    /// What did not arrive, one line each: where it would be inside Downloads, and why.
    pub failed: Vec<String>,
    /// Failures past those named.
    pub more_failed: usize,
    /// Stopped with Cancel.
    pub cancelled: bool,
    /// Why it stopped short on its own: the disk full or about to be, the connection lost.
    pub problem: Option<String>,
    /// It can be gone on with: downloading the same again resumes it.
    pub resumable: bool,
    /// The most files that were found and waiting at once: what the reading's lead cost.
    #[serde(skip)]
    pub peak_waiting: usize,
}

/// How events reach the window.
pub(crate) type Emit = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

/// Drive, as one download asks it, and the computer it saves to.
pub(crate) struct Drive {
    pub http: reqwest::Client,
    /// `drivews`: the listings.
    pub drivews: String,
    /// `docws`: the bytes.
    pub docws: String,
    pub params: Vec<(String, String)>,
    /// The first wait before a request that met a busy Drive is made again.
    pub retry_after: Duration,
    /// How much room a folder's disk has (see [`free_space`]).
    pub free_space: fn(&Path) -> Option<u64>,
    /// How often that is looked at again while files arrive ([`ROOM_EVERY`] in the app).
    pub room_every: Duration,
}

/// One download asked for.
pub(crate) struct Request<'a> {
    pub batch: &'a str,
    /// What the window calls it — a folder's name, "3 items" — kept for resuming it later.
    pub title: &'a str,
    pub items: Vec<DriveItem>,
    /// Downloads.
    pub into: &'a Path,
    /// Where unfinished downloads are remembered; none, and nothing is resumable.
    pub journal: Option<&'a Journal>,
    /// A backup's (src-tauri/src/backup.rs), not a download's: the one item goes into this same
    /// folder every time, carried on in place, and a file changed in iCloud since is not written
    /// over: the copy here is moved into `earlier` first, under the date it was last changed.
    pub backup: Option<Backup<'a>>,
}

/// Where a backup keeps iCloud Drive: `top`, the folder its whole tree lives in, and `earlier`,
/// where a file replaced by a newer version goes, at the same place in the tree.
pub(crate) struct Backup<'a> {
    pub top: &'a Path,
    pub earlier: &'a Path,
}

// ── Stopping ────────────────────────────────────────────────────────────────────────────────

/// A download's Cancel. Pressed once, everything still running is dropped and nothing more
/// begins.
#[derive(Clone, Default)]
pub(crate) struct Stop(Arc<StopInner>);

#[derive(Default)]
struct StopInner {
    stopped: AtomicBool,
    /// What is running now, by a number of its own: let go of as each finishes, so a download
    /// of thousands of files keeps no more than the few running.
    running: Mutex<std::collections::HashMap<u64, AbortHandle>>,
    next: AtomicU64,
}

impl Stop {
    pub fn stop(&self) {
        self.0.stopped.store(true, Ordering::SeqCst);
        let running = std::mem::take(&mut *self.running());
        for handle in running.into_values() {
            handle.abort();
        }
    }

    pub fn stopped(&self) -> bool {
        self.0.stopped.load(Ordering::SeqCst)
    }

    fn running(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<u64, AbortHandle>> {
        self.0
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `work`, unless Cancel is pressed first or while it runs (`None` then). Registered under
    /// the lock `stop` takes, after the flag is read: work that starts as Cancel is pressed is
    /// either stopped by it or never starts.
    async fn guard<F: std::future::Future>(&self, work: F) -> Option<F::Output> {
        let (handle, registration) = AbortHandle::new_pair();
        let id = self.0.next.fetch_add(1, Ordering::Relaxed);
        {
            let mut running = self.running();
            if self.stopped() {
                return None;
            }
            running.insert(id, handle);
        }
        let outcome = Abortable::new(work, registration).await.ok();
        self.running().remove(&id);
        outcome
    }
}

// ── The journal ─────────────────────────────────────────────────────────────────────────────

/// Downloads not yet finished cleanly, one small file each in Skykeep's own data folder: what
/// was chosen, and where in Downloads each part of it went. Read when the same is downloaded
/// again, so it goes on where it was rather than starting over beside itself.
pub(crate) struct Journal {
    dir: PathBuf,
}

/// An unfinished download, as the journal keeps it.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    pub key: String,
    pub title: String,
    pub items: Vec<DriveItem>,
    pub placed: Vec<Placed>,
    pub started_at: i64,
}

/// Where one thing chosen went.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Placed {
    pub drivewsid: String,
    pub path: PathBuf,
    pub folder: bool,
}

impl Journal {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// What a choice of items is known by: the same items, in any order, are the same download.
    pub fn key(items: &[DriveItem]) -> String {
        use sha2::Digest;
        let mut ids: Vec<&str> = items.iter().map(|i| i.drivewsid.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        let digest = sha2::Sha256::digest(ids.join("\n").as_bytes());
        digest[..16].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The file a key is kept in — only ever a key's own shape, so no key names another path.
    fn file(&self, key: &str) -> Option<PathBuf> {
        (key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| self.dir.join(format!("{key}.json")))
    }

    pub fn load(&self, key: &str) -> Option<Record> {
        let text = std::fs::read(self.file(key)?).ok()?;
        serde_json::from_slice::<Record>(&text)
            .ok()
            .filter(|r| r.key == key)
    }

    /// Kept whole or not at all: written beside itself and moved into place.
    pub fn save(&self, record: &Record) -> std::io::Result<()> {
        let file = self
            .file(&record.key)
            .ok_or_else(|| std::io::Error::other("not a download's key"))?;
        std::fs::create_dir_all(&self.dir)?;
        let fresh = file.with_extension("json.new");
        std::fs::write(
            &fresh,
            serde_json::to_vec(record).map_err(std::io::Error::other)?,
        )?;
        quince_core::files::rename(&fresh, &file)
    }

    pub fn forget(&self, key: &str) {
        if let Some(file) = self.file(key) {
            let _ = std::fs::remove_file(file);
        }
    }

    /// Every download not yet finished, oldest first.
    pub fn unfinished(&self) -> Vec<Record> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut records: Vec<Record> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                self.load(name.strip_suffix(".json")?)
            })
            .collect();
        records.sort_by_key(|r| r.started_at);
        records
    }
}

// ── Names ───────────────────────────────────────────────────────────────────────────────────

/// `s` composed (NFC): "é" as one character, the way Linux and Windows programs write it, where
/// a Mac may have stored it as "e" and an accent.
fn composed(s: &str) -> String {
    icu_normalizer::ComposingNormalizerBorrowed::new_nfc()
        .normalize(s)
        .into_owned()
}

/// Names given out in one folder of the copy, compared composed and with case ignored — as
/// Windows, macOS and exFAT compare them.
#[derive(Default)]
pub(crate) struct Taken(HashSet<String>);

impl Taken {
    /// Where `given` is saved in `dir`, a folder that was here before the download: one plain
    /// name, numbered — "Photos 2", "report 2.pdf" — past anything at all on disk under it (a
    /// link too, even one leading nowhere) and past what this download has given out there.
    pub fn name(&mut self, dir: &Path, given: &str, folder: bool) -> std::io::Result<PathBuf> {
        self.pick(dir, given, folder, true)
    }

    /// Where `given` is saved in `dir`, a folder the download made itself: numbered past what
    /// this download has given out there only, so the same tree is named the same way every
    /// time it is read — what makes a download resumable.
    pub fn ours(&mut self, dir: &Path, given: &str, folder: bool) -> std::io::Result<PathBuf> {
        self.pick(dir, given, folder, false)
    }

    /// A name this download already holds in the folder (a resumed one's).
    pub fn hold(&mut self, path: &Path) {
        if let Some(name) = path.file_name() {
            self.0
                .insert(composed(&name.to_string_lossy().to_lowercase()));
        }
    }

    fn pick(
        &mut self,
        dir: &Path,
        given: &str,
        folder: bool,
        disk: bool,
    ) -> std::io::Result<PathBuf> {
        let clean =
            crate::export::file_name(&composed(given), if folder { "Folder" } else { "Download" });
        // A folder's name is all stem: "v1.2" numbered is "v1.2 2", not "v1 2.2".
        let (stem, ext) = if folder {
            (clean.as_str(), "")
        } else {
            crate::name_parts(&clean)
        };
        for n in 1..=10_000u32 {
            let name = if n == 1 {
                clean.clone()
            } else {
                format!("{stem} {n}{ext}")
            };
            let key = composed(&name.to_lowercase());
            if self.0.contains(&key) {
                continue;
            }
            let path = dir.join(&name);
            if !crate::saving::directly_in(dir, &path) {
                return Err(std::io::Error::other(
                    "that name can\u{2019}t be saved here",
                ));
            }
            if disk && path.symlink_metadata().is_ok() {
                continue;
            }
            self.0.insert(key);
            return Ok(path);
        }
        Err(std::io::Error::other(
            "too many things of that name are already there",
        ))
    }
}

/// A new folder for `given` in `dir`, numbered past anything already there, and made — never
/// one that was there before, so nothing of anyone else's is written into.
pub(crate) fn make_folder(taken: &mut Taken, dir: &Path, given: &str) -> std::io::Result<PathBuf> {
    loop {
        let path = taken.name(dir, given, true)?;
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            // Taken between the look and the making: the next number.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
}

/// The folder for `given` inside one the download made: made, or — resumed — the one made for
/// it before. Whether it was made now.
fn ensure_folder(taken: &mut Taken, dir: &Path, given: &str) -> std::io::Result<(PathBuf, bool)> {
    let path = taken.ours(dir, given, true)?;
    match std::fs::create_dir(&path) {
        Ok(()) => Ok((path, true)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if is_real_dir(&path) {
                Ok((path, false))
            } else {
                Err(std::io::Error::other(
                    "something else is already where it would go",
                ))
            }
        }
        Err(e) => Err(e),
    }
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// Whether every folder from `top` down to `dir` is a real folder, not a link: what this
/// download made, still as it made it.
pub(crate) fn real_folders(top: &Path, dir: &Path) -> bool {
    let Ok(below) = dir.strip_prefix(top) else {
        return false;
    };
    let mut at = top.to_path_buf();
    if !is_real_dir(&at) {
        return false;
    }
    for part in below.components() {
        let std::path::Component::Normal(name) = part else {
            return false;
        };
        at.push(name);
        if !is_real_dir(&at) {
            return false;
        }
    }
    true
}

/// The partial files a download of `path` may have left — `name.part`, `name.part.1` … — as
/// `quince_icloud::drive` names them.
fn partials(path: &Path) -> impl Iterator<Item = PathBuf> {
    let base = path.with_extension(match path.extension() {
        Some(ext) => format!("{}.part", ext.to_string_lossy()),
        None => "part".into(),
    });
    (0..=100).map(move |n| {
        if n == 0 {
            base.clone()
        } else {
            let mut name = base.as_os_str().to_owned();
            name.push(format!(".{n}"));
            PathBuf::from(name)
        }
    })
}

// ── The disk ────────────────────────────────────────────────────────────────────────────────

/// The error codes that mean the disk (or the person's share of it) is full.
#[cfg(unix)]
const NO_ROOM: &[i32] = &[libc::ENOSPC, libc::EDQUOT];
/// ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL.
#[cfg(windows)]
const NO_ROOM: &[i32] = &[39, 112];
#[cfg(not(any(unix, windows)))]
const NO_ROOM: &[i32] = &[];

/// The error codes that mean a path is longer than this computer takes.
#[cfg(unix)]
const TOO_LONG: &[i32] = &[libc::ENAMETOOLONG];
/// ERROR_FILENAME_EXCED_RANGE.
#[cfg(windows)]
const TOO_LONG: &[i32] = &[206];
#[cfg(not(any(unix, windows)))]
const TOO_LONG: &[i32] = &[];

fn no_room(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::StorageFull
        || e.kind() == std::io::ErrorKind::QuotaExceeded
        || e.raw_os_error().is_some_and(|c| NO_ROOM.contains(&c))
}

/// What went wrong writing to the disk, in words a person can act on.
fn disk_words(e: &std::io::Error) -> String {
    if no_room(e) {
        "there isn\u{2019}t enough space left on the disk".into()
    } else if e.raw_os_error().is_some_and(|c| TOO_LONG.contains(&c)) {
        "its path is too long for this computer".into()
    } else if e.kind() == std::io::ErrorKind::PermissionDenied {
        "Skykeep isn\u{2019}t allowed to write there".into()
    } else {
        e.to_string()
    }
}

/// How many bytes can still be written to the disk `dir` is on, by this user.
pub(crate) fn free_space(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: a zeroed statvfs is a valid value to be written over; `c` is a NUL-ended
        // path that outlives the call.
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        #[allow(clippy::useless_conversion)]
        Some(u64::from(s.f_bavail).saturating_mul(u64::from(s.f_frsize)))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        // SAFETY: `wide` is a NUL-ended path that outlives the call; Windows writes one u64.
        unsafe {
            windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                windows::core::PCWSTR(wide.as_ptr()),
                Some(&mut free),
                None,
                None,
            )
        }
        .ok()?;
        Some(free)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = dir;
        None
    }
}

/// A size as a person reads it: "3.2 GB".
fn size_words(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

// ── The download ────────────────────────────────────────────────────────────────────────────

/// `within` and a name below it, as a path is shown: `Photos/Trip`.
fn below(within: &str, name: &str) -> String {
    if within.is_empty() {
        name.to_string()
    } else {
        format!("{within}/{name}")
    }
}

fn shown(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn size_of(item: &DriveItem) -> Option<u64> {
    item.size.and_then(|n| u64::try_from(n).ok())
}

/// A folder of Drive's to read, and where it is saved.
struct Folder {
    item: DriveItem,
    dir: PathBuf,
    /// Where it is inside Downloads, as saved: `Photos/Trip`.
    within: String,
    /// Which of the things chosen it is in.
    top: usize,
    depth: usize,
    /// Already on disk, from an earlier run: what is in it is looked at while counting.
    on_disk: bool,
}

/// A file to fetch, and where it goes.
struct Fetch {
    item: DriveItem,
    path: PathBuf,
    within: String,
    /// The folder this download made that it goes into, checked down to the file's own before
    /// it is written; `None` for a file chosen on its own, which goes into Downloads itself.
    top_dir: Option<PathBuf>,
    top: usize,
    placing: Placing,
    id: u64,
}

/// One thing chosen, and where in Downloads it goes.
struct Top {
    item: DriveItem,
    path: Option<PathBuf>,
    /// From an earlier run of this download.
    resumed: bool,
    arrived: AtomicUsize,
}

/// Which reading of the tree this is.
#[derive(Clone, Copy)]
enum Pass<'a> {
    /// Counting it: what there is, and what of it is already here. Nothing is written.
    Count,
    /// Saving it: folders made, files handed on to be fetched.
    Save(&'a tokio::sync::mpsc::Sender<Fetch>),
}

/// Everything a running download shares.
struct Run<'a> {
    drive: &'a Drive,
    batch: &'a str,
    into: &'a Path,
    /// A backup's earlier versions, when this run is a backup's (`Request::backup`).
    earlier: Option<PathBuf>,
    emit: Emit,
    stop: &'a Stop,
    tops: Vec<Top>,
    failed: Mutex<(Vec<String>, usize)>,
    problem: Mutex<Option<String>>,
    /// Set once Apple asks for a fresh sign-in: nothing more can arrive.
    reauth: AtomicBool,
    folders_failed: AtomicU64,
    /// Files and bytes found by the reading under way.
    found: AtomicU64,
    found_bytes: AtomicU64,
    /// Counting: the bytes of files already here, whole, from an earlier run.
    have_bytes: AtomicU64,
    /// Saving: the bytes still to come when it began, and those that have come since.
    needed: AtomicU64,
    arrived_bytes: AtomicU64,
    last_found: Mutex<Instant>,
    last_room: Mutex<Instant>,
    /// Files in a row that failed on a dropped connection.
    lost: AtomicUsize,
    next_id: AtomicU64,
    waiting: AtomicUsize,
    peak_waiting: AtomicUsize,
}

impl Run<'_> {
    fn note_failure(&self, line: String) {
        let mut failed = self.failed.lock().unwrap_or_else(PoisonError::into_inner);
        if failed.0.len() < NAMED_FAILURES {
            failed.0.push(line);
        } else {
            failed.1 += 1;
        }
    }

    fn event(&self, id: String, name: &str, within: &str, state: &'static str) -> DownloadEvent {
        DownloadEvent {
            id,
            batch: self.batch.to_string(),
            name: name.to_string(),
            within: within.to_string(),
            sent: 0,
            total: 0,
            state,
            folder: false,
            error: None,
        }
    }

    fn fail_folder(&self, name: &str, within: &str, error: String) {
        let n = self.folders_failed.fetch_add(1, Ordering::Relaxed);
        let mut e = self.event(format!("{}:x:{n}", self.batch), name, within, "failed");
        e.folder = true;
        e.error = Some(error.clone());
        (self.emit)(e);
        self.note_failure(format!("{}: {error}", below(within, name)));
    }

    /// Stopped short on its own, for a reason said once: everything else stops.
    fn give_up(&self, why: String) {
        self.problem
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert(why);
        self.stop.stop();
    }

    /// Apple wants a fresh sign-in: nothing more can arrive.
    fn reauth(&self) {
        self.reauth.store(true, Ordering::SeqCst);
        self.stop.stop();
    }

    /// A disk error placing something: said in words, and the lot stopped when the disk is full.
    fn disk_error(&self, e: &std::io::Error) -> String {
        if no_room(e) {
            self.give_up(
                "Downloads ran out of space, so the download stopped. What arrived is kept; nothing half-saved was."
                    .into(),
            );
        }
        disk_words(e)
    }

    /// The tally of what the reading has found, told at most every FOUND_EVERY (and at once
    /// when `now`).
    fn tell_found(&self, now: bool) {
        let mut last = self
            .last_found
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !now && last.elapsed() < FOUND_EVERY {
            return;
        }
        *last = Instant::now();
        let mut e = self.event(format!("{}:found", self.batch), "", "", "found");
        e.folder = true;
        e.sent = self.found.load(Ordering::Relaxed);
        e.total = self.found_bytes.load(Ordering::Relaxed);
        (self.emit)(e);
    }

    /// Whether there is still room for what is still to come, looked at every `room_every`: other
    /// programs write to the same disk while a long download runs.
    fn check_room(&self) {
        {
            let mut last = self
                .last_room
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if last.elapsed() < self.drive.room_every {
                return;
            }
            *last = Instant::now();
        }
        let still = self
            .needed
            .load(Ordering::Relaxed)
            .saturating_sub(self.arrived_bytes.load(Ordering::Relaxed));
        if let Some(free) = (self.drive.free_space)(self.into).filter(|free| *free < still) {
            self.give_up(format!(
                "Downloads is running out of space, so the download paused: about {} more is needed and {} is free. What arrived is kept.",
                size_words(still),
                size_words(free),
            ));
        }
    }

    async fn list(&self, folder: &DriveItem) -> Option<quince_icloud::Result<Vec<DriveItem>>> {
        let d = self.drive;
        let mut op = || {
            quince_icloud::drive::fetch_folder(
                &d.http,
                &d.drivews,
                d.params.clone(),
                &folder.drivewsid,
            )
        };
        self.stop
            .guard(quince_icloud::drive::patiently_after(
                d.retry_after,
                &mut op,
            ))
            .await
    }

    /// Read the tree from `frontier` down, four folders at a time, keeping only the folders
    /// still to read. Counting, it tallies; saving, it makes folders and hands each file on to
    /// be fetched as it is found. The folders it made, in the order made.
    async fn walk(&self, mut frontier: VecDeque<Folder>, pass: Pass<'_>) -> Vec<PathBuf> {
        let mut made = Vec::new();
        // Folders already gone into: a listing that named one again is not followed round.
        let mut seen: HashSet<String> = frontier.iter().map(|f| f.item.drivewsid.clone()).collect();
        while !frontier.is_empty() && !self.stop.stopped() {
            let now: Vec<Folder> = frontier.drain(..frontier.len().min(AT_ONCE)).collect();
            // Answered in order: the same tree is named the same way every time it is read.
            let listed = futures_util::future::join_all(now.into_iter().map(|folder| async move {
                let listing = self.list(&folder.item).await;
                (folder, listing)
            }))
            .await;
            for (folder, listing) in listed {
                let mut children = match listing {
                    None => return made,
                    Some(Ok(children)) => children,
                    Some(Err(Error::ReauthRequired)) => {
                        self.reauth();
                        return made;
                    }
                    // Counting, a folder that cannot be read is passed over: the saving pass
                    // asks again, and says so if it still cannot.
                    Some(Err(_)) if matches!(pass, Pass::Count) => continue,
                    Some(Err(e)) => {
                        // Not left behind empty, as if that were what it held.
                        let _ = std::fs::remove_dir(&folder.dir);
                        let parent = folder
                            .within
                            .rsplit_once('/')
                            .map_or(String::new(), |(p, _)| p.to_string());
                        self.fail_folder(&shown(&folder.dir), &parent, e.to_string());
                        continue;
                    }
                };
                // In name order, as Finder lists them: which of two clashing names is numbered
                // is then the same every time.
                children.sort_by_cached_key(|c| {
                    (
                        composed(&c.display_name().to_lowercase()),
                        c.drivewsid.clone(),
                    )
                });
                let mut here = Taken::default();
                for child in children {
                    if self.stop.stopped() {
                        return made;
                    }
                    let name = child.display_name();
                    if child.is_container() {
                        if !seen.insert(child.drivewsid.clone()) {
                            continue;
                        }
                        if folder.depth + 1 >= DEEPEST {
                            if let Pass::Save(_) = pass {
                                self.fail_folder(
                                    &name,
                                    &folder.within,
                                    "it is too many folders deep to save".into(),
                                );
                            }
                            continue;
                        }
                        let placed = match pass {
                            Pass::Count => here.ours(&folder.dir, &name, true).map(|dir| {
                                let on_disk = folder.on_disk && is_real_dir(&dir);
                                (dir, on_disk, false)
                            }),
                            Pass::Save(_) => ensure_folder(&mut here, &folder.dir, &name)
                                .map(|(dir, fresh)| (dir, !fresh, fresh)),
                        };
                        match placed {
                            Ok((dir, on_disk, fresh)) => {
                                if fresh {
                                    made.push(dir.clone());
                                }
                                frontier.push_back(Folder {
                                    within: below(&folder.within, &shown(&dir)),
                                    item: child,
                                    dir,
                                    top: folder.top,
                                    depth: folder.depth + 1,
                                    on_disk,
                                });
                            }
                            Err(_) if matches!(pass, Pass::Count) => {}
                            Err(e) => {
                                let words = self.disk_error(&e);
                                self.fail_folder(&name, &folder.within, words);
                            }
                        }
                    } else {
                        let path = match here.ours(&folder.dir, &name, false) {
                            Ok(path) => path,
                            Err(_) if matches!(pass, Pass::Count) => continue,
                            Err(e) => {
                                self.note_failure(format!(
                                    "{}: {}",
                                    below(&folder.within, &name),
                                    disk_words(&e)
                                ));
                                continue;
                            }
                        };
                        if !self.found_file(&child, &path, folder.on_disk, pass) {
                            return made;
                        }
                        if let Pass::Save(files) = pass {
                            let job = Fetch {
                                item: child,
                                path,
                                within: folder.within.clone(),
                                top_dir: self.tops[folder.top].path.clone(),
                                top: folder.top,
                                placing: Placing::Replace,
                                id: self.next_id.fetch_add(1, Ordering::Relaxed),
                            };
                            if !self.hand_on(files, job).await {
                                return made;
                            }
                        }
                    }
                }
                if matches!(pass, Pass::Count) {
                    self.tell_found(false);
                }
            }
        }
        made
    }

    /// A file found: counted, and — counting a resumed download — what of it is here already.
    /// False once there is nothing more to do.
    fn found_file(&self, item: &DriveItem, path: &Path, on_disk: bool, pass: Pass<'_>) -> bool {
        let size = size_of(item).unwrap_or(0);
        self.found.fetch_add(1, Ordering::Relaxed);
        self.found_bytes.fetch_add(size, Ordering::Relaxed);
        if matches!(pass, Pass::Count) && on_disk && whole_here(path, item) {
            self.have_bytes.fetch_add(size, Ordering::Relaxed);
        }
        !self.stop.stopped()
    }

    /// Hand a file on to be fetched — waiting while AHEAD are waiting already.
    async fn hand_on(&self, files: &tokio::sync::mpsc::Sender<Fetch>, job: Fetch) -> bool {
        let now = self.waiting.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak_waiting.fetch_max(now, Ordering::Relaxed);
        files.send(job).await.is_ok()
    }

    /// Fetch one file.
    async fn fetch(&self, job: Fetch) {
        self.fetch_one(&job).await;
        self.waiting.fetch_sub(1, Ordering::Relaxed);
    }

    async fn fetch_one(&self, job: &Fetch) {
        if self.stop.stopped() {
            return;
        }
        let name = shown(&job.path);
        let id = format!("{}:{}", self.batch, job.id);
        let size = size_of(&job.item);
        let done = |sent: u64| {
            let mut e = self.event(id.clone(), &name, &job.within, "done");
            e.sent = sent;
            e.total = sent.max(size.unwrap_or(0));
            (self.emit)(e);
            self.tops[job.top].arrived.fetch_add(1, Ordering::Relaxed);
        };
        let fail = |words: String| {
            let mut e = self.event(id.clone(), &name, &job.within, "failed");
            e.total = size.unwrap_or(0);
            e.error = Some(words.clone());
            (self.emit)(e);
            self.note_failure(format!("{}: {words}", below(&job.within, &name)));
        };
        if job.placing == Placing::Replace {
            // Resumed: a file already here at iCloud's size is the one that arrived before.
            if whole_here(&job.path, &job.item) {
                done(size.unwrap_or(0));
                return;
            }
            if is_real_dir(&job.path) {
                fail("a folder is already where it would go".into());
                return;
            }
            // A backup keeps the version it had: moved into its earlier versions before the new
            // one comes. Not moved, it is never written over.
            if let Some(earlier) = &self.earlier {
                if let Err(e) = keep_earlier(self.into, earlier, &job.path) {
                    fail(format!("its earlier version couldn't be kept: {e}"));
                    return;
                }
            }
            // Partial files a crash left behind are cleared, so they cannot pile up.
            for partial in partials(&job.path) {
                if std::fs::symlink_metadata(&partial).is_ok_and(|m| m.is_file()) {
                    let _ = std::fs::remove_file(&partial);
                }
            }
        }
        let large = size.is_none_or(|s| s >= LARGE);
        let progress: Option<quince_icloud::drive::Progress> = large.then(|| {
            let mut start = self.event(id.clone(), &name, &job.within, "downloading");
            start.total = size.unwrap_or(0);
            (self.emit)(start.clone());
            // Told about every chunk, passed on at every whole percent: a smooth bar without a
            // flood of messages for a large file.
            let (emit, last) = (self.emit.clone(), AtomicU64::new(0));
            let progress: quince_icloud::drive::Progress =
                Arc::new(move |sent: u64, total: u64| {
                    let step = (total / 100).max(1);
                    let before = last.load(Ordering::Relaxed);
                    if sent / step != before / step || sent == total {
                        last.store(sent, Ordering::Relaxed);
                        let mut e = start.clone();
                        e.sent = sent;
                        e.total = total.max(e.total);
                        emit(e);
                    }
                });
            progress
        });
        let stopped = || {
            if large {
                let mut e = self.event(id.clone(), &name, &job.within, "cancelled");
                e.total = size.unwrap_or(0);
                (self.emit)(e);
            }
        };
        let d = self.drive;
        let (item, path, top_dir, placing) =
            (&job.item, &job.path, job.top_dir.as_deref(), job.placing);
        let mut op = || {
            let progress = progress.clone();
            async move {
                // Checked before every try: a folder swapped for a link meanwhile is not
                // written through.
                let unchanged = match (top_dir, path.parent()) {
                    (Some(top), Some(dir)) => real_folders(top, dir),
                    (None, _) => true,
                    (Some(_), None) => false,
                };
                if !unchanged {
                    return Err(Error::Refused(
                        "the folder it was going into has changed, so it wasn\u{2019}t saved"
                            .into(),
                    ));
                }
                quince_icloud::drive::download_with_progress(
                    &d.http,
                    &d.docws,
                    d.params.clone(),
                    item,
                    path,
                    progress,
                    placing,
                )
                .await
            }
        };
        let outcome = self
            .stop
            .guard(quince_icloud::drive::patiently_after(
                d.retry_after,
                &mut op,
            ))
            .await;
        match outcome {
            Some(Ok(())) => {
                crate::saving::mark_from_internet(path);
                let len = std::fs::metadata(path).map_or(size.unwrap_or(0), |m| m.len());
                self.arrived_bytes.fetch_add(len, Ordering::Relaxed);
                self.lost.store(0, Ordering::Relaxed);
                done(len);
                self.check_room();
            }
            Some(Err(Error::ReauthRequired)) => {
                self.reauth();
                stopped();
            }
            Some(Err(e)) => {
                if quince_icloud::drive::transient(&e)
                    && self.lost.fetch_add(1, Ordering::Relaxed) + 1 >= LOST_AFTER
                {
                    self.give_up(
                        "The connection to iCloud was lost, so the download paused. What arrived is kept; download it again to go on."
                            .into(),
                    );
                }
                let words = match &e {
                    Error::Io(io) => self.disk_error(io),
                    other => other.to_string(),
                };
                fail(words);
            }
            None => stopped(),
        }
    }
}

/// Whether the file at `path` is `item` whole and current: a plain file, at the size iCloud has
/// it, carrying the date it was last changed there (every download gives a file that date). By
/// size alone, a file edited in iCloud since — a fixed typo, the same length — was kept stale.
/// Dates are compared to within two seconds: FAT and exFAT keep no finer.
fn whole_here(path: &Path, item: &DriveItem) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    let same_size = size_of(item).is_some_and(|size| meta.is_file() && meta.len() == size);
    let same_version = match (quince_icloud::drive::changed_at(item), meta.modified()) {
        (Some(changed), Ok(on_disk)) => {
            let apart = changed
                .duration_since(on_disk)
                .unwrap_or_else(|e| e.duration());
            apart <= std::time::Duration::from_secs(2)
        }
        // Apple gave no date: the size is all there is to go by.
        (None, _) => true,
        (Some(_), Err(_)) => false,
    };
    same_size && same_version
}

/// A backup's copy of a file, about to be replaced by a newer version, moved to the same place
/// under `earlier` and named for when it was last changed ("Report (2026-11-08 15.10).docx"),
/// numbered when that name is taken. A plain file only; nothing there, nothing to keep.
fn keep_earlier(into: &Path, earlier: &Path, path: &Path) -> std::io::Result<()> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !meta.is_file() {
        return Ok(());
    }
    let rel = path.strip_prefix(into).unwrap_or(path);
    let dir = earlier.join(rel.parent().unwrap_or(Path::new("")));
    std::fs::create_dir_all(&dir)?;
    let when = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| jiff::Timestamp::from_second(d.as_secs() as i64).ok())
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H.%M")
                .to_string()
        })
        .unwrap_or_else(|| "earlier".into());
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    for n in 1.. {
        let candidate = if n == 1 {
            dir.join(format!("{stem} ({when}){ext}"))
        } else {
            dir.join(format!("{stem} ({when}) {n}{ext}"))
        };
        if std::fs::symlink_metadata(&candidate).is_err() {
            return std::fs::rename(path, candidate);
        }
    }
    unreachable!("a free name is always found")
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Save what was asked for from Drive into Downloads: each file chosen as a file there, each
/// folder as a folder there with everything in it — or, when the same was downloaded before
/// and not finished, on into what it saved then. Progress goes to `emit` as it happens; `stop`
/// cancels. Fails as a whole only when Apple wants a fresh sign-in; anything else that did not
/// arrive is in the report.
pub(crate) async fn run(
    drive: &Drive,
    request: Request<'_>,
    emit: Emit,
    stop: &Stop,
) -> Result<DownloadReport, String> {
    let Request {
        batch,
        title,
        items,
        into,
        journal,
        backup,
    } = request;
    if !into.is_dir() {
        return Err("Couldn\u{2019}t find your Downloads folder.".into());
    }
    let mut chosen = HashSet::new();
    let items: Vec<DriveItem> = items
        .into_iter()
        .filter(|i| chosen.insert(i.drivewsid.clone()))
        .collect();
    let key = Journal::key(&items);
    let before = journal.and_then(|j| j.load(&key));

    // Where an earlier run put each thing chosen, if it is still there as it left it — and in
    // Downloads: a journal edited to point elsewhere is not followed.
    let resumed_at = |item: &DriveItem| -> Option<PathBuf> {
        let placed = before
            .as_ref()?
            .placed
            .iter()
            .find(|p| p.drivewsid == item.drivewsid && p.folder == item.is_container())?;
        if !crate::saving::directly_in(into, &placed.path) {
            return None;
        }
        let fits = if placed.folder {
            is_real_dir(&placed.path)
        } else {
            std::fs::symlink_metadata(&placed.path).map_or(true, |m| m.is_file())
        };
        fits.then(|| placed.path.clone())
    };
    // A backup's one item always goes on in its own folder, made if it is not there yet.
    if let Some(b) = &backup {
        std::fs::create_dir_all(b.top).map_err(|e| disk_words(&e))?;
    }
    let tops: Vec<Top> = items
        .iter()
        .map(|item| {
            let path = match &backup {
                Some(b) => Some(b.top.to_path_buf()),
                None => resumed_at(item),
            };
            Top {
                item: item.clone(),
                resumed: path.is_some(),
                path,
                arrived: AtomicUsize::new(0),
            }
        })
        .collect();

    let mut run = Run {
        drive,
        batch,
        into,
        earlier: backup.as_ref().map(|b| b.earlier.to_path_buf()),
        emit: emit.clone(),
        stop,
        tops,
        failed: Mutex::new((Vec::new(), 0)),
        problem: Mutex::new(None),
        reauth: AtomicBool::new(false),
        folders_failed: AtomicU64::new(0),
        found: AtomicU64::new(0),
        found_bytes: AtomicU64::new(0),
        have_bytes: AtomicU64::new(0),
        needed: AtomicU64::new(0),
        arrived_bytes: AtomicU64::new(0),
        last_found: Mutex::new(Instant::now()),
        last_room: Mutex::new(Instant::now()),
        lost: AtomicUsize::new(0),
        next_id: AtomicU64::new(0),
        waiting: AtomicUsize::new(0),
        peak_waiting: AtomicUsize::new(0),
    };
    let report = |run: &Run<'_>, resumable: bool| {
        let (failed, more_failed) =
            std::mem::take(&mut *run.failed.lock().unwrap_or_else(PoisonError::into_inner));
        let problem = run
            .problem
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        DownloadReport {
            key: key.clone(),
            saved: run
                .tops
                .iter()
                .filter_map(|t| t.path.as_ref())
                .filter(|p| std::fs::symlink_metadata(p).is_ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            failed,
            more_failed,
            cancelled: stop.stopped() && problem.is_none() && !run.reauth.load(Ordering::SeqCst),
            problem,
            resumable,
            peak_waiting: run.peak_waiting.load(Ordering::Relaxed),
        }
    };

    // ── First, count it: nothing is written until it is known there is room. ──
    let mut frontier = VecDeque::new();
    for (top, t) in run.tops.iter().enumerate() {
        if t.item.is_container() {
            frontier.push_back(Folder {
                item: t.item.clone(),
                dir: t.path.clone().unwrap_or_default(),
                within: String::new(),
                top,
                depth: 0,
                on_disk: t.resumed,
            });
        } else if let Some(path) = &t.path {
            run.found_file(&t.item, path, true, Pass::Count);
        } else {
            run.found_file(&t.item, Path::new(""), false, Pass::Count);
        }
    }
    run.walk(frontier, Pass::Count).await;
    if run.reauth.load(Ordering::SeqCst) {
        return Err(Error::ReauthRequired.to_string());
    }
    run.tell_found(true);
    if stop.stopped() {
        return Ok(report(&run, before.is_some()));
    }
    let total_files = run.found.swap(0, Ordering::Relaxed);
    let total_bytes = run.found_bytes.swap(0, Ordering::Relaxed);
    let needed = total_bytes.saturating_sub(run.have_bytes.load(Ordering::Relaxed));
    run.needed.store(needed, Ordering::Relaxed);
    if let Some(free) = (drive.free_space)(into).filter(|free| needed > *free) {
        run.give_up(format!(
            "There isn\u{2019}t room in Downloads for this: it needs {} and {} is free.",
            size_words(needed),
            size_words(free),
        ));
        return Ok(report(&run, before.is_some()));
    }
    let planned = move |files: u64, bytes: u64| DownloadEvent {
        id: format!("{batch}:planned"),
        batch: batch.to_string(),
        name: String::new(),
        within: String::new(),
        sent: files,
        total: bytes,
        state: "planned",
        folder: true,
        error: None,
    };
    emit(planned(total_files, total_bytes));

    // ── Then place each thing chosen in Downloads, and remember where. ──
    let mut made = Vec::new();
    let mut in_downloads = Taken::default();
    for t in run.tops.iter().filter_map(|t| t.path.as_ref()) {
        in_downloads.hold(t);
    }
    let mut early = Vec::new();
    for t in run.tops.iter_mut().filter(|t| t.path.is_none()) {
        let name = t.item.display_name();
        let placed = if t.item.is_container() {
            make_folder(&mut in_downloads, into, &name).inspect(|p| made.push(p.clone()))
        } else {
            in_downloads.name(into, &name, false)
        };
        match placed {
            Ok(path) => t.path = Some(path),
            Err(e) => early.push((name, t.item.is_container(), e)),
        }
    }
    for (name, folder, e) in early {
        let words = run.disk_error(&e);
        if folder {
            run.fail_folder(&name, "", words);
        } else {
            run.note_failure(format!("{name}: {words}"));
        }
    }
    let placed: Vec<Placed> = run
        .tops
        .iter()
        .filter_map(|t| {
            Some(Placed {
                drivewsid: t.item.drivewsid.clone(),
                path: t.path.clone()?,
                folder: t.item.is_container(),
            })
        })
        .collect();
    let remembered = match journal {
        Some(j) if !placed.is_empty() => j
            .save(&Record {
                key: key.clone(),
                title: title.to_string(),
                items: items.clone(),
                placed,
                started_at: before.as_ref().map_or_else(now_seconds, |b| b.started_at),
            })
            .is_ok(),
        _ => false,
    };

    // ── Then save it: read again, and fetched as it is read. ──
    let (files, queue) = tokio::sync::mpsc::channel::<Fetch>(AHEAD);
    let run = &run;
    let reading = async move {
        let files = files;
        let mut frontier = VecDeque::new();
        for (top, t) in run.tops.iter().enumerate() {
            let Some(path) = t.path.clone() else { continue };
            if t.item.is_container() {
                frontier.push_back(Folder {
                    item: t.item.clone(),
                    within: shown(&path),
                    dir: path,
                    top,
                    depth: 0,
                    on_disk: true,
                });
            } else {
                run.found_file(&t.item, &path, false, Pass::Save(&files));
                let job = Fetch {
                    item: t.item.clone(),
                    path,
                    within: String::new(),
                    top_dir: None,
                    top,
                    placing: if t.resumed {
                        Placing::Replace
                    } else {
                        Placing::New
                    },
                    id: run.next_id.fetch_add(1, Ordering::Relaxed),
                };
                if !run.hand_on(&files, job).await {
                    return Vec::new();
                }
            }
        }
        let made = run.walk(frontier, Pass::Save(&files)).await;
        // All of it is known: the count it ends at, in case Drive changed since it was counted.
        if !stop.stopped() {
            emit(planned(
                run.found.load(Ordering::Relaxed),
                run.found_bytes.load(Ordering::Relaxed),
            ));
        }
        made
    };
    let fetching = futures_util::stream::unfold(queue, |mut queue| async move {
        queue.recv().await.map(|job| (job, queue))
    })
    .map(|job| run.fetch(job))
    .buffer_unordered(AT_ONCE)
    .for_each(|_| async {});
    let (below_made, ()) = futures_util::future::join(reading, fetching).await;
    made.extend(below_made);

    let stopped = stop.stopped();
    // Stopped part-way: the folders made now that hold nothing are taken away again, deepest
    // first. A folder only ever goes while empty (remove_dir takes nothing else), so a file
    // that arrived — or anything put there meanwhile — keeps its folder.
    if stopped {
        for dir in made.iter().rev() {
            if is_real_dir(dir) {
                let _ = std::fs::remove_dir(dir);
            }
        }
    }
    if run.reauth.load(Ordering::SeqCst) {
        return Err(Error::ReauthRequired.to_string());
    }
    let clean = !stopped && {
        let failed = run.failed.lock().unwrap_or_else(PoisonError::into_inner);
        failed.0.is_empty() && failed.1 == 0
    };
    let anything_there = run
        .tops
        .iter()
        .filter_map(|t| t.path.as_ref())
        .any(|p| std::fs::symlink_metadata(p).is_ok());
    let resumable = remembered && !clean && anything_there;
    if let Some(j) = journal.filter(|_| !resumable) {
        j.forget(&key);
    }
    Ok(report(run, resumable))
}

#[cfg(test)]
mod tests;
