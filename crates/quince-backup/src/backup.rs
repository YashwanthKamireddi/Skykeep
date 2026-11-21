//! Backup: every photo and video in iCloud Photos copied, as the original file, into a folder
//! the person chooses (an external drive, say), and kept up to date as new ones arrive. The
//! design and the research behind it: docs/backup.md (icloudpd's open problems designed out).
//!
//! - Complete: the list is the synced library (every asset, personal and Shared Library), never
//!   a live query paged while it shifts.
//! - Few requests: download links looked up 200 records at a time.
//! - Paced: three downloads at once; any refusal or lost connection ends the pass, and the next
//!   one starts two minutes later where this one stopped. Apple is never hammered.
//! - Safe: each file streamed to a `.part` and named only when whole; each photo recorded the
//!   moment it is, so a quit or crash repeats nothing and loses nothing. Never deletes: a photo
//!   deleted in iCloud stays in the backup; Recently Deleted is never copied.
//! - Light: no process of its own. A pass runs when the folder is set, at launch, and when
//!   Apple's push says Photos changed; idle, it costs nothing.
//!
//! Layout: `Photos/<year>/<month>/<file>` by the day it was taken, the file dated that day; a
//! Live Photo's video beside its still, under the same name.

use crate::fetch::stream_to_new_file;
use futures_util::future::BoxFuture;
use quince_core::backup::{Copied, Progress, LIVE, ORIGINAL, PHOTOS};
use quince_core::photos::{Kind, Photo};
use quince_sync::library_lock::LibraryLock;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const FOLDER_KEY: &str = "backup.folder";
const HIDDEN_KEY: &str = "backup.hidden";
/// iCloud Drive in the backup too: on unless turned off ("0").
const DRIVE_KEY: &str = "backup.drive";

/// Downloads at once: enough to keep a connection busy, few enough to stay polite.
const AT_ONCE: usize = 3;
/// Photos taken per round, looked up together: few enough that their download links (which
/// expire) are still good when the last of the round's videos starts.
const ROUND: usize = 50;
/// Tries for one photo whose download link Apple refused (expired), each with fresh links.
const TRIES: u8 = 2;
/// After a pass that had to stop (offline, Apple busy), the next one.
const AGAIN_AFTER: Duration = Duration::from_secs(120);

static RUNNING: AtomicBool = AtomicBool::new(false);
/// A change came in while a pass ran: run again once it ends.
static AGAIN: AtomicBool = AtomicBool::new(false);
/// The folder was checked against the record this run (once per launch and per folder).
static CHECKED: AtomicBool = AtomicBool::new(false);
/// Why the last pass stopped, if it did; and photos iCloud has no file for, left alone this run.
static PROBLEM: Mutex<Option<String>> = Mutex::new(None);
static SKIPPED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// iCloud Drive's tree is walked folder by folder, so it is gone through only when due: once at
/// launch, when Drive's change feed says something changed, and when backup is turned on or set
/// again. Photos' own pass, one query when nothing is new, runs after every Photos sync.
static DRIVE_DUE: AtomicBool = AtomicBool::new(true);
static DRIVE_RUNNING: AtomicBool = AtomicBool::new(false);
/// How iCloud Drive's last pass ended: the files it couldn't copy, if any.
static DRIVE_NOTE: Mutex<Option<String>> = Mutex::new(None);
/// The pass under way, stopped when backup is turned off.
static DRIVE_STOP: Mutex<Option<crate::folder_download::Stop>> = Mutex::new(None);
/// iCloud Drive's pass as it goes: files and bytes in the backup of all there are.
static DRIVE_NOW: Mutex<DriveCount> = Mutex::new(DriveCount::ZERO);
/// The last finished pass's count, kept across launches ("files,bytes").
const DRIVE_LAST_KEY: &str = "backup.drive.last";

#[derive(Clone, Copy, Default)]
struct DriveCount {
    files: u64,
    bytes: u64,
    files_total: u64,
    bytes_total: u64,
}

impl DriveCount {
    const ZERO: DriveCount = DriveCount {
        files: 0,
        bytes: 0,
        files_total: 0,
        bytes_total: 0,
    };
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub folder: Option<String>,
    pub hidden: bool,
    pub copied: i64,
    pub total: i64,
    pub running: bool,
    pub problem: Option<String>,
    /// Hidden is locked this run: including it asks for the unlock first (the window's sheet).
    pub hidden_locked: bool,
    /// iCloud Drive is in the backup too.
    pub drive: bool,
    /// iCloud Drive's pass is under way.
    pub drive_running: bool,
    /// What iCloud Drive's last pass couldn't copy, said in words.
    pub drive_note: Option<String>,
    /// Photos' bytes in the backup, and of all there are.
    pub photos_bytes: i64,
    pub photos_bytes_total: i64,
    /// iCloud Drive: files and bytes in the backup, of all there are (as it goes, or as the
    /// last pass finished).
    pub drive_files: u64,
    pub drive_files_total: u64,
    pub drive_bytes: u64,
    pub drive_bytes_total: u64,
}

struct Settings {
    folder: Option<PathBuf>,
    hidden: bool,
    drive: bool,
}

async fn settings<H: Host>(host: &H) -> Settings {
    // Read beside a sync pass, not in the writer's turn: Settings asks for the status while
    // a Photos store can hold the writer for seconds (desktop audit H1).
    let [folder, hidden, drive] = host
        .library()
        .read(|library| {
            Ok([FOLDER_KEY, HIDDEN_KEY, DRIVE_KEY].map(|key| library.state(key).ok().flatten()))
        })
        .await
        .unwrap_or_default();
    let folder = folder.filter(|f| !f.is_empty()).map(PathBuf::from);
    let hidden = hidden.as_deref() == Some("1");
    let drive = drive.as_deref() != Some("0");
    Settings {
        folder,
        hidden,
        drive,
    }
}

