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

use crate::photos::stream_to_new_file;
use crate::Data;
use quince_core::backup::{Copied, Progress, LIVE, ORIGINAL, PHOTOS};
use quince_core::photos::{Kind, Photo};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const FOLDER_KEY: &str = "backup.folder";
const HIDDEN_KEY: &str = "backup.hidden";
/// iCloud Drive in the backup too: on unless turned off ("0").
const DRIVE_KEY: &str = "backup.drive";
/// The window hears how a backup is going here.
pub const PROGRESS: &str = "quince://backup";

/// Downloads at once: enough to keep a connection busy, few enough to stay polite.
const AT_ONCE: usize = 3;
/// Photos taken per round, looked up together: few enough that their download links (which
/// expire) are still good when the last of the round's videos starts.
const ROUND: usize = 50;
/// Tries for one photo whose download link Apple refused (expired), each with fresh links.
const TRIES: u8 = 2;
/// After a pass that had to stop (offline, Apple busy), the next one.
const AGAIN_AFTER: Duration = Duration::from_secs(120);

/// The app, kept at launch for Photos' change notice (refresh.rs has none of its own).
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

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

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    folder: Option<String>,
    hidden: bool,
    copied: i64,
    total: i64,
    running: bool,
    problem: Option<String>,
    /// Hidden is locked this run: including it asks for the unlock first (the window's sheet).
    hidden_locked: bool,
    /// iCloud Drive is in the backup too.
    drive: bool,
    /// iCloud Drive's pass is under way.
    drive_running: bool,
    /// What iCloud Drive's last pass couldn't copy, said in words.
    drive_note: Option<String>,
    /// Photos' bytes in the backup, and of all there are.
    photos_bytes: i64,
    photos_bytes_total: i64,
    /// iCloud Drive: files and bytes in the backup, of all there are (as it goes, or as the
    /// last pass finished).
    drive_files: u64,
    drive_files_total: u64,
    drive_bytes: u64,
    drive_bytes_total: u64,
}

struct Settings {
    folder: Option<PathBuf>,
    hidden: bool,
    drive: bool,
}

async fn settings(app: &AppHandle) -> Settings {
    let data = app.state::<Data>();
    let library = data.0.lock().await;
    let folder = library
        .state(FOLDER_KEY)
        .ok()
        .flatten()
        .filter(|f| !f.is_empty())
        .map(PathBuf::from);
    let hidden = library.state(HIDDEN_KEY).ok().flatten().as_deref() == Some("1");
    let drive = library.state(DRIVE_KEY).ok().flatten().as_deref() != Some("0");
    Settings {
        folder,
        hidden,
        drive,
    }
}

