//! A file's own bytes written through the folder, as programs write them: a new file, an
//! append, an editor's save (a temporary file renamed over the old), a file changed on
//! another device meanwhile, a scratch file, and a restart before the send. Each checked
//! against what Apple ends up holding. Skipped where FUSE is not available.
#![cfg(target_os = "linux")]

use quince_core::drive::DriveItem;
use quince_vfs::{fuse, Cache, Fut, Remote, Store, ROOT};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct World {
    folders: Mutex<HashMap<String, Vec<DriveItem>>>,
    bytes: Mutex<HashMap<String, Vec<u8>>>,
    uploads: Mutex<Vec<String>>,
    /// Files whose contents were written in place, by id.
    replaced: Mutex<Vec<String>>,
    /// Apple refusing every change, as when it is down.
    refuse: Mutex<bool>,
    /// How long each step at "Apple" takes: slow, as on a real connection.
    slow: Mutex<Duration>,
    deleted: Mutex<Vec<String>>,
    next: Mutex<u32>,
}

impl World {
    fn file(&self, folder: &str, name: &str, bytes: &[u8]) -> DriveItem {
        let mut n = self.next.lock().unwrap();
        *n += 1;
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), Some(e.to_string())),
            _ => (name.to_string(), None),
        };
        let item = DriveItem {
            drivewsid: format!("FILE::z::{n}"),
            parent_id: Some(folder.into()),
            name: stem,
            extension: ext,
            kind: "FILE".into(),
            size: Some(bytes.len() as i64),
            child_count: None,
            created_at: Some(1_700_000_000),
            modified_at: Some(1_700_000_000 + i64::from(*n)),
            etag: Some(format!("v{n}")),
            share: None,
        };
        self.bytes
            .lock()
            .unwrap()
            .insert(item.drivewsid.clone(), bytes.to_vec());
        self.folders
            .lock()
            .unwrap()
            .entry(folder.into())
            .or_default()
            .push(item.clone());
        item
    }
    fn names(&self, folder: &str) -> Vec<String> {
        let mut n: Vec<String> = self
            .folders
            .lock()
            .unwrap()
            .get(folder)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(DriveItem::display_name)
            .collect();
        n.sort();
        n
    }
    fn bytes_of(&self, folder: &str, name: &str) -> Option<Vec<u8>> {
        let f = self.folders.lock().unwrap();
        let id = f
            .get(folder)?
            .iter()
            .find(|i| i.display_name() == name)?
            .drivewsid
            .clone();
        self.bytes.lock().unwrap().get(&id).cloned()
    }
}