pub async fn status<H: Host>(host: &H) -> Status {
    let Settings {
        folder,
        hidden,
        drive,
    } = settings(host).await;
    let Progress {
        copied,
        total,
        copied_bytes,
        total_bytes,
    } = host
        .library()
        .read(|library| library.backup_progress(hidden))
        .await
        .unwrap_or_default();
    let drive_count = if DRIVE_RUNNING.load(Ordering::SeqCst) {
        DRIVE_NOW.lock().map(|c| *c).unwrap_or_default()
    } else {
        let last = host
            .library()
            .read(|library| library.state(DRIVE_LAST_KEY))
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
        let mut parts = last.split(',').map(|p| p.parse::<u64>().unwrap_or(0));
        let (files, bytes) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        DriveCount {
            files,
            bytes,
            files_total: files,
            bytes_total: bytes,
        }
    };
    Status {
        folder: folder.map(|f| f.display().to_string()),
        hidden,
        copied,
        total,
        running: RUNNING.load(Ordering::SeqCst),
        problem: PROBLEM.lock().ok().and_then(|p| p.clone()),
        hidden_locked: !host.hidden_open(),
        drive,
        drive_running: DRIVE_RUNNING.load(Ordering::SeqCst),
        drive_note: DRIVE_NOTE.lock().ok().and_then(|n| n.clone()),
        photos_bytes: copied_bytes,
        photos_bytes_total: total_bytes,
        drive_files: drive_count.files,
        drive_files_total: drive_count.files_total,
        drive_bytes: drive_count.bytes,
        drive_bytes_total: drive_count.bytes_total,
    }
}

/// A CloudKit or iCloud Drive service: the HTTP client, the service's address and the
/// session's query parameters.
pub type Service = (reqwest::Client, String, Vec<(String, String)>);

/// What a backup needs from the app it runs in: the desktop app (src-tauri) and the Mac app
/// (quince-mobile) each give it their own library, session and way of telling the window.
pub trait Host: Clone + Send + Sync + 'static {
    /// The account's library.
    fn library(&self) -> &LibraryLock;
    /// CloudKit (`ckdatabasews`), for Photos.
    fn cloudkit(&self) -> BoxFuture<'_, Result<Service, String>>;
    /// iCloud Drive (`drivews`), and its download service's address (`docws`).
    fn drive(&self) -> BoxFuture<'_, Result<(Service, String), String>>;
    /// The Hidden album is unlocked this run (the Apple ID password or Windows Hello).
    fn hidden_open(&self) -> bool;
    /// How the backup is going, for the window: about once a second while it runs, and once
    /// when a pass ends.
    fn tell(&self, status: Status);
    /// Runs `work` in the background, on the app's runtime.
    fn spawn(&self, work: BoxFuture<'static, ()>);
}

/// The backup folder, if one is set: for Show in Folder.
pub async fn folder<H: Host>(host: &H) -> Option<PathBuf> {
    settings(host).await.folder
}

/// Backup on into `folder` (chosen by the person, so theirs to write), or off (`None`); with
/// the Hidden album or without; with iCloud Drive or without. A new folder starts from
/// nothing: what is there is checked as it is copied, never assumed.
pub async fn set<H: Host>(
    host: &H,
    folder: Option<String>,
    hidden: bool,
    drive: bool,
) -> Result<Status, String> {
    {
        let mut library = host.library().lock().await;
        // Hidden copied into a plain folder would step around its lock: turned on only while
        // it is unlocked (the Apple ID password or Windows Hello, in Photos).
        let had_hidden = library.state(HIDDEN_KEY).ok().flatten().as_deref() == Some("1");
        if hidden && !had_hidden && !host.hidden_open() {
            return Err("Unlock the Hidden album in Photos first, then include it.".into());
        }
        let before = library.state(FOLDER_KEY).ok().flatten().unwrap_or_default();
        let after = folder.clone().unwrap_or_default();
        if before != after {
            library.backup_clear().map_err(|e| e.to_string())?;
            CHECKED.store(false, Ordering::SeqCst);
        }
        library
            .set_state(FOLDER_KEY, &after)
            .map_err(|e| e.to_string())?;
        library
            .set_state(HIDDEN_KEY, if hidden { "1" } else { "0" })
            .map_err(|e| e.to_string())?;
        library
            .set_state(DRIVE_KEY, if drive { "1" } else { "0" })
            .map_err(|e| e.to_string())?;
    }
    if let Ok(mut p) = PROBLEM.lock() {
        *p = None;
    }
    // Off, or iCloud Drive left out: a pass of it under way stops; on, it is gone through.
    if folder.is_none() || !drive {
        if let Some(stop) = DRIVE_STOP.lock().ok().and_then(|s| s.clone()) {
            stop.stop();
        }
    } else {
        DRIVE_DUE.store(true, Ordering::SeqCst);
    }
    kick(host);
    Ok(status(host).await)
}

/// iCloud Drive changed at Apple (its change feed): gone through again on the next pass. The
/// caller kicks a pass after.
pub fn drive_due() {
    DRIVE_DUE.store(true, Ordering::SeqCst);
}

