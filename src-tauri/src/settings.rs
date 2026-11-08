//! What Settings reads of this computer: how much room the local copy takes, the pictures
//! that can be let go of because Apple still has them, and whether this desktop shows alerts.
//!
//! Sizes are the disk the files really take — the Drive folder's cache is sparse, and a video
//! opened in another app is a hard link to its cached file — so the figure is what clearing
//! would give back, not what the files claim to be.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The local copy, by what it holds, in bytes on disk.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalStorage {
    /// Every record: calendars, reminders, notes' text, contacts, the Drive and Photos lists.
    pub library: u64,
    /// Photos' pictures and videos, and photos waiting to go up from the Photos folder.
    pub photos: u64,
    /// Notes' pictures and attachments.
    pub notes: u64,
    /// The Drive folder's downloads and what waits to go up from it, and file thumbnails.
    pub drive: u64,
    /// Everything else here: the window's own cache, other accounts set aside.
    pub other: u64,
    pub total: u64,
    /// What "Clear Cached Pictures" would give back.
    pub clearable: u64,
}

/// What belongs to each part, by name in the data folder.
const LIBRARY: [&str; 3] = ["library.db", "library.db-wal", "library.db-shm"];
const PHOTOS: [&str; 3] = ["photos", "photos-folder-cache", "photos-folder-writes"];
const NOTES: [&str; 2] = ["note-media", "note-files"];
const DRIVE: [&str; 3] = ["drive-cache", "drive-writes", "thumbnails"];

/// Pictures Apple still has, made or fetched for showing: let go of, each comes back when it
/// is next wanted. Photos' grid thumbnails are not among them (the grid is drawn from them),
/// nor is the Drive folder's cache (it holds what was chosen to Keep on This Computer), nor
/// anything waiting to be sent.
const CLEARABLE: [&str; 3] = ["photos", "note-media", "note-files"];
/// Drive's file thumbnails: wholly a cache.
const CLEARABLE_WHOLE: [&str; 1] = ["thumbnails"];

/// A file's identity, so one held under two names (a hard link) is counted once.
type Seen = HashSet<(u64, u64)>;

/// The room a file takes on disk, or nothing when it was counted already under another name.
fn on_disk(meta: &std::fs::Metadata, seen: &mut Seen) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() > 1 && !seen.insert((meta.dev(), meta.ino())) {
            return 0;
        }
        meta.blocks().saturating_mul(512)
    }
    #[cfg(not(unix))]
    {
        let _ = seen;
        meta.len()
    }
}

/// Everything under `path` (never following a link out of it), in bytes on disk.
fn size_of(path: &Path, seen: &mut Seen) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return if meta.is_file() {
            on_disk(&meta, seen)
        } else {
            0
        };
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| size_of(&e.path(), seen))
        .sum()
}

/// Every file under `path`, a download still being written (`.part`) left out.
fn files_under(path: &Path, out: &mut Vec<PathBuf>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_file() {
        if path.extension().is_none_or(|e| e != "part") {
            out.push(path.to_path_buf());
        }
        return;
    }
    if meta.is_dir() {
        for e in std::fs::read_dir(path).into_iter().flatten().flatten() {
            files_under(&e.path(), out);
        }
    }
}

/// The files "Clear Cached Pictures" removes, under the data folder `dir`.
fn clearable_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for name in CLEARABLE {
        let Ok(entries) = std::fs::read_dir(dir.join(name)) else {
            continue;
        };
        for e in entries.flatten() {
            // Photos keeps its pictures a folder per size; the grid's thumbnails stay.
            let photos_thumbs = name == "photos" && e.file_name() == "thumb";
            // A file loose at the head of photos/ is none of Photos' sizes: not known, so kept.
            let loose = name == "photos" && e.path().is_file();
            if !photos_thumbs && !loose {
                files_under(&e.path(), &mut out);
            }
        }
    }
    for name in CLEARABLE_WHOLE {
        files_under(&dir.join(name), &mut out);
    }
    out
}

/// How the data folder `dir` is used.
pub fn measure(dir: &Path) -> LocalStorage {
    let mut seen = Seen::new();
    let mut part =
        |names: &[&str]| -> u64 { names.iter().map(|n| size_of(&dir.join(n), &mut seen)).sum() };
    let library = part(&LIBRARY);
    let photos = part(&PHOTOS);
    let notes = part(&NOTES);
    let drive = part(&DRIVE);
    let named: Vec<&str> = [&LIBRARY[..], &PHOTOS, &NOTES, &DRIVE].concat();
    let rest: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !named.contains(&n.as_str()))
        .collect();
    let other = rest.iter().map(|n| size_of(&dir.join(n), &mut seen)).sum();
    let mut seen = Seen::new();
    let clearable = clearable_files(dir)
        .iter()
        .filter_map(|p| std::fs::symlink_metadata(p).ok())
        .map(|m| on_disk(&m, &mut seen))
        .sum();
    LocalStorage {
        library,
        photos,
        notes,
        drive,
        other,
        total: library + photos + notes + drive + other,
        clearable,
    }
}

/// Remove the cached pictures under `dir` through `remove` (which may decline a file in use),
/// answering how the folder is used after. Files only: the folders stay, so a picture being
/// saved this moment never finds its folder gone.
pub fn clear(dir: &Path, remove: &dyn Fn(&Path)) -> LocalStorage {
    for f in clearable_files(dir) {
        remove(&f);
    }
    measure(dir)
}