struct Apple(Arc<World>);
struct Library(Arc<World>);
impl Store for Library {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        Some(
            self.0
                .folders
                .lock()
                .unwrap()
                .get(folder)
                .cloned()
                .unwrap_or_default(),
        )
    }
}
impl Remote for Apple {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let out = self
            .0
            .folders
            .lock()
            .unwrap()
            .get(folder)
            .cloned()
            .unwrap_or_default();
        Box::pin(async move { Ok(out) })
    }
    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        let b = self.0.bytes.lock().unwrap()[&item.drivewsid]
            [offset as usize..(offset + len) as usize]
            .to_vec();
        Box::pin(async move { Ok(b) })
    }
    fn upload(&self, folder: &DriveItem, path: &Path) -> Fut<DriveItem> {
        if *self.0.refuse.lock().unwrap() {
            return Box::pin(async { Err("iCloud is busy right now".to_string()) });
        }
        let bytes = std::fs::read(path).unwrap();
        let mut name = path.file_name().unwrap().to_str().unwrap().to_string();
        // A name already taken is filed as "name 2", as Apple files it.
        if self.0.names(&folder.drivewsid).contains(&name) {
            name = match name.rsplit_once('.') {
                Some((s, e)) => format!("{s} 2.{e}"),
                None => format!("{name} 2"),
            };
        }
        self.0.uploads.lock().unwrap().push(name.clone());
        let slow = *self.0.slow.lock().unwrap();
        let (world, folder) = (Arc::clone(&self.0), folder.drivewsid.clone());
        Box::pin(async move {
            tokio::time::sleep(slow).await;
            Ok(world.file(&folder, &name, &bytes))
        })
    }
    /// New contents into the same file, refused when it is no longer the version named — as
    /// Apple's `modify_file` with its etag (probe `drive_modify_in_place`).
    fn replace(&self, it: &DriveItem, path: &Path) -> Fut<Option<DriveItem>> {
        if *self.0.refuse.lock().unwrap() {
            return Box::pin(async { Err("iCloud is busy right now".to_string()) });
        }
        let bytes = std::fs::read(path).unwrap();
        self.0.replaced.lock().unwrap().push(it.drivewsid.clone());
        let slow = *self.0.slow.lock().unwrap();
        let (world, it) = (Arc::clone(&self.0), it.clone());
        Box::pin(async move {
            tokio::time::sleep(slow).await;
            let mut n = world.next.lock().unwrap();
            *n += 1;
            let mut folders = world.folders.lock().unwrap();
            let Some(now) = folders
                .values_mut()
                .flat_map(|l| l.iter_mut())
                .find(|x| x.drivewsid == it.drivewsid)
            else {
                return Err("no such file".to_string());
            };
            if now.etag != it.etag {
                return Ok(None);
            }
            now.etag = Some(format!("v{n}"));
            now.size = Some(bytes.len() as i64);
            let now = now.clone();
            world
                .bytes
                .lock()
                .unwrap()
                .insert(now.drivewsid.clone(), bytes);
            Ok(Some(now))
        })
    }
    fn rename(&self, it: &DriveItem, name: &str) -> Fut<()> {
        // A name already taken is refused, as Apple refuses it.
        let folder = it.parent_id.clone().unwrap_or_default();
        if self.0.names(&folder).iter().any(|n| n == name) {
            return Box::pin(async { Err("name taken".to_string()) });
        }
        for list in self.0.folders.lock().unwrap().values_mut() {
            for x in list.iter_mut().filter(|x| x.drivewsid == it.drivewsid) {
                let (s, e) = name
                    .rsplit_once('.')
                    .map_or((name, None), |(s, e)| (s, Some(e)));
                x.name = s.into();
                x.extension = e.map(Into::into);
                x.etag = Some(format!("{}r", x.etag.clone().unwrap_or_default()));
            }
        }
        Box::pin(async { Ok(()) })
    }
    fn delete(&self, it: &DriveItem) -> Fut<()> {
        if *self.0.refuse.lock().unwrap() {
            return Box::pin(async { Err("iCloud is busy right now".to_string()) });
        }
        let slow = *self.0.slow.lock().unwrap() / 5;
        let (world, id) = (Arc::clone(&self.0), it.drivewsid.clone());
        Box::pin(async move {
            tokio::time::sleep(slow).await;
            for list in world.folders.lock().unwrap().values_mut() {
                list.retain(|x| x.drivewsid != id);
            }
            world.deleted.lock().unwrap().push(id);
            Ok(())
        })
    }
}

fn mount(
    world: &Arc<World>,
    rt: &tokio::runtime::Runtime,
    at: &Path,
    cache: &Path,
    staging: &Path,
) -> fuser::BackgroundSession {
    let fs = fuse::DriveFs::new(
        Arc::new(Apple(Arc::clone(world))),
        Arc::new(Library(Arc::clone(world))),
        Arc::new(Cache::new(cache).unwrap()),
        rt.handle().clone(),
    )
    .with_writes(staging)
    .unwrap();
    fuse::mount(fs, at).expect("mounted")
}