/// Copy what isn't copied yet, if backup is on: at launch, when Photos changed, when turned on.
pub fn kick<H: Host>(host: &H) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    let host = host.clone();
    let work = host.clone();
    work.spawn(Box::pin(async move {
        let retry = loop {
            AGAIN.store(false, Ordering::SeqCst);
            let ended = pass(&host).await;
            if let Ok(mut p) = PROBLEM.lock() {
                *p = ended.clone().err();
            }
            if !AGAIN.load(Ordering::SeqCst) {
                break ended.is_err();
            }
        };
        RUNNING.store(false, Ordering::SeqCst);
        host.tell(status(&host).await);
        if retry {
            tokio::time::sleep(AGAIN_AFTER).await;
            kick(&host);
        }
    }));
}

/// One pass: Photos, then iCloud Drive when it is due, only into the backup's own folder.
async fn pass<H: Host>(host: &H) -> Result<(), String> {
    let Settings {
        folder: Some(folder),
        hidden,
        drive,
    } = settings(host).await
    else {
        return Ok(());
    };
    let recorded = {
        let library = host.library().lock().await;
        library.backup_files().map_err(|e| e.to_string())?
    };
    let at = folder.clone();
    tokio::task::spawn_blocking(move || ready(&at, &recorded))
        .await
        .map_err(|e| e.to_string())??;
    photos_pass(host, &folder, hidden).await?;
    if drive && DRIVE_DUE.swap(false, Ordering::SeqCst) {
        if let Err(e) = drive_pass(host, &folder).await {
            // Due again: the next pass (two minutes on) carries on where this one stopped.
            DRIVE_DUE.store(true, Ordering::SeqCst);
            return Err(e);
        }
    }
    Ok(())
}

/// iCloud Drive's whole tree into `folder/iCloud Drive`, carried on in place every time by the
/// same engine as a Drive folder's download (folder_download.rs): any size, safe names, whole
/// files only, unchanged files left alone. A file changed in iCloud since keeps the version it
/// had, in `folder/Earlier versions`. Nothing is ever deleted.
async fn drive_pass<H: Host>(host: &H, folder: &Path) -> Result<(), String> {
    use crate::folder_download as fd;
    let ((http, drivews, params), docws) = host.drive().await?;
    let drive = fd::Drive {
        http,
        drivews,
        docws,
        params,
        retry_after: Duration::from_secs(1),
        free_space: fd::free_space,
        room_every: fd::ROOM_EVERY,
    };
    let root = quince_core::drive::DriveItem {
        drivewsid: quince_vfs::ROOT.to_string(),
        parent_id: None,
        name: "iCloud Drive".into(),
        extension: None,
        kind: "FOLDER".into(),
        size: None,
        child_count: None,
        created_at: None,
        modified_at: None,
        etag: None,
    };
    let top = folder.join("iCloud Drive");
    let earlier = folder.join("Earlier versions");
    let stop = fd::Stop::default();
    if let Ok(mut s) = DRIVE_STOP.lock() {
        *s = Some(stop.clone());
    }
    if let Ok(mut c) = DRIVE_NOW.lock() {
        *c = DriveCount::default();
    }
    DRIVE_RUNNING.store(true, Ordering::SeqCst);
    host.tell(status(host).await);
    // The window told how it goes about once a second, never a message per file.
    let ticker = {
        let teller = host.clone();
        let (tick, ticker) = futures_util::future::abortable(async move {
            while DRIVE_RUNNING.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                teller.tell(status(&teller).await);
            }
        });
        host.spawn(Box::pin(async move {
            let _ = tick.await;
        }));
        ticker
    };
    // The engine's own reports, counted: the tree's size once known, each file as it is whole
    // in the backup (one already there and unchanged included).
    let counting: fd::Emit = std::sync::Arc::new(|e: fd::DownloadEvent| {
        if let Ok(mut c) = DRIVE_NOW.lock() {
            match e.state {
                "found" | "planned" => {
                    c.files_total = e.sent;
                    c.bytes_total = e.total;
                }
                "done" if !e.folder => {
                    c.files += 1;
                    c.bytes += e.total;
                }
                _ => {}
            }
        }
    });
    let request = fd::Request {
        batch: "backup",
        title: "iCloud Drive",
        items: vec![root],
        into: folder,
        journal: None,
        backup: Some(fd::Backup {
            top: &top,
            earlier: &earlier,
        }),
    };
    let report = fd::run(&drive, request, counting, &stop).await;
    DRIVE_RUNNING.store(false, Ordering::SeqCst);
    ticker.abort();
    if let Ok(mut s) = DRIVE_STOP.lock() {
        *s = None;
    }
    let report = report?;
    if let Some(problem) = report.problem {
        return Err(problem);
    }
    // Kept for the next launch: what the backup holds of iCloud Drive.
    let done = DRIVE_NOW.lock().map(|c| *c).unwrap_or_default();
    {
        let mut library = host.library().lock().await;
        let _ = library.set_state(DRIVE_LAST_KEY, &format!("{},{}", done.files, done.bytes));
    }
    let missed = report.failed.len() + report.more_failed;
    if let Ok(mut n) = DRIVE_NOTE.lock() {
        *n = (missed > 0).then(|| match report.failed.first() {
            Some(first) if missed == 1 => {
                format!("1 iCloud Drive file couldn't be backed up: {first}")
            }
            _ => format!("{missed} iCloud Drive files couldn't be backed up"),
        });
    }
    Ok(())
}

/// The file that says a folder is the backup: made when the backup is first written there.
const MARKER: &str = ".skykeep-backup";

