//! Changes through the folder, as a file manager or a shell makes them: a new folder, a
//! rename, a move, a delete — each made at Apple and then shown as Apple has it. A folder
//! with something in it is not deleted whole; nothing is replaced by a rename. Skipped where
//! FUSE is not available.
#![cfg(target_os = "linux")]

use quince_core::drive::DriveItem;
use quince_vfs::{fuse, Cache, Fut, Remote, Store, ROOT};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn item(id: &str, parent: &str, name: &str, ext: Option<&str>, kind: &str) -> DriveItem {
    DriveItem {
        drivewsid: id.into(),
        parent_id: Some(parent.into()),
        name: name.into(),
        extension: ext.map(Into::into),
        kind: kind.into(),
        size: Some(if kind == "FILE" { 3 } else { 0 }),
        child_count: Some(0),
        created_at: Some(1_700_000_000),
        modified_at: Some(1_700_000_100),
        etag: Some("e1".into()),
        share: None,
    }
}

/// Apple's Drive, and the library that mirrors it after every change.
#[derive(Default)]
struct World {
    folders: Mutex<HashMap<String, Vec<DriveItem>>>,
    deleted: Mutex<Vec<String>>,
    next: Mutex<u32>,
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
    fn read(&self, _: &DriveItem, _: u64, len: u64) -> Fut<Vec<u8>> {
        Box::pin(async move { Ok(vec![b'x'; len as usize]) })
    }
    fn mkdir(&self, parent: &DriveItem, name: &str) -> Fut<DriveItem> {
        let mut n = self.0.next.lock().unwrap();
        *n += 1;
        let made = item(
            &format!("FOLDER::z::new{n}"),
            &parent.drivewsid,
            name,
            None,
            "FOLDER",
        );
        self.0
            .folders
            .lock()
            .unwrap()
            .entry(parent.drivewsid.clone())
            .or_default()
            .push(made.clone());
        Box::pin(async move { Ok(made) })
    }
    fn rename(&self, it: &DriveItem, name: &str) -> Fut<()> {
        let mut folders = self.0.folders.lock().unwrap();
        for list in folders.values_mut() {
            for x in list.iter_mut().filter(|x| x.drivewsid == it.drivewsid) {
                match name.rsplit_once('.') {
                    Some((stem, ext)) if x.kind == "FILE" => {
                        x.name = stem.into();
                        x.extension = Some(ext.into());
                    }
                    _ => x.name = name.into(),
                }
            }
        }
        Box::pin(async { Ok(()) })
    }
    fn move_to(&self, it: &DriveItem, to: &DriveItem) -> Fut<()> {
        let mut folders = self.0.folders.lock().unwrap();
        let mut moved = None;
        for list in folders.values_mut() {
            if let Some(i) = list.iter().position(|x| x.drivewsid == it.drivewsid) {
                moved = Some(list.remove(i));
            }
        }
        let mut m = moved.unwrap();
        m.parent_id = Some(to.drivewsid.clone());
        folders.entry(to.drivewsid.clone()).or_default().push(m);
        Box::pin(async { Ok(()) })
    }
    fn delete(&self, it: &DriveItem) -> Fut<()> {
        for list in self.0.folders.lock().unwrap().values_mut() {
            list.retain(|x| x.drivewsid != it.drivewsid);
        }
        self.0.deleted.lock().unwrap().push(it.drivewsid.clone());
        Box::pin(async { Ok(()) })
    }
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut n: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    n.sort();
    n
}

#[test]
fn changes_through_the_folder_are_made_at_apple() {
    if !std::path::Path::new("/dev/fuse").exists() {
        eprintln!("no FUSE here; skipped");
        return;
    }
    let world = Arc::new(World::default());
    world.folders.lock().unwrap().insert(
        ROOT.into(),
        vec![
            item("FILE::z::a", ROOT, "Notes", Some("txt"), "FILE"),
            item("FILE::z::b", ROOT, "Plan", Some("pdf"), "FILE"),
        ],
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    let fs = fuse::DriveFs::new(
        Arc::new(Apple(Arc::clone(&world))),
        Arc::new(Library(Arc::clone(&world))),
        Arc::new(Cache::new(cache.path()).unwrap()),
        rt.handle().clone(),
    );
    let session = fuse::mount(fs, at.path()).expect("mounted");
    let root = at.path();

    std::fs::create_dir(root.join("Trips")).unwrap();
    assert_eq!(
        names(root),
        ["Notes.txt", "Plan.pdf", "Trips"],
        "a new folder"
    );
    assert_eq!(
        std::fs::create_dir(root.join("Trips"))
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EEXIST),
        "not twice"
    );

    std::fs::rename(root.join("Plan.pdf"), root.join("Plan 2026.pdf")).unwrap();
    assert_eq!(
        names(root),
        ["Notes.txt", "Plan 2026.pdf", "Trips"],
        "renamed in place"
    );

    std::fs::rename(root.join("Plan 2026.pdf"), root.join("Trips/Plan 2026.pdf")).unwrap();
    assert_eq!(
        names(&root.join("Trips")),
        ["Plan 2026.pdf"],
        "moved into a folder"
    );
    std::fs::rename(root.join("Trips/Plan 2026.pdf"), root.join("Itinerary.pdf")).unwrap();
    assert_eq!(
        names(root),
        ["Itinerary.pdf", "Notes.txt", "Trips"],
        "moved back out under a new name"
    );

    // A rename over a file that is there replaces it, as on any disk — the replaced one into
    // Apple's Recently Deleted, never gone.
    std::fs::rename(root.join("Itinerary.pdf"), root.join("Notes.txt")).unwrap();
    assert_eq!(names(root), ["Notes.txt", "Trips"]);
    assert_eq!(world.deleted.lock().unwrap().len(), 1);
    std::fs::rename(root.join("Notes.txt"), root.join("Itinerary.pdf")).unwrap();

    std::fs::rename(root.join("Itinerary.pdf"), root.join("Trips/Itinerary.pdf")).unwrap();
    let full = std::fs::remove_dir(root.join("Trips")).unwrap_err();
    assert_eq!(
        full.raw_os_error(),
        Some(libc::ENOTEMPTY),
        "a folder with something in it is not deleted whole"
    );
    std::fs::remove_file(root.join("Trips/Itinerary.pdf")).unwrap();
    std::fs::remove_dir(root.join("Trips")).unwrap();
    assert!(
        names(root).is_empty(),
        "deleted, file then folder, as rm -r does"
    );
    assert_eq!(
        world.deleted.lock().unwrap().len(),
        3,
        "each a delete at Apple (into Recently Deleted), the replaced file among them"
    );

    drop(session);
}
