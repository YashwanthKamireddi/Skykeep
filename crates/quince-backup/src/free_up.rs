//! Free up iCloud space: photos and videos the backup holds leave iCloud, each one checked
//! against Apple's own copy at that moment. Which photos may go is the library's call
//! (quince_core::free_up); this is the check and the removal. docs/free-up-space.md has the
//! research: every rule below is a lost photo in another tool.
//!
//! A photo leaves only when, read from Apple now:
//! - it has no edit (an edit lives in iCloud, and the backup keeps originals);
//! - every file Apple keeps for it (the original; a Live Photo's video) is in the backup, the
//!   size Apple gives, and the same bytes: Apple's own checksum (`fileChecksum`, the salted
//!   SHA-1 the Drive code proves uploads with) matches the file on the backup drive.
//!
//! Removal is Photos' own Delete: to Recently Deleted, through the outbox (shown at once, sent
//! in batches, anything Apple refuses put back). Nothing leaves while Apple says the library is
//! still being prepared (icloud-photos-sync#1181), or while the backup folder isn't there.

use crate::backup::Host;
use quince_core::backup::{Copied, LIVE, ORIGINAL};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Photos looked up and checked together.
const BATCH: usize = 50;

static RUNNING: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static NOW: Mutex<Progress> = Mutex::new(Progress::ZERO);

/// How freeing up space is going, or how it last ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub running: bool,
    /// Photos checked of all asked for.
    pub checked: u64,
    pub total: u64,
    /// Removed from iCloud, and their bytes.
    pub freed: u64,
    pub freed_bytes: u64,
    /// Kept in iCloud: edited; not the same as Apple's (or not whole in the backup); gone from
    /// iCloud already.
    pub edited: u64,
    pub not_same: u64,
    pub gone: u64,
}

impl Progress {
    const ZERO: Progress = Progress {
        running: false,
        checked: 0,
        total: 0,
        freed: 0,
        freed_bytes: 0,
        edited: 0,
        not_same: 0,
        gone: 0,
    };
}

/// How it is going now.
#[must_use]
pub fn progress() -> Progress {
    NOW.lock().map(|p| *p).unwrap_or_default()
}

/// Stops a run after the batch under way; what was removed stays removed.
pub fn stop() {
    STOP.store(true, Ordering::SeqCst);
}

/// What one photo's check came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every file is in the backup, the same bytes as Apple's: it may leave.
    Free,
    /// Edited on a device: the edit lives in iCloud only.
    Edited,
    /// A file missing from the backup, another size, or other bytes than Apple's.
    NotSame,
}

/// Apple's files for a photo that the backup must hold, by the backup's part name.
const PARTS: [(&str, &str); 2] = [
    (ORIGINAL, "resOriginalRes"),
    (LIVE, "resOriginalVidComplRes"),
];

/// Whether a photo the sheet listed may still leave, as the library has it now: not deleted,
/// favourited or hidden on a device since, not in a Shared Library, its date known.
#[must_use]
pub fn still_may_leave(p: &quince_core::photos::Photo, in_shared_library: bool) -> bool {
    !p.deleted && !p.favorite && !p.hidden && !in_shared_library && p.taken_at > 0
}

/// Whether a photo was edited on a device: its asset carries an adjustment (`adjustmentType`)
/// or the edit's own full picture (`resJPEGFullRes`, as icloud.com and the grid read an edit,
/// `links_with_edit`). Not `adjustmentRenderType` or `adjustmentSourceType`: measured on a real
/// library (27 Nov 2026), Apple puts those on almost every photo, edited or not (481 of 544),
/// where an edit's own fields were on 40.
#[must_use]
pub fn edited(asset: &Value) -> bool {
    let f = &asset["fields"];
    ["adjustmentType", "resJPEGFullRes"]
        .iter()
        .any(|k| !f[k].is_null() && !f[k]["value"].is_null())
}