/// How much room the local copy takes on this computer. Walked off the main thread.
#[tauri::command]
pub async fn local_storage(app: tauri::AppHandle) -> Result<LocalStorage, String> {
    let dir = crate::state_dir(&app);
    tauri::async_runtime::spawn_blocking(move || measure(&dir))
        .await
        .map_err(|e| e.to_string())
}

/// Let go of the pictures Apple still has (see `CLEARABLE`); answers how the copy is used now.
/// A photo being read this moment, or waiting to be uploaded, stays (see `photo_cache`).
#[tauri::command]
pub async fn local_storage_clear(app: tauri::AppHandle) -> Result<LocalStorage, String> {
    let dir = crate::state_dir(&app);
    let waiting: Vec<PathBuf> = {
        use tauri::Manager;
        app.state::<crate::Data>()
            .0
            .lock()
            .await
            .unfinished_uploads()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|u| PathBuf::from(u.path))
            .collect()
    };
    tauri::async_runtime::spawn_blocking(move || {
        let protected = crate::photo_cache::identities(&waiting);
        crate::photo_cache::clearing(&app, || {
            clear(&dir, &|f| {
                crate::photo_cache::remove_unless_in_use(f, &protected);
            })
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Whether this desktop shows alerts: on Linux an alert is shown by a notification service,
/// and a desktop without one (a bare window manager) drops every alert silently. The question
/// is the one an alert asks first — the same D-Bus service, started on demand as an alert would
/// start it. `None` where the system always shows them, or the answer did not come in time.
#[tauri::command]
pub async fn alerts_shown() -> Option<bool> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let ask =
            tauri::async_runtime::spawn_blocking(|| notify_rust::get_server_information().is_ok());
        tokio::time::timeout(std::time::Duration::from_secs(4), ask)
            .await
            .ok()
            .and_then(Result::ok)
    }
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(dir: &Path, rel: &str, bytes: usize) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![7u8; bytes]).unwrap();
    }

    /// A data folder as the app leaves it: the library, every cache, things waiting to go up.
    fn folder() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        put(d, "library.db", 40_000);
        put(d, "library.db-wal", 8_000);
        put(d, "photos/thumb/A", 10_000);
        put(d, "photos/medium/A", 50_000);
        put(d, "photos/original/V", 200_000);
        put(d, "photos/fit/512/A", 20_000);
        put(d, "photos/full/B.part", 30_000);
        put(d, "photos/LOOSE", 5_000);
        put(d, "photos-folder-writes/outgoing/h/IMG.HEIC", 60_000);
        put(d, "note-media/n1-preview", 12_000);
        put(d, "note-files/n1/Scan.pdf", 12_000);
        put(d, "drive-cache/FILE__x", 16_000);
        put(d, "drive-cache/.kept.json", 100);
        put(d, "drive-writes/journal", 4_000);
        put(d, "thumbnails/F-1", 3_000);
        put(d, "WebKitCache/blob", 9_000);
        put(d, "accounts/abc/library.db", 9_000);
        t
    }

    #[test]
    fn the_parts_add_up_to_the_whole() {
        let t = folder();
        let s = measure(t.path());
        assert!(s.library >= 48_000 && s.photos >= 375_000 && s.notes >= 24_000);
        assert!(s.drive >= 23_000 && s.other >= 18_000);
        assert_eq!(s.total, s.library + s.photos + s.notes + s.drive + s.other);
        assert!(s.clearable > 0 && s.clearable < s.total);
    }

    #[test]
    fn clearing_keeps_the_library_thumbnails_and_what_waits_to_go_up() {
        let t = folder();
        let d = t.path();
        let before = measure(d);
        let after = clear(d, &|f| {
            let _ = std::fs::remove_file(f);
        });
        for kept in [
            "library.db",
            "library.db-wal",
            "photos/thumb/A",
            "photos/full/B.part",
            "photos/LOOSE",
            "photos-folder-writes/outgoing/h/IMG.HEIC",
            "drive-cache/FILE__x",
            "drive-cache/.kept.json",
            "drive-writes/journal",
            "WebKitCache/blob",
            "accounts/abc/library.db",
        ] {
            assert!(d.join(kept).exists(), "{kept} was kept");
        }
        for gone in [
            "photos/medium/A",
            "photos/original/V",
            "photos/fit/512/A",
            "note-media/n1-preview",
            "note-files/n1/Scan.pdf",
            "thumbnails/F-1",
        ] {
            assert!(!d.join(gone).exists(), "{gone} was cleared");
        }
        // The folders stay, for a picture being saved this moment.
        assert!(d.join("photos/medium").is_dir() && d.join("note-media").is_dir());
        assert_eq!(after.clearable, 0);
        assert_eq!(before.total - after.total, before.clearable);
        assert_eq!(after.library, before.library);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_under_two_names_is_counted_once() {
        let t = folder();
        let d = t.path();
        std::fs::create_dir_all(d.join("photos/open/V")).unwrap();
        let before = measure(d);
        std::fs::hard_link(
            d.join("photos/original/V"),
            d.join("photos/open/V/Clip.mov"),
        )
        .unwrap();
        let linked = measure(d);
        assert_eq!(linked.total, before.total);
        assert_eq!(linked.clearable, before.clearable);
        clear(d, &|f| {
            let _ = std::fs::remove_file(f);
        });
        assert!(!d.join("photos/open/V/Clip.mov").exists());
    }

    #[test]
    fn an_empty_or_missing_folder_is_nothing() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(measure(t.path()), LocalStorage::default());
        assert_eq!(measure(&t.path().join("none")), LocalStorage::default());
        assert_eq!(
            clear(&t.path().join("none"), &|_| {}),
            LocalStorage::default()
        );
    }
}
