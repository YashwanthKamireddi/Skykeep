//! iCloud Drive in File Explorer, on a real Windows: a cloud folder registered under the user's
//! profile with a stand-in for Apple behind it, listed and read by another process, as Explorer
//! and every other app reach it. The sizes are the ones that broke other clients: none, one byte,
//! either side of a 4 KB block (Nextcloud #10845), and across our 4 MB pieces. A name Windows
//! can't hold must be there under its look-alike (#9524).
#![cfg(windows)]

use quince_core::drive::DriveItem;
use quince_vfs::cloud::{self, Provider, Shown};
use quince_vfs::{Fut, Remote, Store, ROOT};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

fn byte(i: u64) -> u8 {
    ((i * 31 + 7) % 251) as u8
}

fn file(id: &str, name: &str, ext: &str, size: i64) -> DriveItem {
    DriveItem {
        drivewsid: format!("FILE::com.apple.CloudDocs::{id}"),
        parent_id: Some(ROOT.into()),
        name: name.into(),
        extension: Some(ext.into()),
        kind: "FILE".into(),
        size: Some(size),
        child_count: None,
        created_at: Some(1_790_000_000),
        modified_at: Some(1_790_000_000),
        etag: Some("e1".into()),
    }
}

struct Apple(
    HashMap<String, Vec<DriveItem>>,
    Arc<std::sync::Mutex<Vec<String>>>,
);

impl Apple {
    fn said(&self, what: String) {
        self.1.lock().unwrap().push(what);
    }
}

impl Remote for Apple {
    fn mkdir(&self, parent: &DriveItem, name: &str) -> Fut<DriveItem> {
        self.said(format!("mkdir {name}"));
        let mut made = file(&format!("dir-{name}"), name, "", 0);
        made.drivewsid = format!("FOLDER::com.apple.CloudDocs::dir-{name}");
        made.kind = "FOLDER".into();
        made.extension = None;
        made.parent_id = Some(parent.drivewsid.clone());
        Box::pin(async move { Ok(made) })
    }
    fn upload(&self, folder: &DriveItem, path: &Path) -> Fut<DriveItem> {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let len = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
        self.said(format!("upload {name} {len}"));
        let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
        let mut sent = file(&format!("up-{name}"), stem, ext, len);
        sent.parent_id = Some(folder.drivewsid.clone());
        Box::pin(async move { Ok(sent) })
    }
    fn rename(&self, item: &DriveItem, name: &str) -> Fut<()> {
        self.said(format!("rename {} {name}", item.display_name()));
        Box::pin(async { Ok(()) })
    }
    fn move_to(&self, item: &DriveItem, folder: &DriveItem) -> Fut<()> {
        self.said(format!("move {} {}", item.display_name(), folder.name));
        Box::pin(async { Ok(()) })
    }
    fn delete(&self, item: &DriveItem) -> Fut<()> {
        self.said(format!("delete {}", item.display_name()));
        Box::pin(async { Ok(()) })
    }
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let items = self.0.get(folder).cloned().unwrap_or_default();
        Box::pin(async move { Ok(items) })
    }
    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        let size = item.size.unwrap_or(0) as u64;
        let end = (offset + len).min(size);
        Box::pin(async move {
            // As a network would: the bytes arrive a moment later, on another thread.
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            Ok((offset..end).map(byte).collect())
        })
    }
}

/// The library knows the root (as after a sync); the subfolder only Apple knows, so opening it
/// lists it from Apple, as a folder never opened before does.
struct Library(HashMap<String, Vec<DriveItem>>);

impl Store for Library {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        (folder == ROOT).then(|| self.0.get(folder).cloned().unwrap_or_default())
    }
}