/// One photo judged against Apple's records as they are now. Blocking (reads whole files).
#[must_use]
pub fn judge(folder: &Path, files: &[Copied], asset: &Value, master: &Value) -> Verdict {
    if edited(asset) {
        return Verdict::Edited;
    }
    let mut any = false;
    for (part, field) in PARTS {
        let value = &master["fields"][field]["value"];
        if value.is_null() {
            continue;
        }
        any = true;
        let (Some(size), Some(sum)) = (value["size"].as_u64(), value["fileChecksum"].as_str())
        else {
            return Verdict::NotSame;
        };
        let Some(copy) = files.iter().find(|c| c.part == part) else {
            return Verdict::NotSame;
        };
        if copy.size != size as i64
            || checksum(&folder.join(&copy.path), size).as_deref() != Some(sum)
        {
            return Verdict::NotSame;
        }
    }
    // A master with no original Apple will name is nothing to prove: it stays.
    if any {
        Verdict::Free
    } else {
        Verdict::NotSame
    }
}

/// Apple's checksum of a file on disk, when it is `size` bytes.
fn checksum(path: &Path, size: u64) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() != size {
        return None;
    }
    let mut sum = quince_icloud::drive::AppleChecksum::default();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        sum.update(&buf[..n]);
    }
    Some(sum.finish())
}

/// Removes `ids` from iCloud, each only once judged [`Verdict::Free`]; answers how it ended.
/// One run at a time.
///
/// # Errors
/// When nothing can be checked: no backup folder, the drive not there, iCloud preparing the
/// library, or Apple out of reach. Photos already removed stay removed.
pub async fn run<H: Host>(host: &H, ids: Vec<String>) -> Result<Progress, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Space is already being freed up.".into());
    }
    STOP.store(false, Ordering::SeqCst);
    let ended = run_inner(host, ids).await;
    RUNNING.store(false, Ordering::SeqCst);
    if let Ok(mut p) = NOW.lock() {
        p.running = false;
    }
    host.tell(crate::backup::status(host).await);
    ended.map(|()| progress())
}

