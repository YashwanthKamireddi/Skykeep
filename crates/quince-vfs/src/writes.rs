//! A file's own bytes written through the folder: kept on this computer first, then sent to
//! Apple behind the app writing them.
//!
//! An app saving a file never waits on the network. What it writes goes into a private copy;
//! a few seconds after the last program closes it, the copy goes to Apple. A file saved over
//! gets its new contents in place — the same file at Apple, its id, shares and version history
//! kept, so an app on another device that follows it still has it (#24) — and a file changed
//! on another device meanwhile is never overwritten: Apple refuses a write over a version that
//! is no longer its own, and this computer's version is kept beside it as a conflicted copy.
//! What is waiting to go is written down, so a restart, a crash or an hour offline loses
//! nothing: it goes when it can.

use crate::Remote;
use quince_core::drive::DriveItem;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// How long a file must be left alone after its last writer closes before it is sent: one
/// save is often several writes and closes, and a scratch file often lives a second.
pub const SETTLE: Duration = Duration::from_secs(2);

/// One file written through the folder and not yet at Apple as written.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    /// The folder it goes into (Apple's id).
    pub folder: String,
    /// Its name, extension and all, as the folder shows it.
    pub name: String,
    /// The Apple file it will replace, as it was when this computer's copy was taken: its
    /// version tells whether it changed elsewhere since. `None` for a new file.
    pub replaces: Option<DriveItem>,
    /// Programs that have it open now (not kept across restarts).
    #[serde(skip)]
    pub open: u32,
    /// Written since it was last sent.
    pub dirty: bool,
    /// Bumped on every write: a send is only counted done for the bytes it sent.
    #[serde(skip)]
    pub generation: u64,
    pub modified: SystemTime,
    /// A send of it is waiting or under way (one at a time; not kept across restarts).
    #[serde(skip)]
    pub sender: bool,
    /// Since when nothing has touched it: it is sent once left alone (SETTLE).
    #[serde(skip)]
    pub quiet_since: Option<std::time::Instant>,
    /// On its way to Apple right now.
    #[serde(skip)]
    pub sending: bool,
    /// Why the last try did not go, said once it has failed (cleared when it goes).
    #[serde(skip)]
    pub failure: Option<String>,
}

/// What is waiting to go, as a person is told it: how many files, how many on their way now,
/// and the reason one would not go, if one would not.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub files: u32,
    pub sending: u32,
    pub failure: Option<String>,
}

/// Names programs use for their own scratch files, which never leave this computer: Vim's
/// swap files and its "4913" probe, Emacs's locks and backups, LibreOffice's lock files,
/// GLib's atomic-save temporaries, and anything ending "~".
#[must_use]
pub fn stays_local(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name == "4913"
        || name.ends_with('~')
        || name.starts_with(".#")
        || name.starts_with(".~lock.")
        || name.starts_with(".goutputstream-")
        || [".swp", ".swo", ".swx", ".tmp", ".part", ".crdownload"]
            .iter()
            .any(|s| lower.ends_with(s))
        || (name.starts_with('#') && name.ends_with('#'))
}

/// A conflicted copy's name, as Dropbox and OneDrive name theirs: "Report (conflicted copy
/// 2026-10-20 18:40).docx" — beside the other device's version, never over it.
#[must_use]
pub fn conflicted(name: &str, when: SystemTime) -> String {
    let secs = when
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days since 1970 as a calendar date (UTC), without a date crate for one label.
    let (y, m, d) = civil(i64::try_from(days).unwrap_or(0));
    let stamp = format!(
        "{y:04}-{m:02}-{d:02} {:02}.{:02}",
        rest / 3600,
        (rest % 3600) / 60
    );
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} (conflicted copy {stamp}).{ext}"),
        _ => format!("{name} (conflicted copy {stamp})"),
    }
}