/// Whether `folder` is the backup the library records (`recorded`), there to be written to.
///
/// A network drive or a disk mounted at a fixed folder (`/mnt/nas`) leaves that folder in place,
/// empty, when it is not mounted. Taken for the backup, every file the record names was missing:
/// the record was forgotten and the whole library downloaded again into the computer's own
/// disk, filling it, and copied a second time once the drive was back. As restic, borg and
/// Time Machine know their own folder, the backup marks its folder, and writes only where the
/// mark is. A folder with no mark is the backup when it holds a file the record names (one set
/// up before the mark), or when nothing has been backed up yet; marked then. An empty folder
/// the record says was written to is a drive not there: nothing is forgotten or written, and
/// the pass after next (two minutes on) carries on once it is.
fn ready(folder: &Path, recorded: &[quince_core::backup::Copied]) -> Result<(), String> {
    let away = || {
        format!(
            "The backup folder isn't there: {}. Connect its drive and it carries on.",
            folder.display()
        )
    };
    if !folder.is_dir() {
        return Err(away());
    }
    let marker = folder.join(MARKER);
    if marker.is_file() {
        return Ok(());
    }
    let holds_backup = recorded
        .iter()
        .any(|c| std::fs::metadata(folder.join(&c.path)).is_ok_and(|m| m.len() as i64 == c.size));
    if !recorded.is_empty() && !holds_backup {
        return Err(away());
    }
    std::fs::write(
        &marker,
        "Skykeep backs up iCloud to this folder. It copies here only while this file is here.\n",
    )
    .map_err(|e| format!("The backup folder can't be written to: {e}"))
}

/// Photos' pass: check the folder against the record (once), then copy everything not copied.
async fn photos_pass<H: Host>(host: &H, folder: &Path, hidden: bool) -> Result<(), String> {
    let folder = folder.to_path_buf();
    if !CHECKED.swap(true, Ordering::SeqCst) {
        check(host, &folder).await?;
    }
    let taken = Arc::new(Mutex::new(Names::new({
        let library = host.library().lock().await;
        library.backup_paths().map_err(|e| e.to_string())?
    })));
    let mut refused: HashMap<String, u8> = HashMap::new();
    let mut shown = std::time::Instant::now();
    loop {
        let skip = SKIPPED.lock().map(|s| s.clone()).unwrap_or_default();
        let round: Vec<Photo> = {
            let library = host.library().lock().await;
            library
                .backup_pending(hidden, ROUND, &skip)
                .map_err(|e| e.to_string())?
        };
        if round.is_empty() {
            return Ok(());
        }
        for (id, outcome) in copy_round(host, &folder, round, &taken).await? {
            match outcome {
                Outcome::Copied(files) => {
                    let mut library = host.library().lock().await;
                    library.backup_copied(&files).map_err(|e| e.to_string())?;
                }
                // Its link refused (expired): looked up afresh in the next round, a few times.
                Outcome::Refreshed => {
                    let tries = refused.entry(id.clone()).or_default();
                    *tries += 1;
                    if *tries >= TRIES {
                        skip_this_run(id);
                    }
                }
                Outcome::Skipped => skip_this_run(id),
            }
        }
        if shown.elapsed() > Duration::from_secs(1) {
            host.tell(status(host).await);
            shown = std::time::Instant::now();
        }
    }
}

/// A photo iCloud has no copyable file of, or keeps refusing: left alone until the next launch,
/// never retried in a loop that stalls the rest.
fn skip_this_run(id: String) {
    if let Ok(mut s) = SKIPPED.lock() {
        s.push(id);
    }
}