async fn run_inner<H: Host>(host: &H, ids: Vec<String>) -> Result<(), String> {
    let folder = backup_folder(host).await?;
    let (http, root, params) = host.cloudkit().await?;
    if let quince_icloud::photos::Readiness::Preparing { .. } =
        quince_icloud::photos::readiness(&http, &root, params.clone())
            .await
            .map_err(|e| e.to_string())?
    {
        return Err("iCloud is still preparing your photo library. Try again later.".into());
    }
    if let Ok(mut p) = NOW.lock() {
        *p = Progress {
            running: true,
            total: ids.len() as u64,
            ..Progress::ZERO
        };
    }
    host.tell(crate::backup::status(host).await);
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        &http,
        &root,
        quince_icloud::photos::CONTAINER,
        params,
    );
    let zone = quince_icloud::photos::library_zone(&zc).await;
    for batch in ids.chunks(BATCH) {
        if STOP.load(Ordering::SeqCst) {
            break;
        }
        let (photos, files) = {
            let library = host.library().lock().await;
            // Asked again now, not as the sheet saw it: a photo favourited, hidden, deleted or
            // moved into a Shared Library on a device since stays.
            let mut photos = Vec::new();
            for id in batch {
                let Some(p) = library.photo(id).map_err(|e| e.to_string())? else {
                    continue;
                };
                let shared = library.photo_zone(id).map_err(|e| e.to_string())?.is_some();
                if still_may_leave(&p, shared) {
                    photos.push(p);
                }
            }
            let files = library.free_up_files(batch).map_err(|e| e.to_string())?;
            (photos, files)
        };
        let mut names: Vec<String> = photos.iter().map(|p| p.id.clone()).collect();
        names.extend(photos.iter().map(|p| p.master.clone()));
        let records = zc
            .lookup_records(&zone, &names)
            .await
            .map_err(|e| e.to_string())?;
        let at = folder.clone();
        let judged = tokio::task::spawn_blocking(move || {
            photos
                .into_iter()
                .map(|p| {
                    let verdict = match (records.get(&p.id), records.get(&p.master)) {
                        (Some(asset), Some(master)) => {
                            let mine: Vec<Copied> =
                                files.iter().filter(|c| c.item == p.id).cloned().collect();
                            Some(judge(&at, &mine, asset, master))
                        }
                        _ => None,
                    };
                    (p, verdict)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| e.to_string())?;
        let free: Vec<String> = judged
            .iter()
            .filter(|(_, v)| *v == Some(Verdict::Free))
            .map(|(p, _)| p.id.clone())
            .collect();
        let free_bytes: i64 = judged
            .iter()
            .filter(|(_, v)| *v == Some(Verdict::Free))
            .map(|(p, _)| p.size.unwrap_or(0) + p.live_size.unwrap_or(0))
            .sum();
        if !free.is_empty() {
            let mut library = host.library().lock().await;
            library
                .free_up_remember(&free, jiff::Timestamp::now().as_second())
                .map_err(|e| e.to_string())?;
            quince_sync::photos::record_flags(
                &mut library,
                &free,
                quince_icloud::photos::Flag::Deleted,
                true,
            )?;
        }
        host.send_changes();
        if let Ok(mut p) = NOW.lock() {
            p.checked += batch.len() as u64;
            p.freed += free.len() as u64;
            p.freed_bytes += free_bytes.max(0) as u64;
            for (_, v) in &judged {
                match v {
                    Some(Verdict::Edited) => p.edited += 1,
                    Some(Verdict::NotSame) => p.not_same += 1,
                    None => p.gone += 1,
                    Some(Verdict::Free) => {}
                }
            }
            // Asked for but no longer in the library, or no longer one that may leave.
            p.gone += (batch.len() - judged.len()) as u64;
        }
        host.tell(crate::backup::status(host).await);
    }
    Ok(())
}

// ---- iCloud Drive -------------------------------------------------------------------------

/// The zone of iCloud Drive's own files: App Libraries are other zones (Apple deletes those at
/// once, with no Recently Deleted), and files shared with you have other kinds of id.
const DRIVE_FILES: &str = "FILE::com.apple.CloudDocs::";

/// One iCloud Drive file that may leave iCloud: the backup holds it, Apple still has it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    /// Its folder in iCloud Drive, `/` between names; empty at the top.
    pub folder: String,
    pub bytes: i64,
}

/// Of iCloud Drive's files in the backup, those that stay in iCloud, by why.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveStays {
    /// Pages, Numbers and Keynote bundles: no one checksum to prove them by.
    pub packages: u64,
    pub app_libraries: u64,
    pub shared_with_you: u64,
}

/// What freeing up iCloud Drive would do: the files that may leave, largest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrivePlan {
    pub items: Vec<DriveFile>,
    pub stays: DriveStays,
}

/// iCloud Drive's files in the backup, as recorded (`backup_files`).
fn drive_records(files: Vec<Copied>) -> Vec<Copied> {
    use crate::folder_download::{DRIVE, FILE};
    files
        .into_iter()
        .filter(|c| c.source == DRIVE && c.part == FILE)
        .collect()
}

/// A Drive id's document id, as `download/batch` names it.
fn document_of(drivewsid: &str) -> String {
    // Exactly as Apple gave it: Apple's document ids are case-sensitive, and a batch of ids it
    // does not know answers 404 (measured 27 Nov 2026). Compared without case only here.
    drivewsid
        .rsplit("::")
        .next()
        .unwrap_or(drivewsid)
        .to_string()
}

/// Splits the recorded files into Drive's own (to ask Apple about) and what stays by kind.
fn drive_split(records: Vec<Copied>) -> (Vec<Copied>, DriveStays) {
    let mut stays = DriveStays::default();
    let mut own = Vec::new();
    for c in records {
        if c.item.starts_with(DRIVE_FILES) {
            own.push(c);
        } else if c.item.starts_with("FILE::") {
            stays.app_libraries += 1;
        } else {
            stays.shared_with_you += 1;
        }
    }
    (own, stays)
}

/// A recorded path as the file's name and its folder inside iCloud Drive.
fn name_and_folder(path: &str) -> (String, String) {
    let inside = path.strip_prefix("iCloud Drive/").unwrap_or(path);
    match inside.rsplit_once('/') {
        Some((folder, name)) => (name.to_string(), folder.to_string()),
        None => (inside.to_string(), String::new()),
    }
}