/// Days since 1970-01-01 as (year, month, day): Howard Hinnant's civil_from_days.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// What is waiting to go, by inode, with its bytes in a folder of its own on disk.
pub struct Writes {
    dir: PathBuf,
    pub(crate) items: Mutex<HashMap<u64, Pending>>,
    /// What this computer last sent under each name ("folder/name"): the next save of that
    /// name replaces it as its own, never mistaking it for a change made elsewhere.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) sent: Mutex<HashMap<String, DriveItem>>,
    /// One send at a time per name ("folder/name"): a replace is three steps at Apple (send,
    /// old into Recently Deleted, rename), and a second save of the same name arriving
    /// between them took the name and left the first as "name 2" — measured live.
    lanes: Mutex<HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

impl Writes {
    /// # Errors
    /// When the folder cannot be made.
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            items: Mutex::new(HashMap::new()),
            sent: Mutex::new(HashMap::new()),
            lanes: Mutex::new(HashMap::new()),
        })
    }

    /// Where a pending file's bytes are: a folder of its own, the file named as it will be
    /// sent (Apple names an upload after the file).
    #[must_use]
    pub fn staging(&self, ino: u64, name: &str) -> PathBuf {
        self.dir.join(ino.to_string()).join(name)
    }

    /// The bytes moved when the file is renamed before it is sent.
    ///
    /// # Errors
    /// When the disk refuses.
    pub fn rename_staging(&self, ino: u64, from: &str, to: &str) -> std::io::Result<()> {
        if from == to {
            return Ok(());
        }
        std::fs::rename(self.staging(ino, from), self.staging(ino, to))
    }

    /// The bytes let go of, once sent or given up.
    pub fn drop_staging(&self, ino: u64) {
        let _ = std::fs::remove_dir_all(self.dir.join(ino.to_string()));
    }

    /// What is waiting to go: scratch files, which never go, are not counted.
    #[must_use]
    pub fn waiting(&self) -> Waiting {
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut w = Waiting::default();
        for p in items.values().filter(|p| p.dirty && !stays_local(&p.name)) {
            w.files += 1;
            w.sending += u32::from(p.sending);
            if w.failure.is_none() {
                w.failure.clone_from(&p.failure);
            }
        }
        w
    }

    /// The lane sends under one name wait in, one after another.
    pub fn lane(&self, key: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
        let mut lanes = self
            .lanes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::sync::Arc::clone(lanes.entry(key.to_string()).or_default())
    }

    fn journal(&self) -> PathBuf {
        self.dir.join("pending.json")
    }

    /// What is waiting written down, whole or not at all, so a restart finds it.
    pub fn save(&self) {
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keep: Vec<(u64, &Pending)> = items
            .iter()
            .filter(|(_, p)| p.dirty && !stays_local(&p.name))
            .map(|(i, p)| (*i, p))
            .collect();
        if let Ok(json) = serde_json::to_vec(&keep) {
            // Each waiting file's bytes on the disk before the journal names them, and the
            // journal on the disk before it replaces the last: after a power cut, a journal
            // naming bytes the disk never kept would send an empty file over the real one.
            for (ino, p) in &keep {
                if let Ok(f) = std::fs::File::open(self.staging(*ino, &p.name)) {
                    let _ = f.sync_all();
                }
            }
            let part = self.journal().with_extension("json.part");
            let written = std::fs::File::create(&part).and_then(|mut f| {
                std::io::Write::write_all(&mut f, &json)?;
                f.sync_all()
            });
            if written.is_ok() {
                let _ = std::fs::rename(&part, self.journal());
            }
        }
    }

    /// Where a waiting file's bytes are held while every waiting file moves to its new inode:
    /// apart from every inode's own folder, so no move lands on another's.
    fn held(&self, old: u64) -> PathBuf {
        self.dir.join(".restore").join(old.to_string())
    }

    /// What was waiting when the folder last went away, moved to the inodes it has now —
    /// `place` gives each one its new number.
    ///
    /// Inode numbers start again from the same place at every mount, so a file's old number
    /// is often another's new one, or its own: moved straight across, a file whose new number
    /// was its old one had its folder emptied by the clean-up after the move, and two files
    /// trading numbers had one's bytes laid over or deleted with the other's — a file saved
    /// offline and carried over two restarts was lost. Every one is first set aside, out of
    /// the way of all numbers, then placed; one set aside when the app stopped in between is
    /// found there again.
    ///
    /// # Errors
    /// When the disk refuses a move. Nothing is deleted before its bytes are where they go.
    pub fn take_back(
        &self,
        mut place: impl FnMut(&Pending) -> u64,
    ) -> std::io::Result<Vec<(u64, Pending)>> {
        // Set aside by a start that stopped half-way: back where the journal says they are.
        if let Ok(entries) = std::fs::read_dir(self.dir.join(".restore")) {
            for entry in entries.flatten() {
                let Some(old) = entry
                    .file_name()
                    .to_str()
                    .and_then(|n| n.parse::<u64>().ok())
                else {
                    continue;
                };
                let home = self.dir.join(old.to_string());
                if !home.exists() {
                    std::fs::rename(entry.path(), &home)?;
                }
            }
        }
        let waiting = self.load();
        std::fs::create_dir_all(self.dir.join(".restore"))?;
        for (old, _) in &waiting {
            // A copy left set aside beside the one the journal names (load() saw it at home)
            // is the older of the two.
            if self.held(*old).exists() {
                std::fs::remove_dir_all(self.held(*old))?;
            }
            std::fs::rename(self.dir.join(old.to_string()), self.held(*old))?;
        }
        let mut placed = Vec::with_capacity(waiting.len());
        for (old, pending) in waiting {
            let ino = place(&pending);
            let to = self.staging(ino, &pending.name);
            if let Some(dir) = to.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::rename(self.held(old).join(&pending.name), &to)?;
            let _ = std::fs::remove_dir_all(self.held(old));
            placed.push((ino, pending));
        }
        Ok(placed)
    }

    /// What was waiting when the folder last went away, with where its bytes are.
    #[must_use]
    pub fn load(&self) -> Vec<(u64, Pending)> {
        std::fs::read(self.journal())
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<(u64, Pending)>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|(ino, p)| self.staging(*ino, &p.name).exists())
            .collect()
    }

    /// What waits under `key`, as it stands. For a front end whose keys stay the same from one
    /// run to the next (the phone's Files app keys each file by its document), unlike the
    /// folder's inodes.
    #[must_use]
    pub fn get(&self, key: u64) -> Option<Pending> {
        self.items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&key)
            .cloned()
    }

    /// Every key with something waiting to go.
    #[must_use]
    pub fn keys(&self) -> Vec<u64> {
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut keys: Vec<u64> = items
            .iter()
            .filter(|(_, p)| p.dirty)
            .map(|(k, _)| *k)
            .collect();
        keys.sort_unstable();
        keys
    }

    /// Held to go, or changed where it is held (`change` is given what is there, if anything),
    /// and written down at once: once this returns, a crash or a restart still sends it.
    pub fn hold(&self, key: u64, change: impl FnOnce(Option<Pending>) -> Pending) -> Pending {
        let now = {
            let mut items = self
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let now = change(items.remove(&key));
            items.insert(key, now.clone());
            now
        };
        self.save();
        now
    }

    /// No longer waiting (sent, or given up), written down, and its bytes let go of.
    pub fn release(&self, key: u64) {
        self.items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&key);
        self.save();
        self.drop_staging(key);
    }

    /// What the journal kept taken back under the same keys, for a front end whose keys are
    /// stable (see [`Writes::get`]); the folder, whose inodes are not, uses
    /// [`Writes::take_back`]. Answers how many files are waiting.
    pub fn resume(&self) -> usize {
        let kept = self.load();
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (key, pending) in kept {
            items.entry(key).or_insert(pending);
        }
        items.values().filter(|p| p.dirty).count()
    }
}

