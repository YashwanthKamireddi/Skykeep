//! What a name in the Windows folder stood for a moment ago: the Apple file whose placeholder
//! was deleted or renamed away from it (#265).
//!
//! Many Windows programs save by replacing the file, not by writing into it: a temporary file
//! renamed over it (MoveFileEx, Qt's QSaveFile, JetBrains), ReplaceFileW (.NET's File.Replace),
//! Word's dance (the original renamed to ~WRLnnnn.tmp, the new one renamed in, the original
//! deleted), or a delete and then a create. What is left at the name is a new, plain file: the
//! placeholder, and with it which Apple file this was, went with the file it replaced. Windows
//! says so as it happens (a delete or rename of the placeholder, with its identity and path),
//! and this keeps it, by path, for a short while. A plain file that then appears at the path is
//! that Apple file's new contents, written into it in place (`writes::send`): its id, shares and
//! version history kept, as the Linux folder does with a file written here taking another's
//! name (fuse.rs). Apple still refuses the write if another device changed the file meanwhile,
//! so a real conflict stays a conflicted copy.
//!
//! The Apple side of the delete or rename waits for the save to settle, and is not done at all
//! once a new file has taken the name (`settle`): that file is the Apple file now.

use quince_core::drive::DriveItem;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a vacated name is remembered. A replacing save puts its new file in place within
/// milliseconds; it is sent once left alone a couple of seconds (`writes::SETTLE`), and a send
/// that fails puts the name back for its next try (`put_back`).
pub const KEPT: Duration = Duration::from_secs(30);

/// What left a path: the Apple file its placeholder stood for (the version it held), and
/// where that placeholder went if it was renamed rather than deleted.
#[derive(Clone, Debug)]
pub struct Left {
    pub item: DriveItem,
    pub moved_to: Option<PathBuf>,
}

struct Entry {
    path: PathBuf,
    left: Left,
    at: Instant,
    /// Taken by the new file at the path (`take`): its send is under way or done.
    claimed: bool,
}

/// The most files kept in the record of what was read (`read`): the files programs have opened
/// lately, a few hundred bytes each. The oldest goes first.
pub const READ_MOST: usize = 4096;

/// Vacated paths, by path. Shared by the folder's callbacks (which fill it) and the watcher
/// sending what changes (which takes from it).
///
/// Beside them, the version of each file a program last read here, by path (`read`): what its
/// bytes on this computer are. Not every save that replaces a file is told of: a file renamed
/// over a placeholder, or put in its place by ReplaceFileW, takes it away with no delete or
/// rename notice (measured on Windows 11, #265). The new file at the path is still the next
/// version of the one that was read there, so that version is its base; Apple's check that its
/// file is still that version keeps a change made elsewhere a conflicted copy. Never the version
/// a later listing brought: a program holding the old bytes would write over that change.
#[derive(Default)]
pub struct Vacated {
    entries: Mutex<HashMap<String, Entry>>,
    read: Mutex<ReadRecord>,
}

#[derive(Default)]
struct ReadRecord {
    by_path: HashMap<String, (DriveItem, u64)>,
    next: u64,
}