/// Every recorded file gone from the folder or another size: its photo let go of, to be
/// copied again whole.
async fn check<H: Host>(host: &H, folder: &Path) -> Result<(), String> {
    let files = {
        let library = host.library().lock().await;
        library.backup_files().map_err(|e| e.to_string())?
    };
    let folder = folder.to_path_buf();
    let gone = tokio::task::spawn_blocking(move || {
        let mut gone: Vec<(String, String)> = files
            .into_iter()
            .filter(|c| {
                std::fs::metadata(folder.join(&c.path))
                    .map(|m| m.len() as i64)
                    .ok()
                    != Some(c.size)
            })
            .map(|c| (c.source, c.item))
            .collect();
        gone.sort();
        gone.dedup();
        gone
    })
    .await
    .map_err(|e| e.to_string())?;
    if !gone.is_empty() {
        let mut library = host.library().lock().await;
        library.backup_forget(&gone).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What came of one photo in a round.
enum Outcome {
    /// Its files, in the folder and whole.
    Copied(Vec<Copied>),
    /// Apple refused its download link (it expires): tried again with a fresh one.
    Refreshed,
    /// iCloud has no file of it to copy, or no longer has the photo.
    Skipped,
}

/// Where a group of photos lives in Apple's databases: the personal library, or the Shared
/// Library's zone. One lookup per group.
type PlaceKey = Option<(String, String, Option<String>)>;

/// One round: links looked up together, files copied three at a time. Answers what came of
/// each photo; `Err` an outage that ends the pass (offline, Apple refusing, the disk).
async fn copy_round<H: Host>(
    host: &H,
    folder: &Path,
    round: Vec<Photo>,
    taken: &Arc<Mutex<Names>>,
) -> Result<Vec<(String, Outcome)>, String> {
    use futures_util::StreamExt;
    let mut groups: HashMap<PlaceKey, Vec<Photo>> = HashMap::new();
    {
        let library = host.library().lock().await;
        for photo in round {
            let key = library
                .shared_library_of(&photo.id)
                .map_err(|e| e.to_string())?
                .map(|s| (s.zone, s.database, s.owner));
            groups.entry(key).or_default().push(photo);
        }
    }
    let (http, root, params) = host.cloudkit().await?;
    let mut outcomes = Vec::new();
    for (key, photos) in groups {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            &http,
            &root,
            quince_icloud::photos::CONTAINER,
            params.clone(),
        );
        let (zc, zone) = match &key {
            Some((zone, database, owner)) => (
                zc.in_database(if database == "shared" {
                    quince_icloud::cloudkit::Database::Shared
                } else {
                    quince_icloud::cloudkit::Database::Private
                })
                .owned_by(owner.clone()),
                zone.clone(),
            ),
            None => {
                let zone = quince_icloud::photos::library_zone(&zc).await;
                (zc, zone)
            }
        };
        let mut names: Vec<String> = photos.iter().map(|p| p.id.clone()).collect();
        names.extend(photos.iter().map(|p| p.master.clone()));
        let records = zc
            .lookup_records(&zone, &names)
            .await
            .map_err(|e| e.to_string())?;
        let jobs = photos.into_iter().map(|photo| {
            let (records, http) = (&records, &http);
            async move {
                let outcome = match (records.get(&photo.id), records.get(&photo.master)) {
                    (Some(asset), Some(master)) => {
                        copy_one(http, folder, &photo, asset, master, taken).await?
                    }
                    // Gone from iCloud since the library last heard.
                    _ => Outcome::Skipped,
                };
                Ok::<_, String>((photo.id, outcome))
            }
        });
        let mut done = futures_util::stream::iter(jobs).buffer_unordered(AT_ONCE);
        while let Some(result) = done.next().await {
            outcomes.push(result?);
        }
    }
    Ok(outcomes)
}

/// One photo copied: its original, and a Live Photo's video beside it. A file already in the
/// folder under its own name and of the size Apple gives is taken as it is, not fetched again
/// (a backup folder pointed at again, or a copy that landed just before a quit).
async fn copy_one(
    http: &reqwest::Client,
    folder: &Path,
    photo: &Photo,
    asset: &serde_json::Value,
    master: &serde_json::Value,
    taken: &Arc<Mutex<Names>>,
) -> Result<Outcome, String> {
    let video = photo.kind == Kind::Video;
    let Some((url, name)) = quince_icloud::photos::download_choice(
        asset,
        master,
        quince_icloud::photos::Want::Originals,
        video,
    ) else {
        return Ok(Outcome::Skipped);
    };
    let dir = month_folder(photo.taken_at);
    let expected = size_of(master, "resOriginalRes");
    let mut files = Vec::new();
    let rel = match fetch(http, folder, taken, &dir, &name, &url, expected).await? {
        Fetched::Done(rel, size) => {
            dated(&folder.join(&rel), photo.taken_at).await;
            files.push(copied(photo, ORIGINAL, &rel, size));
            rel
        }
        Fetched::Found(rel, size) => {
            files.push(copied(photo, ORIGINAL, &rel, size));
            rel
        }
        Fetched::Refused => return Ok(Outcome::Refreshed),
        Fetched::Missing => return Ok(Outcome::Skipped),
    };
    if photo.kind == Kind::Live {
        if let Some(live) = quince_icloud::photos::links_of(master).live {
            let stem = Path::new(&rel)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let expected = size_of(master, "resOriginalVidComplRes");
            match fetch(
                http,
                folder,
                taken,
                &dir,
                &format!("{stem}.MOV"),
                &live,
                expected,
            )
            .await?
            {
                Fetched::Done(live_rel, size) => {
                    dated(&folder.join(&live_rel), photo.taken_at).await;
                    files.push(copied(photo, LIVE, &live_rel, size));
                }
                Fetched::Found(live_rel, size) => files.push(copied(photo, LIVE, &live_rel, size)),
                Fetched::Refused => return Ok(Outcome::Refreshed),
                // A still without its video is still a photo: kept, the video left out.
                Fetched::Missing => {}
            }
        }
    }
    Ok(Outcome::Copied(files))
}

fn copied(photo: &Photo, part: &str, path: &str, size: u64) -> Copied {
    Copied {
        source: PHOTOS.into(),
        item: photo.id.clone(),
        part: part.into(),
        path: path.into(),
        size: size as i64,
        version: None,
    }
}

/// The size Apple gives for a master's file, when it gives one.
fn size_of(master: &serde_json::Value, resource: &str) -> Option<u64> {
    master
        .pointer(&format!("/fields/{resource}/value/size"))
        .and_then(serde_json::Value::as_u64)
}

enum Fetched {
    Done(String, u64),
    /// Already in the folder, whole: taken as it is, left untouched.
    Found(String, u64),
    /// Apple refused the link (expired, 401/403/410).
    Refused,
    /// Apple has no such file (404).
    Missing,
}

/// One file into `dir`: taken as it is when already there whole, otherwise downloaded under a
/// free name and checked against the size Apple gives. `Err` an outage that ends the pass.
async fn fetch(
    http: &reqwest::Client,
    folder: &Path,
    taken: &Arc<Mutex<Names>>,
    dir: &str,
    name: &str,
    url: &str,
    expected: Option<u64>,
) -> Result<Fetched, String> {
    // Already there whole: the first of `name`, `name 2`, … on disk that no other photo has
    // and that is the size Apple gives (two photos may share a name; each has its own file).
    if let Some(size) = expected {
        let found = taken
            .lock()
            .map(|mut t| t.adopt(folder, dir, name, size))
            .unwrap_or(None);
        if let Some(rel) = found {
            return Ok(Fetched::Found(rel, size));
        }
    }
    let rel = taken
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .reserve(folder, dir, name);
    let path = folder.join(&rel);
    // The name was made one plain file name, but nothing is written until that is proven:
    // a file anywhere but directly in the month's folder is left out, never created.
    let month = folder.join(dir);
    if !crate::saving::directly_in(&month, &path) {
        return Ok(Fetched::Missing);
    }
    if let Err(e) = tokio::fs::create_dir_all(&month).await {
        return Err(disk_trouble(e.kind(), &e.to_string(), folder));
    }
    // A `.part` under this name is what a download cut short by a quit or a crash left behind:
    // no other is being written (passes never overlap, and this name was given to this file
    // alone). Left there, it refused the new download, the pass ended on it, and as the same
    // photo came first again every pass after it did too: nothing more was ever backed up.
    // Only a file or a link goes, never what a link points to.
    let part = crate::fetch::part_of(&path);
    if tokio::fs::symlink_metadata(&part)
        .await
        .is_ok_and(|m| !m.is_dir())
    {
        if let Err(e) = tokio::fs::remove_file(&part).await {
            return Err(disk_trouble(e.kind(), &e.to_string(), folder));
        }
    }
    match stream_to_new_file(http, url, &path).await {
        Ok(size) if expected.is_none_or(|e| e == size) => Ok(Fetched::Done(rel, size)),
        Ok(_) => {
            // Shorter or longer than Apple said: not kept, and tried again next pass.
            tokio::fs::remove_file(&path).await.ok();
            Err("A download arrived incomplete. It's tried again shortly.".into())
        }
        Err(f) => match (f.status, f.disk) {
            (_, Some(kind)) => Err(disk_trouble(kind, &f.message, folder)),
            (Some(401 | 403 | 410), _) => Ok(Fetched::Refused),
            (Some(404), _) => Ok(Fetched::Missing),
            // Offline, Apple busy or refusing service: the pass ends, and carries on later.
            _ => Err(f.message),
        },
    }
}

/// The disk's trouble, in words a person can act on.
fn disk_trouble(kind: std::io::ErrorKind, message: &str, folder: &Path) -> String {
    match kind {
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded => format!(
            "The backup drive is full. Free up space on it, or choose another folder. ({})",
            folder.display()
        ),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            format!(
                "Skykeep can't write to the backup folder: {}. Choose another folder.",
                folder.display()
            )
        }
        _ => format!("The backup folder couldn't be written: {message}"),
    }
}