/// How sending one held file went ([`send_held`]).
pub enum Held {
    /// Nothing waits under that key.
    Nothing,
    /// An app still has it open: sent once it closes it.
    Open,
    /// At Apple. `settled`: nothing wrote to it while it went, and it waits no more; else what
    /// was written meanwhile waits, to go as the next version of what was just sent.
    Sent { sent: Box<Sent>, settled: bool },
}

/// One held file sent ([`Writes::hold`]): against the version of Apple's file its copy was
/// taken from when it was opened (`Pending::replaces`), never the version the library has by
/// the time it goes, so a change made on another device in between is kept and this one goes
/// beside it as a conflicted copy (see [`send`]). Once settled, its bytes are moved to
/// `keep_to` (the front end's own copy of the file, current now) or let go of.
///
/// # Errors
/// When Apple cannot be reached or refuses: the file still waits, to be sent again.
pub async fn send_held(
    remote: &dyn Remote,
    writes: &Writes,
    key: u64,
    folder: &DriveItem,
    keep_to: Option<&Path>,
) -> Result<Held, String> {
    let Some(pending) = writes.get(key) else {
        return Ok(Held::Nothing);
    };
    if pending.open > 0 {
        return Ok(Held::Open);
    }
    let lane = writes.lane(&format!("{}/{}", pending.folder, pending.name));
    let _turn = lane.lock().await;
    let staged = writes.staging(key, &pending.name);
    let stamp = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    let before = (stamp(&staged), pending.generation);
    let sent = send(remote, folder, &staged, pending.replaces.as_ref(), None).await?;
    // Let go of while it went (the account signed out): nothing more to keep.
    let Some(now) = writes.get(key) else {
        return Ok(Held::Sent {
            sent: Box::new(sent),
            settled: true,
        });
    };
    let settled = now.open == 0 && (stamp(&staged), now.generation) == before;
    if settled {
        // A conflicted copy is another file than the one kept at `keep_to`: never put in its place.
        if let Some(to) = keep_to.filter(|_| !sent.conflicted) {
            if let Some(dir) = to.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::rename(&staged, to);
        }
        writes.release(key);
    } else {
        // Written again while it went: the next send replaces what just went, which is this
        // phone's own. A conflicted copy is not the file the app has open, so what comes next
        // is checked against the file by its name again, never written over it.
        let item = (!sent.conflicted).then(|| sent.item.clone());
        writes.hold(key, |p| Pending {
            replaces: item,
            ..p.unwrap_or(pending)
        });
    }
    Ok(Held::Sent {
        sent: Box::new(sent),
        settled,
    })
}