async fn status(app: &AppHandle) -> Status {
    let Settings {
        folder,
        hidden,
        drive,
    } = settings(app).await;
    let Progress {
        copied,
        total,
        copied_bytes,
        total_bytes,
    } = {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        library.backup_progress(hidden).unwrap_or_default()
    };
    let drive_count = if DRIVE_RUNNING.load(Ordering::SeqCst) {
        DRIVE_NOW.lock().map(|c| *c).unwrap_or_default()
    } else {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        let last = library
            .state(DRIVE_LAST_KEY)
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
        hidden_locked: !crate::photos::HIDDEN_OPEN.load(Ordering::SeqCst),
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

/// The backup folder, shown in the file manager.
#[tauri::command]
pub async fn backup_reveal(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let folder = settings(&app).await.folder.filter(|f| f.is_dir()).ok_or(
        "The backup folder isn\u{2019}t there. Connect its drive, or choose another folder.",
    )?;
    app.opener()
        .reveal_item_in_dir(&folder)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn backup_status(app: AppHandle) -> Result<Status, String> {
    Ok(status(&app).await)
}

/// Backup on into `folder` (chosen in the folder dialog, so granted), or off (`None`); with the
/// Hidden album or without; with iCloud Drive or without. A new folder starts from nothing:
/// what is there is checked as it is copied, never assumed.
#[tauri::command]
pub async fn backup_set(
    app: AppHandle,
    folder: Option<String>,
    hidden: bool,
    drive: bool,
) -> Result<Status, String> {
    if let Some(f) = &folder {
        crate::grants::take_write(f)?;
    }
    {
        let data = app.state::<Data>();
        let mut library = data.0.lock().await;
        // Hidden copied into a plain folder would step around its lock: turned on only while
        // it is unlocked (the Apple ID password or Windows Hello, in Photos).
        let had_hidden = library.state(HIDDEN_KEY).ok().flatten().as_deref() == Some("1");
        if hidden && !had_hidden && !crate::photos::HIDDEN_OPEN.load(Ordering::SeqCst) {
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
    kick(&app);
    Ok(status(&app).await)
}

/// At launch: the app kept for Photos' notices, which come from refresh.rs with no app of their
/// own, so a launch in the tray (no window to sign in) still backs up.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// Photos changed at Apple: what is new goes into the backup, when backup is on.
pub fn photos_changed() {
    if let Some(app) = APP.get() {
        kick(app);
    }
}

/// iCloud Drive changed at Apple (its change feed, drive_watch.rs): gone through again.
pub fn drive_changed() {
    DRIVE_DUE.store(true, Ordering::SeqCst);
    if let Some(app) = APP.get() {
        kick(app);
    }
}

/// Copy what isn't copied yet, if backup is on: at launch, when Photos changed, when turned on.
pub fn kick(app: &AppHandle) {
    let _ = APP.set(app.clone());
    if RUNNING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let retry = loop {
            AGAIN.store(false, Ordering::SeqCst);
            let ended = pass(&app).await;
            if let Ok(mut p) = PROBLEM.lock() {
                *p = ended.clone().err();
            }
            if !AGAIN.load(Ordering::SeqCst) {
                break ended.is_err();
            }
        };
        RUNNING.store(false, Ordering::SeqCst);
        app.emit(PROGRESS, status(&app).await).ok();
        if retry {
            tokio::time::sleep(AGAIN_AFTER).await;
            kick(&app);
        }
    });
}

/// One pass: Photos, then iCloud Drive when it is due.
async fn pass(app: &AppHandle) -> Result<(), String> {
    let Settings {
        folder: Some(folder),
        hidden,
        drive,
    } = settings(app).await
    else {
        return Ok(());
    };
    photos_pass(app, &folder, hidden).await?;
    if drive && DRIVE_DUE.swap(false, Ordering::SeqCst) {
        if let Err(e) = drive_pass(app, &folder).await {
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
async fn drive_pass(app: &AppHandle, folder: &Path) -> Result<(), String> {
    use crate::folder_download as fd;
    let (http, drivews, params) = crate::drive_context(&app.state::<crate::Auth>()).await?;
    let docws = {
        let auth = app.state::<crate::Auth>();
        let client = auth.client.lock().await;
        client
            .session()
            .service_url("docws")
            .ok_or("Apple hasn't offered iCloud Drive downloads for this account.")?
    };
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
    app.emit(PROGRESS, status(app).await).ok();
    // The window told how it goes about once a second, never a message per file.
    let ticker = {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while DRIVE_RUNNING.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(1)).await;
                app.emit(PROGRESS, status(&app).await).ok();
            }
        })
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
        let data = app.state::<Data>();
        let mut library = data.0.lock().await;
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

/// Photos' pass: check the folder against the record (once), then copy everything not copied.
async fn photos_pass(app: &AppHandle, folder: &Path, hidden: bool) -> Result<(), String> {
    let folder = folder.to_path_buf();
    // An external drive unplugged is not an empty backup: nothing is forgotten, it waits.
    if !folder.is_dir() {
        return Err(format!(
            "The backup folder isn't there: {}. Connect its drive and it carries on.",
            folder.display()
        ));
    }
    if !CHECKED.swap(true, Ordering::SeqCst) {
        check(app, &folder).await?;
    }
    let taken = Arc::new(Mutex::new(Names::new({
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        library.backup_paths().map_err(|e| e.to_string())?
    })));
    let mut refused: HashMap<String, u8> = HashMap::new();
    let mut shown = std::time::Instant::now();
    loop {
        let skip = SKIPPED.lock().map(|s| s.clone()).unwrap_or_default();
        let round: Vec<Photo> = {
            let data = app.state::<Data>();
            let library = data.0.lock().await;
            library
                .backup_pending(hidden, ROUND, &skip)
                .map_err(|e| e.to_string())?
        };
        if round.is_empty() {
            return Ok(());
        }
        for (id, outcome) in copy_round(app, &folder, round, &taken).await? {
            match outcome {
                Outcome::Copied(files) => {
                    let data = app.state::<Data>();
                    let mut library = data.0.lock().await;
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
            app.emit(PROGRESS, status(app).await).ok();
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
async fn check(app: &AppHandle, folder: &Path) -> Result<(), String> {
    let files = {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
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
        let data = app.state::<Data>();
        let mut library = data.0.lock().await;
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
async fn copy_round(
    app: &AppHandle,
    folder: &Path,
    round: Vec<Photo>,
    taken: &Arc<Mutex<Names>>,
) -> Result<Vec<(String, Outcome)>, String> {
    use futures_util::StreamExt;
    let mut groups: HashMap<PlaceKey, Vec<Photo>> = HashMap::new();
    {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        for photo in round {
            let key = library
                .shared_library_of(&photo.id)
                .map_err(|e| e.to_string())?
                .map(|s| (s.zone, s.database, s.owner));
            groups.entry(key).or_default().push(photo);
        }
    }
    let (http, root, params) = crate::cloudkit_context(&app.state::<crate::Auth>()).await?;
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
    if let Err(e) = tokio::fs::create_dir_all(path.parent().unwrap_or(folder)).await {
        return Err(disk_trouble(e.kind(), &e.to_string(), folder));
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

    /// `name`, then `name 2`, `name 3`, … in `dir`.
    fn candidates<'a>(dir: &'a str, name: &'a str) -> impl Iterator<Item = String> + 'a {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
            _ => (name, String::new()),
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

    #[test]
    fn a_photo_goes_under_the_month_it_was_taken() {
        // 15 Jul 2025, midday UTC: July in every time zone.
        assert!(month_folder(1_752_580_800).ends_with("/2025/07"));
        assert!(month_folder(1_752_580_800).starts_with("Photos/"));
    }
}