/// `Photos/<year>/<month>`, by the day it was taken on this computer's clock.
fn month_folder(taken_at: i64) -> String {
    let zone = jiff::tz::TimeZone::system();
    let (y, m) = jiff::Timestamp::from_second(taken_at)
        .map(|t| t.to_zoned(zone).date())
        .map_or((1970, 1), |d| (d.year(), d.month()));
    format!("Photos/{y:04}/{m:02}")
}

/// The names in the backup folder: those a photo has (recorded, found or being copied), and
/// those on disk, compared ignoring case (a backup drive is often exFAT or NTFS, where
/// `IMG_1.HEIC` and `img_1.heic` are one file). Kept apart: a file on disk is not yet any
/// photo's, so a photo can find its own among them.
struct Names {
    claimed: HashSet<String>,
    on_disk: HashMap<String, HashSet<String>>,
}

impl Names {
    fn new(claimed: HashSet<String>) -> Self {
        Names {
            claimed,
            on_disk: HashMap::new(),
        }
    }

    /// `name`, then `name 2`, `name 3`, … in `dir`. The name is the photo's owner's (a Shared
    /// Library's are other people's), so it is made one plain file name first, the same way a
    /// single download is ([`crate::names::file_name`]): `../../x` or `a\b` would otherwise put
    /// the file outside the month's folder.
    fn candidates<'a>(dir: &'a str, name: &str) -> impl Iterator<Item = String> + 'a {
        let name = crate::names::file_name(name, "Photo");
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        (1..).map(move |n| {
            if n == 1 {
                format!("{dir}/{stem}{ext}")
            } else {
                format!("{dir}/{stem} {n}{ext}")
            }
        })
    }

    fn present(&mut self, folder: &Path, dir: &str, rel: &str) -> bool {
        let listed = self.on_disk.entry(dir.to_lowercase()).or_insert_with(|| {
            std::fs::read_dir(folder.join(dir))
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    format!(
                        "{}/{}",
                        dir.to_lowercase(),
                        e.file_name().to_string_lossy().to_lowercase()
                    )
                })
                .collect()
        });
        listed.contains(&rel.to_lowercase())
    }

    /// A file already on disk for this photo: the first of `name`, `name 2`, … that is there,
    /// is no other photo's, and is `size` bytes. Claimed when found.
    fn adopt(&mut self, folder: &Path, dir: &str, name: &str, size: u64) -> Option<String> {
        for rel in Self::candidates(dir, name) {
            if !self.present(folder, dir, &rel) {
                return None;
            }
            let key = rel.to_lowercase();
            if !self.claimed.contains(&key)
                && std::fs::metadata(folder.join(&rel)).map(|m| m.len()).ok() == Some(size)
            {
                self.claimed.insert(key);
                return Some(rel);
            }
        }
        None
    }

    /// A name for a new file: the first of `name`, `name 2`, … neither on disk nor any photo's.
    fn reserve(&mut self, folder: &Path, dir: &str, name: &str) -> String {
        for rel in Self::candidates(dir, name) {
            let key = rel.to_lowercase();
            if !self.claimed.contains(&key) && !self.present(folder, dir, &rel) {
                self.claimed.insert(key);
                return rel;
            }
        }
        unreachable!("a free name is always found")
    }
}