/// How a send ended: the file as Apple now has it, and whether it went as a conflicted copy.
pub struct Sent {
    pub item: DriveItem,
    pub conflicted: bool,
}

/// One pending file sent to Apple (see the module's notes). `folder` is the folder's item;
/// `staged` the bytes, named as the file is to be called; `base` the version of Apple's file
/// this computer's copy was taken from; `ours` what this computer itself last sent under that
/// name.
///
/// What it replaces is found when it is sent, not when it was opened: by the version it was
/// taken from, else by its name. It is written over only if it is that version or this
/// computer's own last send, and Apple checks the version again in the same step as the write,
/// so a change landing between this look and the write is refused rather than written over.
/// Anything changed elsewhere is kept, and this computer's version goes beside it.
///
/// # Errors
/// When Apple cannot be reached or refuses; nothing is lost — the copy stays, and the send
/// is tried again.
pub async fn send(
    remote: &dyn Remote,
    folder: &DriveItem,
    staged: &Path,
    base: Option<&DriveItem>,
    ours: Option<&DriveItem>,
) -> Result<Sent, String> {
    let name = staged
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("no name")?
        .to_string();
    let now = remote.list(&folder.drivewsid).await?;
    let current = base
        .and_then(|b| now.iter().find(|i| i.drivewsid == b.drivewsid))
        .or_else(|| now.iter().find(|i| i.display_name() == name))
        .cloned();
    let same = |a: &DriveItem, b: &DriveItem| {
        a.drivewsid == b.drivewsid && a.etag == b.etag && a.size == b.size
    };
    let Some(current) = current else {
        // Nothing there by that version or name: a new file.
        return Ok(Sent {
            item: remote.upload(folder, staged).await?,
            conflicted: false,
        });
    };
    let ours_to_replace =
        base.is_some_and(|b| same(b, &current)) || ours.is_some_and(|o| same(o, &current));
    if ours_to_replace {
        // Written over at the version just read; refused by Apple if another device got there
        // first in the meantime.
        if let Some(item) = remote.replace(&current, staged).await? {
            if item.display_name() == name {
                return Ok(Sent {
                    item,
                    conflicted: false,
                });
            }
            // Renamed here while it waited (an earlier send still on its way when it was):
            // the same file takes its new name. Its contents are at Apple already, so a step
            // that fails is tried again where it stands, never by sending them again.
            let mut tries = 0u64;
            while let Err(e) = remote.rename(&item, &name).await {
                tries += 1;
                if tries >= 3 {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_millis(800 * tries)).await;
            }
            // Read back for the version the rename made; sent all the same if that fails.
            let item = remote
                .list(&folder.drivewsid)
                .await
                .ok()
                .and_then(|after| after.into_iter().find(|i| i.drivewsid == item.drivewsid))
                .unwrap_or(item);
            return Ok(Sent {
                item,
                conflicted: false,
            });
        }
    }
    // Both kept: this computer's beside the other device's, never over it.
    let copy = staged.with_file_name(conflicted(&name, SystemTime::now()));
    std::fs::copy(staged, &copy).map_err(|e| e.to_string())?;
    let sent = remote.upload(folder, &copy).await;
    let _ = std::fs::remove_file(&copy);
    Ok(Sent {
        item: sent?,
        conflicted: true,
    })
}