/// The iCloud Drive files that may leave iCloud now: in the backup, in Drive's own zone, and
/// still at Apple (a file deleted since stays in the backup but is no longer counted), largest
/// first; and what stays, by why.
///
/// # Errors
/// When the backup folder isn't there, or Apple out of reach.
pub async fn drive_plan<H: Host>(host: &H) -> Result<DrivePlan, String> {
    backup_folder(host).await?;
    let records = {
        let library = host.library().lock().await;
        drive_records(library.backup_files().map_err(|e| e.to_string())?)
    };
    let (own, mut stays) = drive_split(records);
    let ((http, _drivews, params), docws) = host.drive().await?;
    let ids: Vec<String> = own.iter().map(|c| document_of(&c.item)).collect();
    let sums = quince_icloud::drive::checksums(&http, &docws, params, &ids)
        .await
        .map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for c in own {
        match sums.get(&document_of(&c.item).to_lowercase()) {
            Some(Some(_)) => {
                let (name, folder) = name_and_folder(&c.path);
                items.push(DriveFile {
                    id: c.item,
                    name,
                    folder,
                    bytes: c.size,
                });
            }
            Some(None) => stays.packages += 1,
            None => {}
        }
    }
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.id.cmp(&b.id)));
    Ok(DrivePlan { items, stays })
}

/// The backup folder, when it is there and marked as the backup.
async fn backup_folder<H: Host>(host: &H) -> Result<std::path::PathBuf, String> {
    crate::backup::folder(host)
        .await
        .filter(|f| f.join(crate::backup::MARKER).is_file())
        .ok_or_else(|| "The backup folder isn't there. Connect its drive, then try again.".into())
}

/// Moves `ids` (iCloud Drive files from [`drive_plan`]) to Recently Deleted, each only once its
/// copy in the backup is the same bytes as Apple's now (its checksum), and only as the version
/// the backup copied (Apple refuses one changed since, so it stays). One run at a time.
///
/// # Errors
/// As [`run`].
pub async fn run_drive<H: Host>(host: &H, ids: Vec<String>) -> Result<Progress, String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("Space is already being freed up.".into());
    }
    STOP.store(false, Ordering::SeqCst);
    let ended = run_drive_inner(host, ids).await;
    RUNNING.store(false, Ordering::SeqCst);
    if let Ok(mut p) = NOW.lock() {
        p.running = false;
    }
    host.tell(crate::backup::status(host).await);
    ended.map(|()| progress())
}

