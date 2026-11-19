//! Room kept on the disk: files File Explorer brought down from iCloud given back to iCloud
//! (dehydrated) when the disk runs low, as OneDrive's Files On-Demand does.
//!
//! A file read out of the folder stays downloaded in it. Nothing gave those bytes back but a
//! person choosing "Free up space", so copying 50 GB out of iCloud Drive needed 100 GB free: 50
//! where it was copied and 50 kept here, and the copy failed part way when the disk filled.
//!
//! Only a file that is downloaded, in sync with iCloud, not kept on the device by choice
//! ("Always keep on this device"), and left alone for a while is given back, the least recently
//! used first, and never under a program that has it open: it is taken with an oplock
//! (`CfOpenFileWithOplock`, exclusive), which Microsoft recommends over a share-nothing handle
//! so a program that opens the file meanwhile wins and never sees "in use" (CfUpdatePlaceholder,
//! FileHandle). Only folders Windows has already listed are looked in, so the guard never
//! makes Windows ask Apple for a folder. The file stays listed, and comes back when opened.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use windows::Win32::Storage::CloudFilters::*;

const PINNED: u32 = 0x0008_0000;
/// FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: some or all of the bytes are not here.
const NOT_ALL_HERE: u32 = 0x0040_0000;
const GIB: u64 = 1 << 30;

/// Room is made when the drive has less free than this: the smaller of a tenth of it and
/// 10 GB, so a small drive is not emptied for a large one's figure.
#[must_use]
pub fn low_mark(total: u64) -> u64 {
    (total / 10).min(10 * GIB)
}

/// How much room making room aims for: the low mark and 5 GB more, so it is not made again a
/// moment later.
#[must_use]
pub fn wanted(total: u64) -> u64 {
    low_mark(total).saturating_add(5 * GIB)
}

/// A file used this recently is never given back: it may well be opened again at once.
pub const RECENT: Duration = Duration::from_secs(10 * 60);

/// Free and total bytes of the drive `path` is on.
#[must_use]
pub fn space(path: &Path) -> Option<(u64, u64)> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: a path string alive for the call, and two out values.
    unsafe {
        GetDiskFreeSpaceExW(
            &crate::cloud::long_path(path),
            Some(&mut free),
            Some(&mut total),
            None,
        )
    }
    .ok()?;
    Some((free, total))
}

/// A file that may be given back: downloaded, in sync, not kept by choice, and when it was
/// last used. `None` for anything else.
fn candidate(path: &Path, now: SystemTime, recent: Duration) -> Option<(SystemTime, u64)> {
    use std::os::windows::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let attrs = meta.file_attributes();
    if attrs & (PINNED | NOT_ALL_HERE) != 0 || meta.len() == 0 {
        return None;
    }
    let used = [meta.accessed().ok(), meta.modified().ok()]
        .into_iter()
        .flatten()
        .max()?;
    if now.duration_since(used).unwrap_or_default() < recent {
        return None;
    }
    let in_sync =
        crate::cloud::placeholder_state(path).is_some_and(|(s, _)| s == CF_IN_SYNC_STATE_IN_SYNC);
    in_sync.then_some((used, meta.len()))
}

/// The bytes of `path` given back to iCloud, only while no program has it open. False when
/// one does (or it could not be done): it is left as it is.
#[must_use]
pub fn give_back(path: &Path) -> bool {
    // SAFETY: a path string alive for the call; the handle is closed below.
    let Ok(handle) = (unsafe {
        CfOpenFileWithOplock(
            &crate::cloud::long_path(path),
            CF_OPEN_FILE_FLAG_EXCLUSIVE | CF_OPEN_FILE_FLAG_WRITE_ACCESS,
        )
    }) else {
        return false;
    };
    // SAFETY: the protected handle just opened; the whole file (length -1).
    let done = unsafe { CfDehydratePlaceholder(handle, 0, -1, CF_DEHYDRATE_FLAG_NONE, None) };
    // SAFETY: closed once, as CfOpenFileWithOplock asks.
    unsafe { CfCloseHandle(handle) };
    done.is_ok()
}

/// Room made in `folders` (the folders Windows has listed), the least recently used files
/// given back first, until `free` and what was given back reach `want`. Answers the bytes
/// given back.
pub fn make_room(folders: &[PathBuf], free: u64, want: u64) -> u64 {
    make_room_sparing(folders, free, want, RECENT)
}

/// [`make_room`], sparing files used within `recent`.
pub fn make_room_sparing(folders: &[PathBuf], free: u64, want: u64, recent: Duration) -> u64 {
    if free >= want {
        return 0;
    }
    let now = SystemTime::now();
    let mut files: Vec<(SystemTime, u64, PathBuf)> = folders
        .iter()
        .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter_map(|p| candidate(&p, now, recent).map(|(used, len)| (used, len, p)))
        .collect();
    files.sort_by_key(|(used, _, _)| *used);
    let mut given = 0u64;
    for (_, len, path) in files {
        if free.saturating_add(given) >= want {
            break;
        }
        if give_back(&path) {
            given = given.saturating_add(len);
        }
    }
    given
}

/// Watches the drive `root` is on until `stop` is set: a minute after it starts, then every
/// five, room is made in `listed()`'s folders when the drive is low.
pub fn guard(
    root: PathBuf,
    listed: impl Fn() -> Vec<PathBuf> + Send + 'static,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Option<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("quince:cloud-room".into())
        .spawn(move || {
            let mut wait = Duration::from_secs(60);
            loop {
                // In short steps, so a folder that stops is not held up.
                let mut slept = Duration::ZERO;
                while slept < wait {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                    slept += Duration::from_secs(1);
                }
                wait = Duration::from_secs(5 * 60);
                let Some((free, total)) = space(&root) else {
                    continue;
                };
                if free >= low_mark(total) {
                    continue;
                }
                let given = make_room(&listed(), free, wanted(total));
                crate::note(&format!(
                    "the disk was low ({} MB free): {} MB of iCloud files given back, still listed",
                    free >> 20,
                    given >> 20
                ));
            }
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small drive's low mark is a tenth of it, a large one's 10 GB; room is made to 5 GB
    /// past it.
    #[test]
    fn the_low_mark_fits_the_drive() {
        assert_eq!(low_mark(64 * GIB), 64 * GIB / 10);
        assert_eq!(low_mark(1024 * GIB), 10 * GIB);
        assert_eq!(wanted(1024 * GIB), 15 * GIB);
    }
}
