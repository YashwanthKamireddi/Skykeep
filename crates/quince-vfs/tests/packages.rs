//! Keynote, Pages and Numbers documents through a real FUSE mount: iCloud lists them at one
//! size and serves them, flattened, at another (rclone #9798). Read by a program the way any
//! file is, they come out whole — never cut at the listed size, never an error for bytes the
//! listing promised — and their size reads as served once Apple has said. Skipped where FUSE
//! is not available.
#![cfg(target_os = "linux")]

use quince_core::drive::DriveItem;
use quince_vfs::{fuse, Cache, Fut, Part, Remote, Store, CHUNK, ROOT};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn item(id: &str, name: &str, ext: &str, listed: i64) -> DriveItem {
    DriveItem {
        drivewsid: id.into(),
        parent_id: Some(ROOT.into()),
        name: name.into(),
        extension: Some(ext.into()),
        kind: "FILE".into(),
        size: Some(listed),
        child_count: None,
        created_at: Some(1_700_000_000),
        modified_at: Some(1_700_000_100),
        etag: Some("e1".into()),
        share: None,
    }
}

/// Apple's download links: a range answered with what of the file there is, and its length
/// as served (`Content-Range`'s total).
struct Apple {
    files: HashMap<String, Vec<u8>>,
    reads: Mutex<Vec<(String, u64, u64)>>,
}
impl Remote for Apple {
    fn list(&self, _: &str) -> Fut<Vec<DriveItem>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn read(&self, _: &DriveItem, _: u64, _: u64) -> Fut<Vec<u8>> {
        unreachable!("read in parts")
    }
    fn read_part(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Part> {
        self.reads
            .lock()
            .unwrap()
            .push((item.name.clone(), offset, len));
        let data = &self.files[&item.drivewsid];
        let n = data.len() as u64;
        let (from, to) = (offset.min(n) as usize, (offset + len).min(n) as usize);
        let part = Part {
            bytes: data[from..to].to_vec(),
            total: Some(n),
        };
        Box::pin(async move { Ok(part) })
    }
}

struct Library(Vec<DriveItem>);
impl Store for Library {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        (folder == ROOT).then(|| self.0.clone())
    }
}

#[test]
fn iwork_documents_read_whole_at_their_served_size() {
    if !std::path::Path::new("/dev/fuse").exists() {
        eprintln!("no FUSE here; skipped");
        return;
    }
    // Larger as served than listed, across a chunk; and smaller.
    let deck: Vec<u8> = (0..CHUNK + 70_000).map(|i| (i % 251) as u8).collect();
    let essay: Vec<u8> = (0..30_000u32).map(|i| (i % 241) as u8).collect();
    let listed = vec![
        item("FILE::z::deck", "Deck", "key", 70_000),
        item("FILE::z::essay", "Essay", "pages", 9_000_000),
    ];
    let apple = Arc::new(Apple {
        files: HashMap::from([
            ("FILE::z::deck".to_string(), deck.clone()),
            ("FILE::z::essay".to_string(), essay.clone()),
        ]),
        reads: Mutex::new(Vec::new()),
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    let fs = fuse::DriveFs::new(
        apple.clone(),
        Arc::new(Library(listed)),
        Arc::new(Cache::new(cache_dir.path()).unwrap()),
        rt.handle().clone(),
    );
    let session = fuse::mount(fs, at.path()).expect("mounted");

    let deck_at = at.path().join("Deck.key");
    let essay_at = at.path().join("Essay.pages");
    // Before anything is asked of Apple: the listing's size.
    assert_eq!(std::fs::metadata(&deck_at).unwrap().len(), 70_000);
    assert!(
        apple.reads.lock().unwrap().is_empty(),
        "a look fetches nothing"
    );

    assert_eq!(
        std::fs::read(&deck_at).unwrap(),
        deck,
        "whole, not cut at 70,000"
    );
    assert_eq!(
        std::fs::read(&essay_at).unwrap(),
        essay,
        "to its end, no error"
    );
    // Copied as a file manager copies one: the same bytes.
    let elsewhere = tempfile::tempdir().unwrap();
    let copied = elsewhere.path().join("copied.key");
    std::fs::copy(&deck_at, &copied).unwrap();
    assert_eq!(std::fs::read(&copied).unwrap(), deck);

    // Once the attributes are looked at again: the size as served.
    std::thread::sleep(std::time::Duration::from_millis(1_200));
    assert_eq!(
        std::fs::metadata(&deck_at).unwrap().len(),
        deck.len() as u64
    );
    assert_eq!(
        std::fs::metadata(&essay_at).unwrap().len(),
        essay.len() as u64
    );
    // Each byte fetched once: the length asked (one byte), then the chunks.
    let reads = apple.reads.lock().unwrap().clone();
    let deck_reads: Vec<(u64, u64)> = reads
        .iter()
        .filter(|(n, ..)| n == "Deck")
        .map(|(_, o, l)| (*o, *l))
        .collect();
    assert_eq!(deck_reads, [(0, 1), (0, CHUNK), (CHUNK, 70_000)]);
    drop(session);
}