/// Windows names are the same name in any case.
fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Whether two paths name the same place on Windows.
pub fn same_place(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

impl Vacated {
    fn entries(&self, now: Instant) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, e| now.saturating_duration_since(e.at) < KEPT);
        entries
    }

    /// `path` was left by `left.item`'s placeholder just now.
    pub fn leave(&self, path: &Path, left: Left, now: Instant) {
        self.entries(now).insert(
            key(path),
            Entry {
                path: path.to_path_buf(),
                left,
                at: now,
                claimed: false,
            },
        );
    }

    /// The Apple file a new file at `path` takes the place of, once: a second look finds it
    /// taken. Kept (as claimed) until it expires, so the delete or rename waiting on it knows.
    pub fn take(&self, path: &Path, now: Instant) -> Option<Left> {
        let mut entries = self.entries(now);
        let e = entries.get_mut(&key(path)).filter(|e| !e.claimed)?;
        e.claimed = true;
        Some(e.left.clone())
    }

    /// A send that took `path`'s entry failed: there again for its next try, kept afresh.
    pub fn put_back(&self, path: &Path, left: Left, now: Instant) {
        self.leave(path, left, now);
    }

    /// Whether Apple's file `id` may now be deleted or renamed as Windows said, the save it may
    /// have been part of settled. Not while any path it left has been taken (`taken`: a file
    /// is there now; or claimed by one already): that file is the Apple file now. When it may
    /// be, its entries go, so no file appearing later takes a file changed at Apple.
    pub fn settle(&self, id: &str, now: Instant, taken: impl Fn(&Path) -> bool) -> bool {
        let mut entries = self.entries(now);
        let mine = |e: &Entry| e.left.item.drivewsid == id;
        if entries
            .values()
            .any(|e| mine(e) && (e.claimed || taken(&e.path)))
        {
            return false;
        }
        entries.retain(|_, e| !mine(e));
        true
    }

    /// Whether Apple's file `id` left a path a moment ago (taken or not).
    pub fn holds(&self, id: &str, now: Instant) -> bool {
        self.entries(now)
            .values()
            .any(|e| e.left.item.drivewsid == id)
    }

    /// A program read `item`'s bytes at `path` (or Skykeep just put `item` there): the version
    /// the file at `path` now holds.
    pub fn read(&self, path: &Path, item: &DriveItem) {
        let mut r = self
            .read
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        r.next += 1;
        let at = r.next;
        r.by_path.insert(key(path), (item.clone(), at));
        if r.by_path.len() > READ_MOST {
            if let Some(oldest) = r
                .by_path
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(k, _)| k.clone())
            {
                r.by_path.remove(&oldest);
            }
        }
    }

    /// The version last read at `path`, if any.
    pub fn last_read(&self, path: &Path) -> Option<DriveItem> {
        self.read
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .by_path
            .get(&key(path))
            .map(|(item, _)| item.clone())
    }

    /// The file at `from` was deleted (`to` None) or renamed to `to`: what was read follows it.
    pub fn read_moved(&self, from: &Path, to: Option<&Path>) {
        let mut r = self
            .read
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(was) = r.by_path.remove(&key(from)) {
            if let Some(to) = to {
                r.by_path.insert(key(to), was);
            }
        }
    }

    /// `id`'s placeholder is back at `path` (a save undone part way): nothing waits there.
    pub fn forget(&self, path: &Path, id: &str, now: Instant) {
        let mut entries = self.entries(now);
        if entries
            .get(&key(path))
            .is_some_and(|e| e.left.item.drivewsid == id && !e.claimed)
        {
            entries.remove(&key(path));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What was read at a path is its base until the file is deleted or renamed away, follows
    /// a rename, is the newest of two reads, and the record never grows past READ_MOST (#265).
    #[test]
    fn what_was_read_is_kept_by_path_bounded() {
        let v = Vacated::default();
        let p = Path::new(r"C:\Drive\Report.docx");
        assert!(v.last_read(p).is_none());
        v.read(p, &item("A"));
        v.read(&PathBuf::from(r"c:\drive\REPORT.docx"), &item("B"));
        assert_eq!(
            v.last_read(p).unwrap().drivewsid,
            "FILE::B",
            "the newest, any case"
        );
        let to = Path::new(r"C:\Drive\Old.docx");
        v.read_moved(p, Some(to));
        assert!(
            v.last_read(p).is_none(),
            "renamed away: nothing read here now"
        );
        assert_eq!(v.last_read(to).unwrap().drivewsid, "FILE::B");
        v.read_moved(to, None);
        assert!(v.last_read(to).is_none(), "deleted");
        for n in 0..READ_MOST + 10 {
            v.read(&PathBuf::from(format!(r"C:\Drive\{n}.txt")), &item("X"));
        }
        assert_eq!(v.read.lock().unwrap().by_path.len(), READ_MOST);
        assert!(
            v.last_read(Path::new(r"C:\Drive\0.txt")).is_none(),
            "the oldest went"
        );
        assert!(v
            .last_read(&PathBuf::from(format!(r"C:\Drive\{}.txt", READ_MOST + 9)))
            .is_some());
    }

    fn item(id: &str) -> DriveItem {
        DriveItem {
            drivewsid: format!("FILE::{id}"),
            parent_id: Some("FOLDER::root".into()),
            name: "Report".into(),
            extension: Some("docx".into()),
            kind: "FILE".into(),
            size: Some(5),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: Some("e1".into()),
            share: None,
        }
    }

    fn left(id: &str, moved_to: Option<&str>) -> Left {
        Left {
            item: item(id),
            moved_to: moved_to.map(PathBuf::from),
        }
    }

    const P: &str = r"C:\Users\a\iCloud Drive\Report.docx";

    #[test]
    fn a_vacated_name_is_taken_once_in_any_case() {
        let v = Vacated::default();
        let now = Instant::now();
        v.leave(Path::new(P), left("rep", None), now);
        let upper = PathBuf::from(P.to_uppercase());
        let got = v.take(&upper, now).expect("the same name in another case");
        assert_eq!(got.item.drivewsid, "FILE::rep");
        assert!(v.take(Path::new(P), now).is_none(), "taken once");
        assert!(v.holds("FILE::rep", now), "kept, claimed, until it expires");
    }

    #[test]
    fn a_vacated_name_is_forgotten_after_a_while() {
        let v = Vacated::default();
        let then = Instant::now();
        v.leave(Path::new(P), left("rep", None), then);
        let later = then + KEPT + Duration::from_millis(1);
        assert!(v.take(Path::new(P), later).is_none());
        assert!(!v.holds("FILE::rep", later));
        assert!(
            v.settle("FILE::rep", later, |_| true),
            "nothing holds it now"
        );
    }

    #[test]
    fn a_failed_send_puts_the_name_back_for_its_next_try() {
        let v = Vacated::default();
        let then = Instant::now();
        v.leave(Path::new(P), left("rep", None), then);
        let taken = v.take(Path::new(P), then).unwrap();
        let retry = then + KEPT - Duration::from_secs(1);
        v.put_back(Path::new(P), taken, retry);
        // Past the first entry's time, within the second's.
        let next = then + KEPT + Duration::from_secs(5);
        assert!(v.take(Path::new(P), next).is_some());
    }

    /// Word's save: the original renamed to ~WRL0001.tmp (its name vacated), the new file put
    /// at the name, ~WRL0001.tmp deleted. The delete is never done at Apple: the name is taken.
    #[test]
    fn apple_is_changed_only_if_nothing_took_the_name() {
        let v = Vacated::default();
        let now = Instant::now();
        let moved = r"C:\Users\a\iCloud Drive\~WRL0001.tmp";
        v.leave(Path::new(P), left("rep", Some(moved)), now);
        // The new file is at the name, not yet sent.
        assert!(!v.settle("FILE::rep", now, |p| same_place(p, Path::new(P))));
        // Sent: claimed, and the file gone again changes nothing.
        let got = v.take(Path::new(P), now).unwrap();
        assert_eq!(got.moved_to.as_deref(), Some(Path::new(moved)));
        assert!(!v.settle("FILE::rep", now, |_| false));
        // Another Apple file is not held by it.
        assert!(v.settle("FILE::other", now, |_| true));
    }

    /// A plain delete or rename: nothing took the name, so Apple is changed, and the name is
    /// let go so a file made there later is a new file, never written into the changed one.
    #[test]
    fn a_plain_delete_lets_the_name_go() {
        let v = Vacated::default();
        let now = Instant::now();
        v.leave(Path::new(P), left("rep", None), now);
        assert!(v.settle("FILE::rep", now, |_| false));
        assert!(v.take(Path::new(P), now).is_none());
        assert!(!v.holds("FILE::rep", now));
    }

    #[test]
    fn a_placeholder_back_at_its_name_is_waited_for_no_more() {
        let v = Vacated::default();
        let now = Instant::now();
        v.leave(Path::new(P), left("rep", None), now);
        v.forget(Path::new(P), "FILE::other", now);
        assert!(v.holds("FILE::rep", now), "only its own placeholder");
        v.forget(Path::new(P), "FILE::rep", now);
        assert!(!v.holds("FILE::rep", now));
        assert!(v.take(Path::new(P), now).is_none());
    }
}