/// `cmd` as another process: what it lists and what it copies out are what any app gets.
fn dir(path: &Path) -> Vec<String> {
    let out = Command::new("cmd")
        .args(["/c", "dir", "/b"])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

fn copy_out(from: &Path, to: &Path) {
    let status = Command::new("cmd")
        .args(["/c", "copy", "/b", "/y"])
        .arg(from)
        .arg(to)
        .status()
        .unwrap();
    assert!(status.success(), "copying {} out failed", from.display());
}

#[test]
fn explorer_lists_and_opens_icloud_drive() {
    let sizes: [i64; 7] = [0, 1, 4095, 4096, 4097, 8193, 9 * 1024 * 1024 + 123];
    let mut root: Vec<DriveItem> = sizes
        .iter()
        .map(|&n| file(&format!("f{n}"), &format!("size {n}"), "bin", n))
        .collect();
    root.push(file("odd", "a:b?", "txt", 5));
    root.push(file("keep", "keep", "bin", 50_000));
    let mut sub = DriveItem {
        drivewsid: "FOLDER::com.apple.CloudDocs::sub".into(),
        kind: "FOLDER".into(),
        name: "Trip".into(),
        extension: None,
        size: None,
        ..file("x", "x", "x", 0)
    };
    sub.parent_id = Some(ROOT.into());
    root.push(sub.clone());
    let inner = vec![file("in", "inside", "bin", 70_000)];
    let tree: HashMap<String, Vec<DriveItem>> =
        [(ROOT.to_string(), root), (sub.drivewsid.clone(), inner)].into();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep test {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let log = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let id = cloud::sync_root_id(&format!("test{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let folder = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(Apple(tree.clone(), Arc::clone(&log))),
            Arc::new(Library(tree)),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("the cloud folder registers and connects");
    println!("registered {} (shell: {})", id, folder.shell);

    let listed = dir(&place);
    for n in sizes {
        assert!(
            listed.contains(&format!("size {n}.bin")),
            "size {n}.bin missing from {listed:?}"
        );
    }
    assert!(
        listed.contains(&"a：b？.txt".to_string()),
        "the odd name is missing from {listed:?}"
    );
    assert!(listed.contains(&"Trip".to_string()));

    for n in sizes {
        let copy = out.join(format!("{n}.bin"));
        copy_out(&place.join(format!("size {n}.bin")), &copy);
        let got = std::fs::read(&copy).unwrap();
        assert_eq!(got.len() as i64, n, "size {n}: wrong length");
        assert!(
            got.iter().enumerate().all(|(i, &b)| b == byte(i as u64)),
            "size {n}: wrong bytes"
        );
    }
    // A folder never listed: from Apple, then its file opens.
    assert_eq!(dir(&place.join("Trip")), ["inside.bin"]);
    let copy = out.join("inside.bin");
    copy_out(&place.join("Trip").join("inside.bin"), &copy);
    assert_eq!(std::fs::read(&copy).unwrap().len(), 70_000);

    // Changes made here, as a person makes them, reach Apple: each waited for (a save settles
    // for two seconds before it is sent).
    let until = |what: &str| {
        for _ in 0..150 {
            if log.lock().unwrap().iter().any(|l| l.starts_with(what)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        panic!(
            "never reached Apple: {what}; Apple heard {:?}",
            log.lock().unwrap()
        );
    };
    let cmd = |line: &str| {
        // Verbatim: cmd reads quotes its own way, and an argument escaped for C (\\") is not it.
        use std::os::windows::process::CommandExt;
        let ok = Command::new("cmd")
            .arg("/c")
            .raw_arg(line)
            .current_dir(&place)
            .status()
            .unwrap()
            .success();
        assert!(ok, "{line}");
    };
    // "Free up space" on a file already here, and "Always keep on this device" on one never
    // opened: the marks Explorer's menu sets, carried out (bytes let go; every byte fetched).
    use std::os::windows::fs::MetadataExt;
    let not_all_here = |name: &str| {
        std::fs::metadata(place.join(name))
            .unwrap()
            .file_attributes()
            & 0x0040_0000
            != 0
    };
    let wait_for = |what: &str, done: &dyn Fn() -> bool| {
        for _ in 0..150 {
            if done() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        panic!("{what} never happened");
    };
    assert!(
        !not_all_here("size 8193.bin"),
        "size 8193.bin was read, so it is here"
    );
    cmd("attrib +U -P \"size 8193.bin\"");
    wait_for("freeing size 8193.bin", &|| not_all_here("size 8193.bin"));
    assert!(not_all_here("keep.bin"), "keep.bin was never opened");
    cmd("attrib +P -U keep.bin");
    wait_for("keeping keep.bin", &|| !not_all_here("keep.bin"));

    cmd("echo hello> new.txt");
    until("upload new.txt");
    cmd("mkdir NewFolder && echo inside> NewFolder\\a.txt");
    until("mkdir NewFolder");
    until("upload a.txt");
    cmd("echo more>> \"size 4096.bin\"");
    until("upload size 4096.bin");
    until("delete size 4096.bin");
    cmd("ren \"size 1.bin\" renamed.bin");
    until("rename size 1.bin renamed.bin");
    cmd("del \"size 4095.bin\"");
    until("delete size 4095.bin");
    // Nothing a person didn't do: no delete of a file that is still there.
    assert!(
        !log.lock().unwrap().iter().any(|l| l == "delete size 0.bin"),
        "{:?}",
        log.lock().unwrap()
    );
    println!("Apple heard: {:?}", log.lock().unwrap());

    drop(folder);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}