/// The file dated the day the photo was taken, as a camera's own files are.
async fn dated(path: &Path, taken_at: i64) {
    let path = path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        let when = std::time::UNIX_EPOCH + Duration::from_secs(taken_at.max(0) as u64);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(when))
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backup writes only into its own folder: a new one, or one made before the mark, is
    /// marked; a mount point left empty by a drive not mounted is refused, with nothing written
    /// and the record kept.
    #[test]
    fn a_backup_writes_only_into_its_own_folder() {
        let base = std::env::temp_dir().join(format!("skykeep-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let copied = |path: &str, size: i64| quince_core::backup::Copied {
            source: "photos".into(),
            item: path.into(),
            part: "original".into(),
            path: path.into(),
            size,
            version: None,
        };
        // New: nothing recorded yet, so the folder is marked and written to.
        let new = base.join("new");
        std::fs::create_dir_all(&new).unwrap();
        assert!(ready(&new, &[]).is_ok());
        assert!(new.join(MARKER).is_file());

        // Made before the mark: it holds a recorded file, so it is marked.
        let old = base.join("old");
        std::fs::create_dir_all(old.join("Photos/2026/11")).unwrap();
        std::fs::write(old.join("Photos/2026/11/IMG_1.JPG"), b"12345").unwrap();
        let record = [
            copied("Photos/2026/11/IMG_1.JPG", 5),
            copied("Photos/2026/11/IMG_2.JPG", 9),
        ];
        assert!(ready(&old, &record).is_ok());
        assert!(old.join(MARKER).is_file());

        // A mount point the drive is not mounted on: empty, though the record names files.
        let unmounted = base.join("nas");
        std::fs::create_dir_all(&unmounted).unwrap();
        assert!(ready(&unmounted, &record).is_err());
        assert_eq!(
            std::fs::read_dir(&unmounted).unwrap().count(),
            0,
            "nothing written"
        );

        // Marked: written to, whatever the record says.
        std::fs::write(unmounted.join(MARKER), b"").unwrap();
        assert!(ready(&unmounted, &record).is_ok());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn names_never_collide_on_disk_or_between_copies() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        std::fs::create_dir_all(folder.join("Photos/2026/11")).unwrap();
        std::fs::write(folder.join("Photos/2026/11/IMG_1.HEIC"), b"x").unwrap();
        let mut names = Names::new(HashSet::new());
        let a = names.reserve(folder, "Photos/2026/11", "IMG_1.HEIC");
        let b = names.reserve(folder, "Photos/2026/11", "IMG_1.HEIC");
        let c = names.reserve(folder, "Photos/2026/11", "img_1.heic");
        assert_eq!(a, "Photos/2026/11/IMG_1 2.HEIC");
        assert_eq!(b, "Photos/2026/11/IMG_1 3.HEIC");
        assert_eq!(
            c, "Photos/2026/11/img_1 4.heic",
            "names differing only in case"
        );
    }

    /// Two photos of one name, copied as `IMG_7.HEIC` and `IMG_7 2.HEIC`, are each found again
    /// as they are (a folder pointed at again): neither is copied a second time.
    #[test]
    fn photos_sharing_a_name_each_find_their_own_file() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let month = folder.join("Photos/2024/03");
        std::fs::create_dir_all(&month).unwrap();
        std::fs::write(month.join("IMG_7.HEIC"), vec![0u8; 10]).unwrap();
        std::fs::write(month.join("IMG_7 2.HEIC"), vec![0u8; 20]).unwrap();
        let mut names = Names::new(HashSet::new());
        assert_eq!(
            names
                .adopt(folder, "Photos/2024/03", "IMG_7.HEIC", 10)
                .as_deref(),
            Some("Photos/2024/03/IMG_7.HEIC")
        );
        assert_eq!(
            names
                .adopt(folder, "Photos/2024/03", "IMG_7.HEIC", 20)
                .as_deref(),
            Some("Photos/2024/03/IMG_7 2.HEIC")
        );
        // A third of that name has no file yet: none found, and a new name taken for it.
        assert_eq!(
            names.adopt(folder, "Photos/2024/03", "IMG_7.HEIC", 30),
            None
        );
        assert_eq!(
            names.reserve(folder, "Photos/2024/03", "IMG_7.HEIC"),
            "Photos/2024/03/IMG_7 3.HEIC"
        );
    }

    /// A Shared Library photo's name is someone else's: whatever it holds, the file it is
    /// copied to, and any file found for it, sits directly in the month's folder.
    #[test]
    fn hostile_names_stay_in_the_month_folder() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let month = "Photos/2026/11";
        std::fs::create_dir_all(folder.join(month)).unwrap();
        // A file outside, where `../x` points: never taken as this photo's.
        std::fs::write(folder.join("Photos/2026/x.JPG"), vec![0u8; 5]).unwrap();
        let mut names = Names::new(HashSet::new());
        for hostile in [
            "../x.JPG",
            "../../../../.config/autostart/x.desktop",
            "a/b.JPG",
            "a\\b.JPG",
            "..",
            ".",
            "CON",
            "CON.JPG",
            "x\0y.JPG",
            "/etc/passwd",
            "C:\\Windows\\x.JPG",
            "name:stream.JPG",
            "",
        ] {
            assert_eq!(names.adopt(folder, month, hostile, 5), None, "{hostile:?}");
            let rel = names.reserve(folder, month, hostile);
            let path = folder.join(&rel);
            assert!(
                crate::saving::directly_in(&folder.join(month), &path),
                "{hostile:?} became {rel:?}"
            );
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(!name.contains('\0') && !name.starts_with('.'), "{name:?}");
            let stem = name.split('.').next().unwrap_or_default().to_uppercase();
            assert_ne!(stem, "CON", "{name:?}");
        }
    }

    /// Apple's files, by path: what each download link answers.
    async fn apple(files: &[(&str, Vec<u8>)]) -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        for (path, bytes) in files {
            wiremock::Mock::given(wiremock::matchers::path(*path))
                .respond_with(wiremock::ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
                .mount(&server)
                .await;
        }
        server
    }

    fn names(claimed: &[&str]) -> Arc<Mutex<Names>> {
        Arc::new(Mutex::new(Names::new(
            claimed.iter().map(|p| p.to_lowercase()).collect(),
        )))
    }

    fn listed(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    /// Two different photos named IMG_0001.JPG taken the same month, of the same size (the
    /// hardest case: no size tells them apart): both kept, each its own bytes. Run again over
    /// the folder (the record lost, a folder pointed at again), each finds its own file and
    /// nothing is fetched, written or renamed; a third of that name gets a third file.
    #[tokio::test(flavor = "multi_thread")]
    async fn two_photos_of_one_name_in_one_month_both_survive_every_run() {
        let (a, b, c) = (vec![1u8; 100], vec![2u8; 100], vec![3u8; 40]);
        let server = apple(&[("/a", a.clone()), ("/b", b.clone()), ("/c", c.clone())]).await;
        let http = reqwest::Client::new();
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let month = "Photos/2026/01";
        let get = |taken: Arc<Mutex<Names>>, link: &'static str, size: usize| {
            let (http, url) = (http.clone(), format!("{}{link}", server.uri()));
            async move {
                fetch(
                    &http,
                    folder,
                    &taken,
                    month,
                    "IMG_0001.JPG",
                    &url,
                    Some(size as u64),
                )
                .await
                .unwrap()
            }
        };

        let first = names(&[]);
        let Fetched::Done(ra, 100) = get(Arc::clone(&first), "/a", 100).await else {
            panic!("a not copied")
        };
        let Fetched::Done(rb, 100) = get(Arc::clone(&first), "/b", 100).await else {
            panic!("b not copied")
        };
        assert_eq!(ra, "Photos/2026/01/IMG_0001.JPG");
        assert_eq!(rb, "Photos/2026/01/IMG_0001 2.JPG");
        assert_eq!(std::fs::read(folder.join(&ra)).unwrap(), a);
        assert_eq!(std::fs::read(folder.join(&rb)).unwrap(), b);
        let asked = server.received_requests().await.unwrap().len();
        assert_eq!(asked, 2);

        // Run again with nothing recorded: each found as it is, none fetched again.
        let again = names(&[]);
        let Fetched::Found(fa, _) = get(Arc::clone(&again), "/a", 100).await else {
            panic!("a fetched again")
        };
        let Fetched::Found(fb, _) = get(Arc::clone(&again), "/b", 100).await else {
            panic!("b fetched again")
        };
        assert_ne!(fa, fb, "never both taken as one file");
        assert_eq!(server.received_requests().await.unwrap().len(), asked);

        // Run again with both recorded: a third photo of that name is a third file.
        let recorded = names(&[&ra, &rb]);
        let Fetched::Done(rc, 40) = get(recorded, "/c", 40).await else {
            panic!("c not copied")
        };
        assert_eq!(rc, "Photos/2026/01/IMG_0001 3.JPG");
        assert_eq!(
            listed(&folder.join(month)),
            ["IMG_0001 2.JPG", "IMG_0001 3.JPG", "IMG_0001.JPG"]
        );
        assert_eq!(std::fs::read(folder.join(&ra)).unwrap(), a, "a untouched");
        assert_eq!(std::fs::read(folder.join(&rb)).unwrap(), b, "b untouched");
        assert_eq!(std::fs::read(folder.join(&rc)).unwrap(), c);
    }

    /// Skykeep quit (or the computer slept for good) half way through a video: its `.part` is
    /// left. The next pass copies that photo whole all the same; the leftover never ends the
    /// pass, so it never stops every pass after it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_cut_short_by_a_quit_never_stalls_the_backup() {
        let video = vec![7u8; 5000];
        let server = apple(&[("/v", video.clone())]).await;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let month = "Photos/2026/02";
        std::fs::create_dir_all(folder.join(month)).unwrap();
        std::fs::write(folder.join(month).join("IMG_0002.MOV.part"), [0u8; 1200]).unwrap();

        let got = fetch(
            &reqwest::Client::new(),
            folder,
            &names(&[]),
            month,
            "IMG_0002.MOV",
            &format!("{}/v", server.uri()),
            Some(5000),
        )
        .await;
        let Ok(Fetched::Done(rel, 5000)) = got else {
            panic!("the backup stopped on a leftover part")
        };
        assert_eq!(rel, "Photos/2026/02/IMG_0002.MOV");
        assert_eq!(std::fs::read(folder.join(&rel)).unwrap(), video);
        assert_eq!(
            listed(&folder.join(month)),
            ["IMG_0002.MOV"],
            "nothing left over"
        );
    }

    #[test]
    fn a_photo_goes_under_the_month_it_was_taken() {
        // 15 Jul 2025, midday UTC: July in every time zone.
        assert!(month_folder(1_752_580_800).ends_with("/2025/07"));
        assert!(month_folder(1_752_580_800).starts_with("Photos/"));
    }
}