#[cfg(test)]
mod send_tests {
    //! `send` against a stand-in for Apple that keeps its files and their versions and, as
    //! Apple does (measured, probe `drive_modify_in_place`), refuses a write naming a version
    //! that is no longer the file's.
    use super::*;
    use crate::Fut;

    #[derive(Default)]
    struct Apple {
        files: Mutex<Vec<DriveItem>>,
        /// Every write, in order: "replace <id>", "upload <name>", "delete <id>", "rename <id>".
        log: Mutex<Vec<String>>,
        /// Another device saves the file right after this computer's next look at the folder.
        changed_after_look: Mutex<Option<String>>,
        next: Mutex<u32>,
    }

    impl Apple {
        fn with(files: Vec<DriveItem>) -> Self {
            Self {
                files: Mutex::new(files),
                ..Self::default()
            }
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
        fn bump(file: &mut DriveItem, size: i64) {
            let n = file
                .etag
                .as_deref()
                .unwrap_or("v0")
                .trim_start_matches('v')
                .parse::<u32>()
                .unwrap_or(0);
            file.etag = Some(format!("v{}", n + 1));
            file.size = Some(size);
        }
    }

    impl Remote for Apple {
        fn list(&self, _folder: &str) -> Fut<Vec<DriveItem>> {
            let listed = self.files.lock().unwrap().clone();
            if let Some(id) = self.changed_after_look.lock().unwrap().take() {
                for f in self
                    .files
                    .lock()
                    .unwrap()
                    .iter_mut()
                    .filter(|f| f.drivewsid == id)
                {
                    Apple::bump(f, 999);
                }
            }
            Box::pin(async move { Ok(listed) })
        }
        fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn replace(&self, item: &DriveItem, path: &Path) -> Fut<Option<DriveItem>> {
            self.log
                .lock()
                .unwrap()
                .push(format!("replace {}", item.drivewsid));
            let size = i64::try_from(std::fs::metadata(path).unwrap().len()).unwrap();
            let mut files = self.files.lock().unwrap();
            let answer = match files.iter_mut().find(|f| f.drivewsid == item.drivewsid) {
                Some(f) if f.etag == item.etag => {
                    Apple::bump(f, size);
                    Some(f.clone())
                }
                // Apple's OP_LOCK_FAILURE: nothing written.
                _ => None,
            };
            Box::pin(async move { Ok(answer) })
        }
        fn upload(&self, folder: &DriveItem, path: &Path) -> Fut<DriveItem> {
            let name = path.file_name().unwrap().to_str().unwrap().to_string();
            self.log.lock().unwrap().push(format!("upload {name}"));
            let mut next = self.next.lock().unwrap();
            *next += 1;
            let (stem, ext) = name
                .rsplit_once('.')
                .map_or((name.as_str(), None), |(s, e)| (s, Some(e)));
            let item = DriveItem {
                drivewsid: format!("FILE::new{next}"),
                parent_id: Some(folder.drivewsid.clone()),
                name: stem.into(),
                extension: ext.map(Into::into),
                size: Some(i64::try_from(std::fs::metadata(path).unwrap().len()).unwrap()),
                etag: Some("v1".into()),
                ..file("unused", "v1")
            };
            self.files.lock().unwrap().push(item.clone());
            Box::pin(async move { Ok(item) })
        }
        fn delete(&self, item: &DriveItem) -> Fut<()> {
            self.log
                .lock()
                .unwrap()
                .push(format!("delete {}", item.drivewsid));
            Box::pin(async { Ok(()) })
        }
        fn rename(&self, item: &DriveItem, _name: &str) -> Fut<()> {
            self.log
                .lock()
                .unwrap()
                .push(format!("rename {}", item.drivewsid));
            Box::pin(async { Ok(()) })
        }
    }