/// Until Apple holds `want` under `name`, or a few seconds pass.
fn settled(world: &World, name: &str, want: &[u8]) -> bool {
    for _ in 0..80 {
        if world.bytes_of(ROOT, name).as_deref() == Some(want) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn writes_reach_apple_safely() {
    if !Path::new("/dev/fuse").exists() {
        eprintln!("no FUSE here; skipped");
        return;
    }
    let world = Arc::new(World::default());
    world
        .folders
        .lock()
        .unwrap()
        .insert(ROOT.into(), Vec::new());
    let notes = world.file(ROOT, "Notes.txt", b"first line\n");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (cache, staging, at) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let root = at.path();
    let session = mount(&world, &rt, root, cache.path(), staging.path());

    // A new file, as a shell writes one: there at once, at Apple once left alone.
    std::fs::write(root.join("Hello.txt"), b"hello\n").unwrap();
    assert_eq!(
        std::fs::read(root.join("Hello.txt")).unwrap(),
        b"hello\n",
        "reads back at once, before Apple has it"
    );
    assert!(
        settled(&world, "Hello.txt", b"hello\n"),
        "at Apple: {:?}",
        world.names(ROOT)
    );
    let listed: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        listed.iter().filter(|n| *n == "Hello.txt").count(),
        1,
        "listed once, not twice: {listed:?}"
    );

    // An append: Apple ends with the whole of it written into the same file (#24) — its id
    // kept, nothing in Recently Deleted, nothing named "Notes 2.txt" left behind.
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(root.join("Notes.txt"))
        .unwrap();
    f.write_all(b"second line\n").unwrap();
    drop(f);
    assert!(
        settled(&world, "Notes.txt", b"first line\nsecond line\n"),
        "appended at Apple: {:?}",
        world.names(ROOT)
    );
    assert_eq!(world.names(ROOT), ["Hello.txt", "Notes.txt"]);
    assert!(
        world.deleted.lock().unwrap().is_empty(),
        "nothing into Recently Deleted"
    );
    let id_now = |world: &World| {
        world.folders.lock().unwrap()[ROOT]
            .iter()
            .find(|i| i.display_name() == "Notes.txt")
            .map(|i| i.drivewsid.clone())
    };
    assert_eq!(
        id_now(&world),
        Some(notes.drivewsid.clone()),
        "the same file at Apple"
    );

    // An editor's save: a temporary file written, then renamed over the old one.
    std::fs::write(root.join(".goutputstream-AB12"), b"rewritten\n").unwrap();
    std::fs::rename(root.join(".goutputstream-AB12"), root.join("Notes.txt")).unwrap();
    assert!(
        settled(&world, "Notes.txt", b"rewritten\n"),
        "saved at Apple: {:?}",
        world.names(ROOT)
    );
    assert!(
        !world
            .uploads
            .lock()
            .unwrap()
            .iter()
            .any(|n| n.starts_with(".goutputstream")),
        "the temporary never went to Apple"
    );
    assert_eq!(world.names(ROOT), ["Hello.txt", "Notes.txt"]);
    // KeePass, LibreOffice and most editors save this way: still the same file at Apple.
    assert_eq!(
        id_now(&world),
        Some(notes.drivewsid.clone()),
        "an editor's save keeps the file"
    );

    // Changed on another device while open here: both kept, theirs untouched.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(root.join("Hello.txt"))
        .unwrap();
    {
        let mut folders = world.folders.lock().unwrap();
        let theirs = folders
            .get_mut(ROOT)
            .unwrap()
            .iter_mut()
            .find(|i| i.display_name() == "Hello.txt")
            .unwrap();
        theirs.etag = Some("changed-on-the-phone".into());
        world
            .bytes
            .lock()
            .unwrap()
            .insert(theirs.drivewsid.clone(), b"from the phone\n".to_vec());
        theirs.size = Some(15);
    }
    f.write_all(b"from this computer\n").unwrap();
    drop(f);
    let mut copy = None;
    for _ in 0..80 {
        copy = world
            .names(ROOT)
            .into_iter()
            .find(|n| n.contains("conflicted copy"));
        if copy.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let copy = copy.expect("a conflicted copy");
    assert_eq!(
        world.bytes_of(ROOT, &copy).as_deref(),
        Some(&b"from this computer\n"[..])
    );
    assert_eq!(
        world.bytes_of(ROOT, "Hello.txt").as_deref(),
        Some(&b"from the phone\n"[..]),
        "the phone's never overwritten"
    );

    // A scratch file never leaves this computer, and goes without a word to Apple.
    let uploads_before = world.uploads.lock().unwrap().len();
    std::fs::write(root.join(".Notes.txt.swp"), b"vim").unwrap();
    std::thread::sleep(Duration::from_millis(2600));
    std::fs::remove_file(root.join(".Notes.txt.swp")).unwrap();
    assert_eq!(
        world.uploads.lock().unwrap().len(),
        uploads_before,
        "never sent"
    );

    // Written, then the folder gone before it was sent: sent when it comes back.
    std::fs::write(root.join("Late.txt"), b"late\n").unwrap();
    drop(session);
    assert!(
        world.bytes_of(ROOT, "Late.txt").is_none(),
        "not sent before the folder went"
    );
    let again = mount(&world, &rt, root, cache.path(), staging.path());
    assert!(
        settled(&world, "Late.txt", b"late\n"),
        "sent after the restart: {:?}",
        world.names(ROOT)
    );
    drop(again);
}

/// A save arriving while the last save of the same file is still on its way (Apple slow):
/// it waits its turn, and Apple ends with one file under the name holding the newest bytes —
/// never "name 2" left beside it (measured live before the fix).
#[test]
fn a_save_during_a_send_waits_its_turn() {
    if !Path::new("/dev/fuse").exists() {
        return;
    }
    let world = Arc::new(World::default());
    world
        .folders
        .lock()
        .unwrap()
        .insert(ROOT.into(), Vec::new());
    world.file(ROOT, "Note.txt", b"one\n");
    // As slow as Apple was when this was measured: a send longer than the settle time.
    *world.slow.lock().unwrap() = Duration::from_millis(3000);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (cache, staging, at) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let root = at.path();
    let session = mount(&world, &rt, root, cache.path(), staging.path());
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(root.join("Note.txt"))
        .unwrap();
    f.write_all(b"two\n").unwrap();
    drop(f);
    // Until the first send has begun at Apple…
    for _ in 0..60 {
        if !world.uploads.lock().unwrap().is_empty() || !world.replaced.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // …then an editor saves over it.
    std::fs::write(root.join(".goutputstream-Z9"), b"three\n").unwrap();
    std::fs::rename(root.join(".goutputstream-Z9"), root.join("Note.txt")).unwrap();
    assert!(
        settled(&world, "Note.txt", b"three\n"),
        "{:?}",
        world.names(ROOT)
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        world.names(ROOT),
        ["Note.txt"],
        "one file under the name, nothing left beside it"
    );
    assert_eq!(
        world.bytes_of(ROOT, "Note.txt").as_deref(),
        Some(&b"three\n"[..])
    );
    drop(session);
}

/// What is waiting is told as it is: one file on its way while Apple is slow, nothing once it
/// is there, and Apple's reason while it refuses — so nothing ever waits unseen.
#[test]
fn what_is_waiting_is_told() {
    if !Path::new("/dev/fuse").exists() {
        return;
    }
    let world = Arc::new(World::default());
    world
        .folders
        .lock()
        .unwrap()
        .insert(ROOT.into(), Vec::new());
    *world.slow.lock().unwrap() = Duration::from_millis(1500);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (cache, staging, at) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let fs = fuse::DriveFs::new(
        Arc::new(Apple(Arc::clone(&world))),
        Arc::new(Library(Arc::clone(&world))),
        Arc::new(Cache::new(cache.path()).unwrap()),
        rt.handle().clone(),
    )
    .with_writes(staging.path())
    .unwrap();
    let writes = fs.writes().unwrap();
    let session = fuse::mount(fs, at.path()).unwrap();
    std::fs::write(at.path().join("a.txt"), b"a").unwrap();
    std::fs::write(at.path().join(".a.txt.swp"), b"vim").unwrap();
    assert_eq!(
        writes.waiting().files,
        1,
        "one waiting; the scratch file is not counted"
    );
    let mut seen_sending = false;
    for _ in 0..60 {
        let w = writes.waiting();
        seen_sending |= w.sending == 1;
        if w.files == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(seen_sending, "told while on its way");
    assert_eq!(
        writes.waiting().files,
        0,
        "nothing waiting once Apple has it"
    );
    *world.refuse.lock().unwrap() = true;
    *world.slow.lock().unwrap() = Duration::ZERO;
    std::fs::write(at.path().join("b.txt"), b"b").unwrap();
    let mut why = None;
    for _ in 0..60 {
        why = writes.waiting().failure;
        if why.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        why.as_deref(),
        Some("iCloud is busy right now"),
        "Apple's reason, while it refuses"
    );
    assert_eq!(
        writes.waiting().files,
        1,
        "and the file still waiting, not lost"
    );
    drop(session);
}

/// Starts a mount over a world whose Apple is slow, as `a_save_during_a_send_waits_its_turn`.
fn slow_world() -> Arc<World> {
    let world = Arc::new(World::default());
    world
        .folders
        .lock()
        .unwrap()
        .insert(ROOT.into(), Vec::new());
    *world.slow.lock().unwrap() = Duration::from_millis(2500);
    world
}

fn upload_started(world: &World) {
    for _ in 0..80 {
        if !world.uploads.lock().unwrap().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A file deleted while it is on its way to Apple stays deleted: what arrives goes straight
/// to Recently Deleted (kept, it came back after `rm` had said it was gone).
#[test]
fn a_file_deleted_while_sending_stays_deleted() {
    if !Path::new("/dev/fuse").exists() {
        return;
    }
    let world = slow_world();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (cache, staging, at) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let root = at.path();
    let session = mount(&world, &rt, root, cache.path(), staging.path());
    std::fs::write(root.join("Big.mov"), b"frames\n").unwrap();
    upload_started(&world);
    std::fs::remove_file(root.join("Big.mov")).unwrap();
    std::thread::sleep(Duration::from_millis(5000));
    assert!(
        !world.names(ROOT).iter().any(|n| n == "Big.mov"),
        "deleted while sending, yet at Apple: {:?}",
        world.names(ROOT)
    );
    drop(session);
}

/// A file renamed while it is on its way to Apple ends under its new name only.
#[test]
fn a_file_renamed_while_sending_ends_under_its_new_name() {
    if !Path::new("/dev/fuse").exists() {
        return;
    }
    let world = slow_world();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (cache, staging, at) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let root = at.path();
    let session = mount(&world, &rt, root, cache.path(), staging.path());
    std::fs::write(root.join("a.txt"), b"words\n").unwrap();
    upload_started(&world);
    std::fs::rename(root.join("a.txt"), root.join("b.txt")).unwrap();
    assert!(
        settled(&world, "b.txt", b"words\n"),
        "{:?}",
        world.names(ROOT)
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(world.names(ROOT), ["b.txt"], "only the new name");
    drop(session);
}

/// Written, the app stopped before the send, and the folder set up again the way the app
/// does it — thumbnails configured after what was waiting is taken back — mounts and sends.
/// Before the fix, taking it back started its sender at once, and configuring the thumbnails
/// next panicked ("configured before it is mounted"): the folder never came up (#253).
#[test]
fn a_folder_with_files_waiting_mounts_and_sends_them() {
    if !Path::new("/dev/fuse").exists() {
        return;
    }
    let world = Arc::new(World::default());
    world
        .folders
        .lock()
        .unwrap()
        .insert(ROOT.into(), Vec::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (cache, staging, at, thumbs) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    // Apple refusing, so the file is still waiting when the app stops (its own runtime,
    // gone with it: no sender of that run is left to send it).
    *world.refuse.lock().unwrap() = true;
    let before = tokio::runtime::Runtime::new().unwrap();
    let session = mount(&world, &before, at.path(), cache.path(), staging.path());
    std::fs::write(at.path().join("Late.txt"), b"late\n").unwrap();
    drop(session);
    before.shutdown_background();
    *world.refuse.lock().unwrap() = false;

    let fs = fuse::DriveFs::new(
        Arc::new(Apple(Arc::clone(&world))),
        Arc::new(Library(Arc::clone(&world))),
        Arc::new(Cache::new(cache.path()).unwrap()),
        rt.handle().clone(),
    )
    .with_writes(staging.path())
    .unwrap()
    .with_thumbnails(at.path().to_path_buf(), thumbs.path().to_path_buf());
    let writes = fs.writes().unwrap();
    assert_eq!(writes.waiting().files, 1, "still waiting before the mount");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        world.bytes_of(ROOT, "Late.txt").is_none(),
        "nothing sent before the folder is mounted"
    );
    let again = fuse::mount(fs, at.path()).expect("mounted");
    assert!(
        settled(&world, "Late.txt", b"late\n"),
        "sent once mounted: {:?}",
        world.names(ROOT)
    );
    drop(again);
}