async fn run_drive_inner<H: Host>(host: &H, ids: Vec<String>) -> Result<(), String> {
    let folder = backup_folder(host).await?;
    let ((http, drivews, params), docws) = host.drive().await?;
    if let Ok(mut p) = NOW.lock() {
        *p = Progress {
            running: true,
            total: ids.len() as u64,
            ..Progress::ZERO
        };
    }
    host.tell(crate::backup::status(host).await);
    for batch in ids.chunks(BATCH) {
        if STOP.load(Ordering::SeqCst) {
            break;
        }
        let records: Vec<Copied> = {
            let library = host.library().lock().await;
            drive_records(library.backup_files().map_err(|e| e.to_string())?)
                .into_iter()
                .filter(|c| c.item.starts_with(DRIVE_FILES) && batch.contains(&c.item))
                .collect()
        };
        let docs: Vec<String> = records.iter().map(|c| document_of(&c.item)).collect();
        let sums = quince_icloud::drive::checksums(&http, &docws, params.clone(), &docs)
            .await
            .map_err(|e| e.to_string())?;
        let at = folder.clone();
        let judged = tokio::task::spawn_blocking(move || {
            records
                .into_iter()
                .map(|c| {
                    let verdict = match sums.get(&document_of(&c.item).to_lowercase()) {
                        None => None,
                        Some(apple) => {
                            let same = apple.is_some()
                                && c.version.is_some()
                                && checksum(&at.join(&c.path), c.size as u64).as_deref()
                                    == apple.as_deref();
                            Some(same)
                        }
                    };
                    (c, verdict)
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| e.to_string())?;
        let free: Vec<(String, String)> = judged
            .iter()
            .filter(|(_, v)| *v == Some(true))
            .filter_map(|(c, _)| Some((c.item.clone(), c.version.clone()?)))
            .collect();
        let answers = if free.is_empty() {
            std::collections::HashMap::new()
        } else {
            quince_icloud::drive::trash_versions(&http, &drivews, params.clone(), &free)
                .await
                .map_err(|e| e.to_string())?
        };
        if let Ok(mut p) = NOW.lock() {
            p.checked += batch.len() as u64;
            for (c, v) in &judged {
                match (v, answers.get(&c.item).map(String::as_str)) {
                    (Some(true), Some("OK")) => {
                        p.freed += 1;
                        p.freed_bytes += c.size.max(0) as u64;
                    }
                    (None, _) => p.gone += 1,
                    _ => p.not_same += 1,
                }
            }
            p.gone += (batch.len() - judged.len()) as u64;
        }
        host.tell(crate::backup::status(host).await);
    }
    Ok(())
}

#[cfg(test)]
mod drive_tests {
    use super::*;

    fn rec(item: &str, path: &str) -> Copied {
        Copied {
            source: crate::folder_download::DRIVE.into(),
            item: item.into(),
            part: crate::folder_download::FILE.into(),
            path: path.into(),
            size: 5,
            version: Some("e".into()),
        }
    }

    /// Only iCloud Drive's own files are asked about; App Libraries (deleted at once by Apple,
    /// no Recently Deleted) and files shared with you stay, counted.
    #[test]
    fn only_drives_own_files_may_leave() {
        let (own, stays) = drive_split(vec![
            rec("FILE::com.apple.CloudDocs::AAA", "iCloud Drive/Docs/a.pdf"),
            rec("FILE::com.apple.Keynote::BBB", "iCloud Drive/Keynote/b.key"),
            rec(
                "FILE_IN_SHARED_FOLDER::com.apple.CloudDocs::CCC",
                "iCloud Drive/Shared/c.txt",
            ),
            rec(
                "SHARED_FILE::com.apple.CloudDocs::DDD",
                "iCloud Drive/d.txt",
            ),
        ]);
        assert_eq!(own.len(), 1);
        assert_eq!(
            stays,
            DriveStays {
                packages: 0,
                app_libraries: 1,
                shared_with_you: 2
            }
        );
        assert_eq!(document_of(&own[0].item), "AAA", "sent as Apple gave it");
        assert_eq!(
            name_and_folder("iCloud Drive/Docs/Tax/2025.pdf"),
            ("2025.pdf".to_string(), "Docs/Tax".to_string())
        );
        assert_eq!(
            name_and_folder("iCloud Drive/top.txt"),
            ("top.txt".to_string(), String::new())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A photo favourited, hidden, deleted or put in a Shared Library on a device after the
    /// sheet listed it stays in iCloud.
    #[test]
    fn a_photo_changed_since_the_sheet_opened_stays() {
        let base: quince_core::photos::Photo = serde_json::from_value(json!({
            "id": "p", "master": "m", "takenAt": 100, "addedAt": null, "kind": "image",
            "width": 1, "height": 1, "duration": null, "filename": null, "size": null,
            "uti": null, "favorite": false, "hidden": false, "deleted": false,
            "deletedAt": null, "expungeAt": null, "title": null, "caption": null, "rating": 0,
            "keywords": [], "latitude": null, "longitude": null, "rendered": 0, "subtype": 0,
            "burst": null, "burstPick": false, "burstCount": 0, "liveSize": null,
            "changeTag": null
        }))
        .unwrap();
        assert!(still_may_leave(&base, false));
        assert!(!still_may_leave(&base, true), "in a Shared Library");
        for change in ["favorite", "hidden", "deleted"] {
            let mut v = serde_json::to_value(&base).unwrap();
            v[change] = json!(true);
            let p: quince_core::photos::Photo = serde_json::from_value(v).unwrap();
            assert!(!still_may_leave(&p, false), "{change}");
        }
        let mut undated = base.clone();
        undated.taken_at = 0;
        assert!(!still_may_leave(&undated, false), "no date");
    }

    /// Apple's checksum, as `sha1sum` makes it independently: base64(0x01 || SHA-1(salt ||
    /// bytes)). Proven live on 27 Nov 2026 against Apple's own for six files, 10 KB to 372 MB.
    #[test]
    fn apples_checksum_of_a_file() {
        let dir = std::env::temp_dir().join(format!("skykeep-ck-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f");
        std::fs::write(&path, b"Skykeep checksum test\n").unwrap();
        assert_eq!(
            checksum(&path, 22).as_deref(),
            Some("AToxVpkA9W6LtiWO0MAWAP87Zggm")
        );
        assert_eq!(checksum(&path, 21), None, "another size is not the file");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            checksum(&path, 0).as_deref(),
            Some("AW2cWT2PxzWixMKuuNMTHJ8CJ3sH")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn copy(part: &str, path: &str, size: i64) -> Copied {
        Copied {
            source: "photos".into(),
            item: "p".into(),
            part: part.into(),
            path: path.into(),
            size,
            version: None,
        }
    }

    fn master(original: (u64, &str), live: Option<(u64, &str)>) -> Value {
        let mut fields = json!({
            "resOriginalRes": { "value": { "size": original.0, "fileChecksum": original.1 } }
        });
        if let Some((size, sum)) = live {
            fields["resOriginalVidComplRes"] =
                json!({ "value": { "size": size, "fileChecksum": sum } });
        }
        json!({ "fields": fields })
    }

    /// A photo may leave only with every file Apple keeps in the backup, the same bytes; an
    /// edit, a missing Live video, a changed byte or a missing file keeps it in iCloud
    /// (icloudpd #614 #1312 #1403 #515).
    #[test]
    fn a_photo_leaves_only_when_every_file_is_the_same_as_apples() {
        let dir = std::env::temp_dir().join(format!("skykeep-judge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), b"Skykeep checksum test\n").unwrap();
        std::fs::write(dir.join("a.mov"), b"").unwrap();
        let (still, video) = (
            (22, "AToxVpkA9W6LtiWO0MAWAP87Zggm"),
            (0, "AW2cWT2PxzWixMKuuNMTHJ8CJ3sH"),
        );
        let plain = json!({ "fields": {} });
        let both = [copy(ORIGINAL, "a.jpg", 22), copy(LIVE, "a.mov", 0)];
        assert_eq!(
            judge(&dir, &both[..1], &plain, &master(still, None)),
            Verdict::Free
        );
        assert_eq!(
            judge(&dir, &both, &plain, &master(still, Some(video))),
            Verdict::Free
        );
        // The Live Photo's video not in the backup.
        assert_eq!(
            judge(&dir, &both[..1], &plain, &master(still, Some(video))),
            Verdict::NotSame
        );
        // Not edited, though Apple marks how almost every photo renders: free to leave.
        let unedited = json!({ "fields": {
            "adjustmentRenderType": { "value": 0 }, "adjustmentSourceType": { "value": 0 },
            "resJPEGThumbRes": { "value": {} } } });
        assert_eq!(
            judge(&dir, &both, &unedited, &master(still, None)),
            Verdict::Free
        );
        // Edited on a device.
        let edited = json!({ "fields": { "adjustmentType": { "value": "com.apple.photo" } } });
        assert_eq!(
            judge(&dir, &both, &edited, &master(still, None)),
            Verdict::Edited
        );
        // Other bytes than Apple's, though the same size.
        assert_eq!(
            judge(
                &dir,
                &both,
                &plain,
                &master((22, "AW2cWT2PxzWixMKuuNMTHJ8CJ3sH"), None)
            ),
            Verdict::NotSame
        );
        // The file gone from the drive.
        std::fs::remove_file(dir.join("a.jpg")).unwrap();
        assert_eq!(
            judge(&dir, &both, &plain, &master(still, None)),
            Verdict::NotSame
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