    fn file(id: &str, etag: &str) -> DriveItem {
        DriveItem {
            drivewsid: format!("FILE::{id}"),
            parent_id: Some("FOLDER::f".into()),
            name: "Passwords".into(),
            extension: Some("kdbx".into()),
            kind: "FILE".into(),
            size: Some(10),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: Some(etag.into()),
        }
    }

    fn folder() -> DriveItem {
        DriveItem {
            drivewsid: "FOLDER::f".into(),
            kind: "FOLDER".into(),
            name: "f".into(),
            extension: None,
            size: None,
            ..file("x", "v1")
        }
    }

    fn staged(dir: &Path, bytes: &[u8]) -> PathBuf {
        let path = dir.join("Passwords.kdbx");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    /// #24: a file saved over is the same file at Apple afterwards, with the new contents —
    /// never a new file given its name with the old one sent to Recently Deleted.
    #[test]
    fn a_file_saved_over_keeps_its_identity() {
        let dir = tempfile::tempdir().unwrap();
        let apple = Apple::with(vec![file("db", "v1")]);
        let base = file("db", "v1");
        let sent = run(send(
            &apple,
            &folder(),
            &staged(dir.path(), b"new contents"),
            Some(&base),
            None,
        ))
        .unwrap();
        assert!(!sent.conflicted);
        assert_eq!(sent.item.drivewsid, "FILE::db", "the same file");
        assert_eq!(sent.item.size, Some(12));
        assert_eq!(
            apple.log(),
            ["replace FILE::db"],
            "nothing uploaded, deleted or renamed"
        );
        // Saved again from what was just sent: this computer's own, written over again.
        let again = run(send(
            &apple,
            &folder(),
            &staged(dir.path(), b"newer"),
            Some(&base),
            Some(&sent.item),
        ))
        .unwrap();
        assert_eq!(
            (again.item.drivewsid.as_str(), again.conflicted),
            ("FILE::db", false)
        );
    }

    /// A file changed on another device since this computer's copy was taken: kept, and this
    /// computer's version goes beside it.
    #[test]
    fn a_file_changed_elsewhere_first_is_kept_and_ours_goes_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let apple = Apple::with(vec![file("db", "v2")]);
        let sent = run(send(
            &apple,
            &folder(),
            &staged(dir.path(), b"ours"),
            Some(&file("db", "v1")),
            None,
        ))
        .unwrap();
        assert!(sent.conflicted);
        assert_eq!(apple.log().len(), 1);
        assert!(
            apple.log()[0].starts_with("upload Passwords (conflicted copy "),
            "{:?}",
            apple.log()
        );
        assert_eq!(
            apple.files.lock().unwrap()[0].etag.as_deref(),
            Some("v2"),
            "theirs untouched"
        );
    }

    /// The race a look-then-write left open: another device saves between this computer's look
    /// and its write. Apple refuses the write; theirs stays, ours goes beside it.
    #[test]
    fn a_change_landing_between_the_look_and_the_write_is_never_written_over() {
        let dir = tempfile::tempdir().unwrap();
        let apple = Apple::with(vec![file("db", "v1")]);
        *apple.changed_after_look.lock().unwrap() = Some("FILE::db".into());
        let sent = run(send(
            &apple,
            &folder(),
            &staged(dir.path(), b"ours"),
            Some(&file("db", "v1")),
            None,
        ))
        .unwrap();
        assert!(sent.conflicted);
        let log = apple.log();
        assert_eq!(log[0], "replace FILE::db");
        assert!(
            log[1].starts_with("upload Passwords (conflicted copy "),
            "{log:?}"
        );
        let theirs = apple.files.lock().unwrap()[0].clone();
        assert_eq!(
            (theirs.etag.as_deref(), theirs.size),
            (Some("v2"), Some(999)),
            "theirs untouched"
        );
    }

    /// Nothing there by that version or name: a new file.
    #[test]
    fn a_new_file_is_added() {
        let dir = tempfile::tempdir().unwrap();
        let apple = Apple::with(Vec::new());
        let sent = run(send(
            &apple,
            &folder(),
            &staged(dir.path(), b"fresh"),
            None,
            None,
        ))
        .unwrap();
        assert!(!sent.conflicted);
        assert_eq!(apple.log(), ["upload Passwords.kdbx"]);
    }

