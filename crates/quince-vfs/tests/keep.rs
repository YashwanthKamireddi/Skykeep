//! "Keep on This Computer", through a real FUSE mount of a made-up iCloud Drive, as any program
//! asks for it: the `user.quince.keep` attribute set on a file fetches it whole, behind; set on
//! a folder it keeps every file inside; a trim never lets a kept file go; taken off, it can go
//! again. Skipped where FUSE is not available.
#![cfg(target_os = "linux")]

use quince_core::drive::DriveItem;
use quince_vfs::{fuse, Cache, Fut, Remote, Store, CHUNK, ROOT};
use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn file(id: &str, parent: &str, name: &str, size: usize) -> DriveItem {
    DriveItem {
        drivewsid: id.into(),
        parent_id: Some(parent.into()),
        name: name.into(),
        extension: Some("bin".into()),
        kind: "FILE".into(),
        size: Some(size as i64),
        child_count: None,
        created_at: Some(1_700_000_000),
        modified_at: Some(1_700_000_100),
        etag: Some("e1".into()),
        share: None,
    }
}

struct Apple {
    folders: HashMap<String, Vec<DriveItem>>,
    files: HashMap<String, Vec<u8>>,
    reads: Mutex<Vec<String>>,
}
impl Remote for Apple {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let out = self.folders.get(folder).cloned().unwrap_or_default();
        Box::pin(async move { Ok(out) })
    }
    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        self.reads.lock().unwrap().push(item.drivewsid.clone());
        let data = &self.files[&item.drivewsid];
        let out = data[offset as usize..(offset + len) as usize].to_vec();
        Box::pin(async move { Ok(out) })
    }
}
struct Library(HashMap<String, Vec<DriveItem>>);
impl Store for Library {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        self.0.get(folder).cloned()
    }
}

fn c(p: &Path) -> CString {
    CString::new(p.as_os_str().as_bytes()).unwrap()
}
fn set(p: &Path, v: &[u8]) -> i32 {
    let name = CString::new("user.quince.keep").unwrap();
    // SAFETY: valid C strings and a buffer of the given length.
    unsafe { libc::setxattr(c(p).as_ptr(), name.as_ptr(), v.as_ptr().cast(), v.len(), 0) }
}
fn get(p: &Path) -> Option<Vec<u8>> {
    let name = CString::new("user.quince.keep").unwrap();
    let mut buf = [0u8; 8];
    // SAFETY: as above; the buffer's length is passed.
    let n = unsafe {
        libc::getxattr(
            c(p).as_ptr(),
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    (n >= 0).then(|| buf[..n as usize].to_vec())
}
fn remove(p: &Path) -> i32 {
    let name = CString::new("user.quince.keep").unwrap();
    // SAFETY: valid C strings.
    unsafe { libc::removexattr(c(p).as_ptr(), name.as_ptr()) }
}
fn until(what: impl Fn() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10) {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn kept_on_this_computer_stays_whole() {
    if !Path::new("/dev/fuse").exists() {
        eprintln!("no FUSE here; skipped");
        return;
    }
    let big: Vec<u8> = (0..(CHUNK * 2 + 77)).map(|i| (i % 251) as u8).collect();
    let small = vec![7u8; 1000];
    let mut trips = file("FOLDER::z::trips", ROOT, "Trips", 0);
    trips.kind = "FOLDER".into();
    trips.extension = None;
    trips.child_count = Some(1);
    let film = file("FILE::z::film", ROOT, "Film", big.len());
    let map = file("FILE::z::map", "FOLDER::z::trips", "Map", small.len());
    let apple = Arc::new(Apple {
        folders: HashMap::from([("FOLDER::z::trips".to_string(), vec![map.clone()])]),
        files: HashMap::from([
            ("FILE::z::film".to_string(), big.clone()),
            ("FILE::z::map".to_string(), small.clone()),
        ]),
        reads: Mutex::new(Vec::new()),
    });
    let library = Arc::new(Library(HashMap::from([
        (ROOT.to_string(), vec![trips, film]),
        ("FOLDER::z::trips".to_string(), vec![map]),
    ])));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (cache_dir, writes, at) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    // A budget smaller than the film: a trim would let it go, were it not kept.
    let cache = Arc::new(Cache::new(cache_dir.path()).unwrap().with_budget(CHUNK));
    let fs = fuse::DriveFs::new(
        apple.clone(),
        library,
        Arc::clone(&cache),
        rt.handle().clone(),
    )
    .with_writes(writes.path())
    .unwrap();
    let session = fuse::mount(fs, at.path()).expect("mounted");
    let (film_at, trips_at, map_at) = (
        at.path().join("Film.bin"),
        at.path().join("Trips"),
        at.path().join("Trips/Map.bin"),
    );
    let _ = std::fs::read_dir(at.path()).unwrap().count();
    let _ = std::fs::read_dir(&trips_at).unwrap().count();

    assert_eq!(get(&film_at), None, "not kept to begin with");
    assert_eq!(set(&film_at, b"1"), 0, "kept by its attribute");
    assert_eq!(get(&film_at).as_deref(), Some(&b"1"[..]));
    let whole = |id: &str| {
        apple
            .reads
            .lock()
            .unwrap()
            .iter()
            .filter(|r| *r == id)
            .count()
    };
    assert!(
        until(|| whole("FILE::z::film") == 3),
        "fetched whole, behind, every chunk once"
    );
    assert_eq!(
        std::fs::read(&film_at).unwrap(),
        big,
        "and reads back, with nothing fetched again"
    );
    assert_eq!(whole("FILE::z::film"), 3);
    cache.evict();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(std::fs::read(&film_at).unwrap(), big);
    assert_eq!(
        whole("FILE::z::film"),
        3,
        "a trim over the budget never lets a kept file go"
    );

    assert_eq!(set(&trips_at, b"1"), 0, "a folder kept");
    assert!(
        until(|| whole("FILE::z::map") == 1),
        "every file in it fetched"
    );
    assert_eq!(
        get(&map_at).as_deref(),
        Some(&b"1"[..]),
        "and shows as kept itself"
    );
    assert_ne!(
        remove(&map_at),
        0,
        "a file kept by its folder is not let go of on its own"
    );

    assert_eq!(remove(&film_at), 0, "let go of");
    assert_eq!(get(&film_at), None);
    drop(session);
}