    /// A file held for the phone's Files app: opened at `base`, its bytes written.
    fn held(writes: &Writes, key: u64, base: DriveItem, bytes: &[u8]) {
        writes.hold(key, |_| Pending {
            folder: "FOLDER::f".into(),
            name: "Passwords.kdbx".into(),
            replaces: Some(base),
            open: 0,
            dirty: true,
            generation: 0,
            modified: SystemTime::now(),
            sender: false,
            quiet_since: None,
            sending: false,
            failure: None,
        });
        let at = writes.staging(key, "Passwords.kdbx");
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, bytes).unwrap();
    }

    /// Android, 11 Nov: the folder refresh moved the library's item to another device's newer
    /// version between the open and the send, and the send, taking its base from the library
    /// then, wrote over that device's work. The base is the version opened: theirs is kept.
    #[test]
    fn a_held_file_is_sent_against_the_version_it_was_opened_at() {
        let dir = tempfile::tempdir().unwrap();
        let writes = Writes::new(dir.path()).unwrap();
        held(&writes, 7, file("db", "v1"), b"ours");
        // Saved on another device after the phone opened it.
        let apple = Apple::with(vec![file("db", "v2")]);
        let Held::Sent { sent, settled } =
            run(send_held(&apple, &writes, 7, &folder(), None)).unwrap()
        else {
            panic!("not sent");
        };
        assert!(sent.conflicted && settled);
        assert_eq!(
            apple.files.lock().unwrap()[0].etag.as_deref(),
            Some("v2"),
            "theirs untouched"
        );
        assert!(writes.keys().is_empty(), "nothing waits once sent");
    }

    /// A save the phone held is still there after the app was killed: the journal gives it
    /// back under its key, and it goes, its bytes moved to where the app keeps its copy.
    #[test]
    fn a_held_file_outlives_the_process_and_goes_once() {
        let dir = tempfile::tempdir().unwrap();
        held(
            &Writes::new(dir.path()).unwrap(),
            7,
            file("db", "v1"),
            b"saved offline",
        );
        // The process gone: a new journal over the same folder.
        let writes = Writes::new(dir.path()).unwrap();
        assert_eq!(writes.resume(), 1);
        assert_eq!(
            writes.get(7).unwrap().replaces.unwrap().etag.as_deref(),
            Some("v1")
        );
        let apple = Apple::with(vec![file("db", "v1")]);
        let copy = dir.path().join("copy/Passwords.kdbx");
        let Held::Sent { sent, settled } =
            run(send_held(&apple, &writes, 7, &folder(), Some(&copy))).unwrap()
        else {
            panic!("not sent");
        };
        assert!(settled && !sent.conflicted);
        assert_eq!(apple.log(), ["replace FILE::db"], "in place, the same file");
        assert_eq!(std::fs::read(&copy).unwrap(), b"saved offline");
        assert!(matches!(
            run(send_held(&apple, &writes, 7, &folder(), None)).unwrap(),
            Held::Nothing
        ));
        assert_eq!(
            Writes::new(dir.path()).unwrap().resume(),
            0,
            "the journal says so too"
        );
    }

    /// An app that still has the file open: nothing goes until it closes it.
    #[test]
    fn a_held_file_still_open_waits() {
        let dir = tempfile::tempdir().unwrap();
        let writes = Writes::new(dir.path()).unwrap();
        held(&writes, 7, file("db", "v1"), b"half");
        writes.hold(7, |p| Pending {
            open: 1,
            ..p.unwrap()
        });
        let apple = Apple::with(vec![file("db", "v1")]);
        assert!(matches!(
            run(send_held(&apple, &writes, 7, &folder(), None)).unwrap(),
            Held::Open
        ));
        assert!(apple.log().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scratch_files_never_leave() {
        for n in [
            ".report.docx.swp",
            "4913",
            "notes.txt~",
            ".#notes.txt",
            ".~lock.Deck.odp#",
            ".goutputstream-ABC123",
            "x.crdownload",
        ] {
            assert!(stays_local(n), "{n}");
        }
        for n in ["Report.docx", "notes.txt", "photo.jpg", "a~b.txt"] {
            assert!(!stays_local(n), "{n}");
        }
    }

    #[test]
    fn a_conflicted_copy_is_named_beside() {
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_521_600); // 2026-10-20 18:40 UTC
        assert_eq!(
            conflicted("Report.docx", when),
            "Report (conflicted copy 2026-10-20 18.40).docx"
        );
        assert_eq!(
            conflicted("Makefile", when),
            "Makefile (conflicted copy 2026-10-20 18.40)"
        );
    }

    #[test]
    fn what_waits_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let w = Writes::new(dir.path()).unwrap();
        let p = Pending {
            folder: "F".into(),
            name: "a.txt".into(),
            replaces: None,
            open: 1,
            dirty: true,
            generation: 3,
            modified: SystemTime::now(),
            sender: false,
            quiet_since: None,
            sending: false,
            failure: None,
        };
        std::fs::create_dir_all(w.staging(7, "a.txt").parent().unwrap()).unwrap();
        std::fs::write(w.staging(7, "a.txt"), b"hi").unwrap();
        w.items.lock().unwrap().insert(7, p.clone());
        w.items.lock().unwrap().insert(
            8,
            Pending {
                name: ".a.txt.swp".into(),
                ..p.clone()
            },
        );
        w.save();
        let again = Writes::new(dir.path()).unwrap().load();
        assert_eq!(again.len(), 1, "the scratch file is not kept");
        assert_eq!(again[0].0, 7);
        assert_eq!(again[0].1.name, "a.txt");
        assert_eq!(again[0].1.open, 0, "nothing is open after a restart");
    }

    fn waiting(name: &str) -> Pending {
        Pending {
            folder: "F".into(),
            name: name.into(),
            replaces: None,
            open: 0,
            dirty: true,
            generation: 1,
            modified: SystemTime::now(),
            sender: false,
            quiet_since: None,
            sending: false,
            failure: None,
        }
    }

    fn stage(w: &Writes, ino: u64, name: &str, bytes: &[u8]) {
        std::fs::create_dir_all(w.staging(ino, name).parent().unwrap()).unwrap();
        std::fs::write(w.staging(ino, name), bytes).unwrap();
        w.items.lock().unwrap().insert(ino, waiting(name));
    }

    /// Inode numbers start from the same place every mount. A file given back its own number
    /// had its bytes deleted by the clean-up after the move; two trading numbers lost one.
    #[test]
    fn files_carried_over_a_restart_keep_their_bytes_whatever_numbers_they_get() {
        let dir = tempfile::tempdir().unwrap();
        let w = Writes::new(dir.path()).unwrap();
        stage(&w, 2, "a.txt", b"first");
        stage(&w, 3, "b.txt", b"second");
        stage(&w, 4, "c.txt", b"third");
        w.save();
        let again = Writes::new(dir.path()).unwrap();
        // c keeps 4; a and b trade.
        let placed = again
            .take_back(|p| match p.name.as_str() {
                "a.txt" => 3,
                "b.txt" => 2,
                _ => 4,
            })
            .unwrap();
        assert_eq!(placed.len(), 3);
        for (ino, p) in &placed {
            let bytes = std::fs::read(again.staging(*ino, &p.name)).unwrap();
            let want: &[u8] = match p.name.as_str() {
                "a.txt" => b"first",
                "b.txt" => b"second",
                _ => b"third",
            };
            assert_eq!(bytes, want, "{} at {ino}", p.name);
        }
    }

    /// Stopped after setting the files aside and before placing them: the next start finds
    /// them where they were set aside, not missing.
    #[test]
    fn files_set_aside_when_the_app_stopped_are_found_again() {
        let dir = tempfile::tempdir().unwrap();
        let w = Writes::new(dir.path()).unwrap();
        stage(&w, 9, "a.txt", b"kept");
        w.save();
        std::fs::create_dir_all(dir.path().join(".restore")).unwrap();
        std::fs::rename(dir.path().join("9"), dir.path().join(".restore").join("9")).unwrap();
        let placed = Writes::new(dir.path()).unwrap().take_back(|_| 2).unwrap();
        assert_eq!(placed.len(), 1);
        assert_eq!(std::fs::read(w.staging(2, "a.txt")).unwrap(), b"kept");
    }
}
