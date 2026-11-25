//! iCloud Drive in File Explorer, on a real Windows: a cloud folder registered under the user's
//! profile with a stand-in for Apple behind it, listed and read by another process, as Explorer
//! and every other app reach it. The sizes are the ones that broke other clients: none, one byte,
//! either side of a 4 KB block (Nextcloud #10845), and across our 4 MB pieces. A name Windows
//! can't hold must be there under its look-alike (#9524).
#![cfg(windows)]

use quince_core::drive::DriveItem;
use quince_vfs::cloud::{self, Provider, Shown};
use quince_vfs::{Arriving, Fut, Landing, Remote, Store, ROOT};
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
        share: None,
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
    /// New contents into the same file, as Apple's modify_file writes them (#24).
    fn replace(&self, item: &DriveItem, path: &Path) -> Fut<Option<DriveItem>> {
        let len = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
        self.said(format!("replace {} {len}", item.display_name()));
        let mut now = item.clone();
        now.size = Some(len);
        now.etag = Some(format!("{}+", item.etag.clone().unwrap_or_default()));
        Box::pin(async move { Ok(Some(now)) })
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
struct Library(Arc<std::sync::Mutex<HashMap<String, Vec<DriveItem>>>>);

impl Store for Library {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        (folder == ROOT).then(|| {
            self.0
                .lock()
                .unwrap()
                .get(folder)
                .cloned()
                .unwrap_or_default()
        })
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
    let library = Arc::new(std::sync::Mutex::new(tree.clone()));
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
            Arc::new(Library(Arc::clone(&library))),
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

    // Changes made at Apple reach a folder Explorer has listed: a new file appears, one gone
    // from Apple goes from here, one changed lets its old bytes go (never opened stale).
    {
        let mut lib = library.lock().unwrap();
        let root = lib.get_mut(ROOT).unwrap();
        root.retain(|i| i.name != "size 0");
        root.push(file("fresh", "fresh", "bin", 10));
        let changed = root.iter_mut().find(|i| i.name == "size 4097").unwrap();
        changed.etag = Some("e2".into());
        changed.size = Some(5000);
    }
    assert!(
        !not_all_here("size 4097.bin"),
        "size 4097.bin was read, so it is here"
    );
    folder.refresh(ROOT);
    let listed = dir(&place);
    assert!(
        listed.contains(&"fresh.bin".to_string()),
        "fresh.bin missing from {listed:?}"
    );
    assert!(
        !listed.contains(&"size 0.bin".to_string()),
        "size 0.bin still in {listed:?}"
    );
    assert!(
        not_all_here("size 4097.bin"),
        "the changed file kept its old bytes"
    );
    assert_eq!(
        std::fs::metadata(place.join("size 4097.bin"))
            .unwrap()
            .len(),
        5000
    );

    cmd("echo hello> new.txt");
    until("upload new.txt");
    cmd("mkdir NewFolder && echo inside> NewFolder\\a.txt");
    until("mkdir NewFolder");
    until("upload a.txt");
    // Saved over: written into the same file at Apple (#24) — never a second file uploaded
    // beside it with the old one deleted.
    cmd("echo more>> \"size 4096.bin\"");
    until("replace size 4096.bin");
    cmd("ren \"size 1.bin\" renamed.bin");
    until("rename size 1.bin renamed.bin");
    cmd("del \"size 4095.bin\"");
    until("delete size 4095.bin");
    assert!(
        !log.lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("upload size 4096.bin") || l == "delete size 4096.bin"),
        "a save over a file is no upload and delete: {:?}",
        log.lock().unwrap()
    );
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

/// A 1×1 JPEG: a real picture, so a thumbnail could be drawn from it if it were fetched.
const JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08,
    0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
    0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20,
    0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27,
    0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01,
    0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x14, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, 0xFF, 0xC4, 0x00, 0x14,
    0x10, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x54, 0xDF, 0xFF, 0xD9,
];

/// iCloud Photos as the Photos folder makes it (Library, then photos), each read whole from the
/// app's photo cache, only ever added to or deleted from.
struct Photos(
    Arc<std::sync::Mutex<Vec<String>>>,
    PathBuf,
    HashMap<String, Vec<DriveItem>>,
);

impl Remote for Photos {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let items = self.2.get(folder).cloned().unwrap_or_default();
        Box::pin(async move { Ok(items) })
    }
    fn read(&self, _: &DriveItem, _: u64, _: u64) -> Fut<Vec<u8>> {
        Box::pin(async { Err("read through the photo cache".to_string()) })
    }
    fn arriving(&self, item: &DriveItem) -> Option<Fut<Arriving>> {
        self.0
            .lock()
            .unwrap()
            .push(format!("fetch {}", item.display_name()));
        let path = self.1.clone();
        Some(Box::pin(async move { Ok(Arriving::Whole(path)) }))
    }
    fn accepts(&self, folder: &DriveItem) -> bool {
        folder.drivewsid == "PH::library"
    }
    fn upload(&self, folder: &DriveItem, path: &Path) -> Fut<DriveItem> {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        self.0.lock().unwrap().push(format!("upload {name}"));
        let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
        let mut sent = file(&format!("up-{name}"), stem, ext, JPEG.len() as i64);
        sent.drivewsid = format!("PHOTO::up-{name}");
        sent.parent_id = Some(folder.drivewsid.clone());
        Box::pin(async move { Ok(sent) })
    }
    fn delete(&self, item: &DriveItem) -> Fut<()> {
        self.0
            .lock()
            .unwrap()
            .push(format!("delete {}", item.display_name()));
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn explorer_shows_icloud_photos() {
    let photo = |id: &str, name: &str| {
        let mut p = file(id, name, "JPG", JPEG.len() as i64);
        p.drivewsid = format!("PHOTO::{id}");
        p.parent_id = Some("PH::library".into());
        p
    };
    let mut library = DriveItem {
        drivewsid: "PH::library".into(),
        kind: "FOLDER".into(),
        name: "Library".into(),
        extension: None,
        size: None,
        ..file("x", "x", "x", 0)
    };
    library.parent_id = Some(ROOT.into());
    let tree: HashMap<String, Vec<DriveItem>> = [
        (ROOT.to_string(), vec![library.clone()]),
        (
            "PH::library".to_string(),
            vec![
                photo("a", "IMG_0001"),
                photo("b", "IMG_0002"),
                photo("c", "IMG_0003"),
            ],
        ),
    ]
    .into();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep photos test {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-photos-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let cached = out.join("cached.jpg");
    std::fs::write(&cached, JPEG).unwrap();
    let log = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let id = cloud::sync_root_id(&format!("photos{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep photos test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let folder = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(Photos(Arc::clone(&log), cached, tree.clone())),
            Arc::new(Library(Arc::new(std::sync::Mutex::new(tree)))),
            rt.handle().clone(),
            out.join("sending"),
        )
        .add_only(),
    )
    .expect("the photos folder registers and connects");
    println!("registered {} (shell: {})", id, folder.shell);

    // Who may write the folder's own SyncRootManager key, where a thumbnail provider is named:
    // printed, the fact the thumbnail provider's registration is built on.
    let acl = Command::new("powershell")
        .env_remove("PSModulePath")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                r"$k = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager\{id}'; (Get-Acl $k).Access | Format-Table IdentityReference,RegistryRights,AccessControlType,IsInherited -AutoSize | Out-String -Width 220; 'HKCU twin: ' + (Test-Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\SyncRootManager')"
            ),
        ])
        .output()
        .unwrap();
    println!(
        "sync root key access:\n{}{}",
        String::from_utf8_lossy(&acl.stdout),
        String::from_utf8_lossy(&acl.stderr)
    );

    let lib = place.join("Library");
    assert_eq!(dir(&place), ["Library"]);
    assert_eq!(dir(&lib), ["IMG_0001.JPG", "IMG_0002.JPG", "IMG_0003.JPG"]);
    let copy = out.join("one.jpg");
    copy_out(&lib.join("IMG_0001.JPG"), &copy);
    assert_eq!(std::fs::read(&copy).unwrap(), JPEG);

    // Windows' own thumbnail of a photo never opened, asked by another process as Explorer asks:
    // whether that fetches the photo is printed, the fact the thumbnail provider is built on.
    let thumb = format!(
        r#"Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
[ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IShellItemImageFactory {{ [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr bitmap); }}
[StructLayout(LayoutKind.Sequential)] public struct SIZE {{ public int cx; public int cy; }}
public static class Thumb {{
  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHCreateItemFromParsingName(string path, IntPtr bc, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory item);
  public static int Of(string path) {{ IShellItemImageFactory f; SHCreateItemFromParsingName(path, IntPtr.Zero, typeof(IShellItemImageFactory).GUID, out f); IntPtr h; return f.GetImage(new SIZE {{ cx = 96, cy = 96 }}, 8, out h); }}
}}
'@
[Thumb]::Of('{}')"#,
        lib.join("IMG_0002.JPG").display()
    );
    let before = log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.contains("IMG_0002"))
        .count();
    let asked = Command::new("powershell")
        .args(["-NoProfile", "-Command", &thumb])
        .output()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let fetched = log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.contains("IMG_0002"))
        .count()
        > before;
    println!(
        "thumbnail of a photo never opened: answer {:?}, fetched the photo: {fetched}",
        String::from_utf8_lossy(&asked.stdout).trim()
    );

    // Skykeep's thumbnail provider (crates/quince-thumbs), built beside this test by the
    // workflow: registered for this folder, answered over its pipe with a stand-in picture,
    // and asked as Explorer asks. The thumbnail comes back; the photo is never fetched.
    if let Some(dll) = std::env::var_os("SKYKEEP_THUMBS_DLL").map(PathBuf::from) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let clsid = format!("{:?}", quince_thumbs::CLSID);
        cloud::add_thumbnails(&id, &clsid, &dll).expect("the thumbnail provider registers");
        let asked_for = Arc::new(std::sync::Mutex::new(Vec::<(u32, String)>::new()));
        let heard = Arc::clone(&asked_for);
        let name = quince_thumbs::protocol::pipe_name(quince_thumbs::this_session().unwrap());
        rt.spawn(async move {
            let mut first = true;
            loop {
                let mut pipe = {
                    let mut security = quince_thumbs::PipeSecurity::new().unwrap();
                    // SAFETY: the security attributes live until the pipe is made.
                    unsafe {
                        tokio::net::windows::named_pipe::ServerOptions::new()
                            .first_pipe_instance(first)
                            .create_with_security_attributes_raw(&name, security.as_raw())
                    }
                    .unwrap()
                };
                first = false;
                pipe.connect().await.unwrap();
                let mut head = [0u8; 8];
                pipe.read_exact(&mut head).await.unwrap();
                let (size, units) = quince_thumbs::protocol::request_head(head).unwrap();
                let mut body = vec![0u8; units as usize * 2];
                pipe.read_exact(&mut body).await.unwrap();
                let path = String::from_utf16_lossy(&quince_thumbs::protocol::path_of(&body));
                heard.lock().unwrap().push((size, path));
                let px = [0x20u8, 0x80, 0xE0, 0xFF].repeat(64 * 48);
                let reply = quince_thumbs::protocol::reply(Some((64, 48, &px)));
                pipe.write_all(&reply).await.unwrap();
                let _ = pipe.read(&mut [0u8; 1]).await;
            }
        });
        let before = log.lock().unwrap().len();
        // The provider itself first, loaded through COM in this process: a fault of the DLL or
        // the pipe is never mistaken for one of the shell's.
        // SAFETY: COM on this thread, its objects released before the test goes on.
        unsafe {
            use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP};
            use windows::Win32::System::Com::*;
            use windows::Win32::UI::Shell::*;
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let item: IShellItem = SHCreateItemFromParsingName(
                &windows::core::HSTRING::from(lib.join("IMG_0001.JPG").as_os_str()),
                None,
            )
            .unwrap();
            let init: IInitializeWithItem =
                CoCreateInstance(&quince_thumbs::CLSID, None, CLSCTX_INPROC_SERVER).unwrap();
            init.Initialize(&item, 0).unwrap();
            let provider: IThumbnailProvider = windows::core::Interface::cast(&init).unwrap();
            let mut bitmap = HBITMAP::default();
            let mut alpha = WTSAT_UNKNOWN;
            provider
                .GetThumbnail(96, &mut bitmap, &mut alpha)
                .expect("Skykeep's provider draws a thumbnail");
            let _ = DeleteObject(bitmap.into());
        }
        assert!(
            asked_for
                .lock()
                .unwrap()
                .iter()
                .any(|(_, p)| p.ends_with("IMG_0001.JPG")),
            "the provider asked Skykeep over the pipe"
        );
        // Send to iPhone, the command Windows 11's menu shows from the MSIX package: the same
        // DLL, its title, state and icon answered at once.
        let menu = format!(
            r"HKCU\Software\Classes\CLSID\{{{:?}}}",
            quince_thumbs::SEND_TO_IPHONE_CLSID
        );
        assert!(Command::new("reg")
            .args([
                "add",
                &format!(r"{menu}\InprocServer32"),
                "/ve",
                "/d",
                &dll.display().to_string(),
                "/f"
            ])
            .status()
            .unwrap()
            .success());
        // SAFETY: COM on this thread, as above; each string freed once read.
        unsafe {
            use windows::Win32::System::Com::*;
            use windows::Win32::UI::Shell::*;
            let command: IExplorerCommand = CoCreateInstance(
                &quince_thumbs::SEND_TO_IPHONE_CLSID,
                None,
                CLSCTX_INPROC_SERVER,
            )
            .unwrap();
            let title = command.GetTitle(None).unwrap();
            assert_eq!(title.to_string().unwrap(), "Send to iPhone");
            CoTaskMemFree(Some(title.0 as *const _));
            let icon = command.GetIcon(None).unwrap();
            assert!(icon.to_string().unwrap().ends_with(r"\skykeep.exe,0"));
            CoTaskMemFree(Some(icon.0 as *const _));
            assert_eq!(command.GetState(None, false).unwrap(), ECS_ENABLED.0 as u32);
        }
        let _ = Command::new("reg").args(["delete", &menu, "/f"]).status();
        // The provider's own trace, on for this ask only: where the shell's way stops, if it does.
        let trace = PathBuf::from(std::env::var("USERPROFILE").unwrap())
            .join(r"AppData\LocalLow\Skykeep thumbnails trace");
        std::fs::create_dir_all(&trace).unwrap();
        let asked = Command::new("powershell")
            .env_remove("PSModulePath")
            .args([
                "-NoProfile",
                "-Command",
                &thumb.replace("IMG_0002", "IMG_0003"),
            ])
            .output()
            .unwrap();
        let answer = String::from_utf8_lossy(&asked.stdout).trim().to_string();
        println!(
            "provider trace:\n{}",
            std::fs::read_to_string(trace.join("trace.txt")).unwrap_or_default()
        );
        let _ = std::fs::remove_dir_all(&trace);
        // Where the shell ran the provider: in the asking process, or isolated in another.
        let hosts = Command::new("tasklist")
            .args(["/m", "quince_thumbs.dll", "/fo", "csv"])
            .output()
            .unwrap();
        println!(
            "processes with the provider loaded:\n{}",
            String::from_utf8_lossy(&hosts.stdout)
        );
        println!(
            "thumbnail through Skykeep's provider: answer {answer:?}, asked {:?} {}",
            asked_for.lock().unwrap(),
            String::from_utf8_lossy(&asked.stderr)
        );
        assert_eq!(answer, "0", "Explorer drew Skykeep's thumbnail");
        let asked_for = asked_for.lock().unwrap();
        assert!(
            asked_for.iter().any(|(_, p)| p.ends_with("IMG_0003.JPG")),
            "the provider asked Skykeep for that photo"
        );
        assert!(
            !log.lock().unwrap()[before..]
                .iter()
                .any(|l| l.contains("IMG_0003")),
            "a thumbnail never downloads the photo"
        );
    }

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
        use std::os::windows::process::CommandExt;
        Command::new("cmd")
            .arg("/c")
            .raw_arg(line)
            .current_dir(&lib)
            .status()
            .unwrap()
            .success()
    };
    // A photo saved over (an editor writing into it): sent once as a new photo, and the one
    // under it back to iCloud's own bytes.
    assert!(cmd("echo edited>> IMG_0001.JPG"));
    until("upload IMG_0001.JPG");
    let mut back = false;
    for _ in 0..150 {
        if std::fs::metadata(lib.join("IMG_0001.JPG"))
            .map(|m| m.len())
            .ok()
            == Some(JPEG.len() as u64)
        {
            back = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(back, "the original never went back to iCloud's copy");
    copy_out(&lib.join("IMG_0001.JPG"), &copy);
    assert_eq!(std::fs::read(&copy).unwrap(), JPEG, "the original's bytes");
    std::thread::sleep(std::time::Duration::from_secs(4));
    let sends = log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.starts_with("upload IMG_0001.JPG"))
        .count();
    assert_eq!(sends, 1, "one save, one new photo");

    std::fs::write(out.join("new.jpg"), JPEG).unwrap();
    assert!(cmd(&format!(
        "copy /b /y \"{}\" new.jpg",
        out.join("new.jpg").display()
    )));
    until("upload new.jpg");
    // A photo has no name of its own: the rename is declined and nothing reaches Apple.
    assert!(!cmd("ren IMG_0003.JPG renamed.JPG"), "a photo was renamed");
    assert!(lib.join("IMG_0003.JPG").exists());
    assert!(cmd("del IMG_0003.JPG"));
    until("delete IMG_0003.JPG");
    assert!(!log.lock().unwrap().iter().any(|l| l.starts_with("rename")));
    println!("Apple heard: {:?}", log.lock().unwrap());

    drop(folder);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// Skykeep quit and started again (#44, #45): the folder, registered and listed by the first
/// run, is taken up again by the second, and a folder never opened before opens — the case
/// File Explorer answered "The cloud operation is invalid" when the second run could not
/// connect.
#[test]
fn a_restart_takes_the_folder_back_up() {
    let mut sub = DriveItem {
        drivewsid: "FOLDER::com.apple.CloudDocs::later".into(),
        kind: "FOLDER".into(),
        name: "Later".into(),
        extension: None,
        size: None,
        ..file("x", "x", "x", 0)
    };
    sub.parent_id = Some(ROOT.into());
    let root = vec![file("a", "first", "bin", 10), sub.clone()];
    let inner: Vec<DriveItem> = (0..600)
        .map(|n| {
            let mut f = file(&format!("in{n}"), &format!("inside {n}"), "bin", 10);
            f.parent_id = Some(sub.drivewsid.clone());
            f
        })
        .collect();
    let tree: HashMap<String, Vec<DriveItem>> = [
        (ROOT.to_string(), root.clone()),
        (sub.drivewsid.clone(), inner),
    ]
    .into();
    let library = Arc::new(std::sync::Mutex::new(HashMap::from([(
        ROOT.to_string(),
        root,
    )])));
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep restart {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-restart-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("restart{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep restart test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let provider = || {
        Provider::new(
            Arc::new(Apple(tree.clone(), Arc::clone(&log))),
            Arc::new(Library(Arc::clone(&library))),
            rt.handle().clone(),
            out.join("sending"),
        )
    };
    let first =
        cloud::start(&place, &shown, provider()).expect("the first run takes the folder up");
    assert!(dir(&place).contains(&"first.bin".to_string()));
    drop(first);
    let second = cloud::start(&place, &shown, provider()).expect("the next run takes it up again");
    let listed = dir(&place.join("Later"));
    assert_eq!(
        listed.len(),
        600,
        "a folder never opened opens after a restart, whole: {} of 600",
        listed.len()
    );
    drop(second);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// A folder already here under a name Apple's root also has (another client's, or a run that
/// ended part way) is left as it is, and the root still opens whole; and a listing Apple fails
/// once is asked for again on the next open, never marked whole with nothing in it (#44).
#[test]
fn a_clash_or_a_failed_listing_never_breaks_a_folder() {
    struct Flaky(Apple, std::sync::atomic::AtomicUsize);
    impl Remote for Flaky {
        fn mkdir(&self, p: &DriveItem, n: &str) -> Fut<DriveItem> {
            self.0.mkdir(p, n)
        }
        fn upload(&self, f: &DriveItem, p: &Path) -> Fut<DriveItem> {
            self.0.upload(f, p)
        }
        fn replace(&self, i: &DriveItem, p: &Path) -> Fut<Option<DriveItem>> {
            self.0.replace(i, p)
        }
        fn rename(&self, i: &DriveItem, n: &str) -> Fut<()> {
            self.0.rename(i, n)
        }
        fn move_to(&self, i: &DriveItem, f: &DriveItem) -> Fut<()> {
            self.0.move_to(i, f)
        }
        fn delete(&self, i: &DriveItem) -> Fut<()> {
            self.0.delete(i)
        }
        fn read(&self, i: &DriveItem, o: u64, l: u64) -> Fut<Vec<u8>> {
            self.0.read(i, o, l)
        }
        fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
            use std::sync::atomic::Ordering::SeqCst;
            if folder != ROOT && self.1.load(SeqCst) > 0 {
                self.1.fetch_sub(1, SeqCst);
                return Box::pin(async { Err("Apple did not answer".to_string()) });
            }
            self.0.list(folder)
        }
    }
    /// What `cmd /c dir` lists, and Windows' error text if it refused.
    fn listed(path: &Path) -> (Vec<String>, String) {
        let out = Command::new("cmd")
            .args(["/c", "dir", "/b"])
            .arg(path)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(str::to_string)
                .collect(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )
    }
    let mut sub = DriveItem {
        drivewsid: "FOLDER::com.apple.CloudDocs::vault".into(),
        kind: "FOLDER".into(),
        name: "Vault".into(),
        extension: None,
        size: None,
        ..file("x", "x", "x", 0)
    };
    sub.parent_id = Some(ROOT.into());
    let other = DriveItem {
        drivewsid: "FOLDER::com.apple.CloudDocs::notes".into(),
        name: "Notes".into(),
        ..sub.clone()
    };
    let root = vec![file("a", "first", "bin", 10), sub.clone(), other.clone()];
    let inner: Vec<DriveItem> = (0..3)
        .map(|n| {
            let mut f = file(&format!("v{n}"), &format!("note {n}"), "md", 10);
            f.parent_id = Some(other.drivewsid.clone());
            f
        })
        .collect();
    let tree: HashMap<String, Vec<DriveItem>> = [
        (ROOT.to_string(), root.clone()),
        (other.drivewsid.clone(), inner),
    ]
    .into();
    let library = Arc::new(std::sync::Mutex::new(HashMap::from([(
        ROOT.to_string(),
        root,
    )])));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep clash {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-clash-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    // A plain folder of the same name, with a file of the person's in it.
    std::fs::create_dir_all(place.join("Vault")).unwrap();
    std::fs::write(place.join("Vault").join("mine.md"), b"mine").unwrap();
    let id = cloud::sync_root_id(&format!("clash{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep clash test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let up = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(Flaky(
                Apple(tree, Arc::new(std::sync::Mutex::new(Vec::new()))),
                std::sync::atomic::AtomicUsize::new(1),
            )),
            Arc::new(Library(Arc::clone(&library))),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("the folder comes up");
    let (names, said) = listed(&place);
    assert!(said.is_empty(), "the root opens despite the clash: {said}");
    for name in ["first.bin", "Vault", "Notes"] {
        assert!(
            names.contains(&name.to_string()),
            "{name} listed: {names:?}"
        );
    }
    assert!(
        listed(&place.join("Vault"))
            .0
            .contains(&"mine.md".to_string()),
        "the person's own folder is left as it was"
    );
    // Apple fails the first listing: that open fails, the next one lists it whole.
    let first = listed(&place.join("Notes"));
    let (names, said) = listed(&place.join("Notes"));
    assert!(
        said.is_empty() && names.len() == 3,
        "a folder whose first listing failed opens whole next time: {names:?} {said:?} (first: {first:?})"
    );
    drop(up);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// A folder opened in an earlier run is still brought up to date after a restart: Windows never
/// asks for a folder it already has whole, so one known only from this run's opens was left
/// stale for good — a note changed on the Mac never reached it.
#[test]
fn a_folder_opened_before_a_restart_stays_up_to_date() {
    struct Known(Arc<std::sync::Mutex<HashMap<String, Vec<DriveItem>>>>);
    impl Store for Known {
        fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
            self.0.lock().unwrap().get(folder).cloned()
        }
    }
    let mut sub = DriveItem {
        drivewsid: "FOLDER::com.apple.CloudDocs::vault".into(),
        kind: "FOLDER".into(),
        name: "Vault".into(),
        extension: None,
        size: None,
        ..file("x", "x", "x", 0)
    };
    sub.parent_id = Some(ROOT.into());
    let note = |n: &str| {
        let mut f = file(n, n, "md", 10);
        f.parent_id = Some(sub.drivewsid.clone());
        f
    };
    let root = vec![sub.clone()];
    let library = Arc::new(std::sync::Mutex::new(HashMap::from([
        (ROOT.to_string(), root.clone()),
        (sub.drivewsid.clone(), vec![note("one")]),
    ])));
    let tree: HashMap<String, Vec<DriveItem>> = library.lock().unwrap().clone();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep fresh {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-fresh-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("fresh{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep fresh test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let provider = || {
        Provider::new(
            Arc::new(Apple(
                tree.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )),
            Arc::new(Known(Arc::clone(&library))),
            rt.handle().clone(),
            out.join("sending"),
        )
    };
    let first = cloud::start(&place, &shown, provider()).expect("up");
    assert_eq!(dir(&place.join("Vault")), vec!["one.md".to_string()]);
    drop(first);
    let second = cloud::start(&place, &shown, provider()).expect("up again");
    // Changed at Apple after the restart: a second note in the folder opened before it.
    library
        .lock()
        .unwrap()
        .insert(sub.drivewsid.clone(), vec![note("one"), note("two")]);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut seen = Vec::new();
    while std::time::Instant::now() < end {
        second.refresh(&sub.drivewsid);
        seen = dir(&place.join("Vault"));
        if seen.contains(&"two.md".to_string()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(
        seen.contains(&"two.md".to_string()),
        "a folder opened before the restart shows Apple's change: {seen:?}"
    );
    drop(second);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// An empty folder opens in File Explorer, empty (QQueng, #44: every empty folder, in iCloud
/// Drive and in iCloud Photos, failed with "The cloud operation is invalid").
#[test]
fn an_empty_folder_opens() {
    struct Known(HashMap<String, Vec<DriveItem>>);
    impl Store for Known {
        fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
            self.0.get(folder).cloned()
        }
    }
    let mut empty = DriveItem {
        drivewsid: "FOLDER::com.apple.CloudDocs::empty".into(),
        kind: "FOLDER".into(),
        name: "Empty".into(),
        extension: None,
        size: None,
        child_count: Some(0),
        ..file("x", "x", "x", 0)
    };
    empty.parent_id = Some(ROOT.into());
    let tree: HashMap<String, Vec<DriveItem>> = [
        (
            ROOT.to_string(),
            vec![file("a", "first", "bin", 10), empty.clone()],
        ),
        (empty.drivewsid.clone(), Vec::new()),
    ]
    .into();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep empty {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-empty-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("empty{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep empty test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let up = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(Apple(
                tree.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )),
            Arc::new(Known(tree)),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("up");
    let out_cmd = Command::new("cmd")
        .args(["/c", "dir", "/b"])
        .arg(place.join("Empty"))
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out_cmd.stderr).trim().to_string();
    let listed = String::from_utf8_lossy(&out_cmd.stdout).trim().to_string();
    println!("REPORT empty folder: listed [{listed}] said [{said}]");
    // An empty folder: `dir /b` says "File Not Found" for no entries, never a cloud error.
    assert!(
        !said.to_lowercase().contains("cloud"),
        "an empty folder opens without a cloud error: {said}"
    );
    // And Windows holds it whole: opened again, it is not asked for again (no hang).
    let again = Command::new("cmd")
        .args(["/c", "dir", "/b"])
        .arg(place.join("Empty"))
        .output()
        .unwrap();
    println!(
        "REPORT opened again: said [{}]",
        String::from_utf8_lossy(&again.stderr).trim()
    );
    drop(up);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// A file deeper than Windows' old 260-character limit is sent to iCloud and then marked in
/// sync like any other. Our own calls into Windows (CreateFileW and the rest) refused such a
/// path without the `\\?\` prefix: the file went to iCloud but stayed "Sync pending" for good
/// (an Obsidian vault's plugin folders are that deep).
#[test]
fn a_file_deeper_than_260_characters_is_kept_in_sync() {
    let root = vec![file("a", "first", "bin", 10)];
    let tree: HashMap<String, Vec<DriveItem>> = [(ROOT.to_string(), root.clone())].into();
    let library = Arc::new(std::sync::Mutex::new(HashMap::from([(
        ROOT.to_string(),
        root,
    )])));
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep deep {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-deep-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("deep{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep deep test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let up = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(Apple(tree, Arc::clone(&log))),
            Arc::new(Library(Arc::clone(&library))),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("up");
    // Five folders of 60 characters each: well past 260 with the file's own name.
    let mut deep = place.clone();
    for n in 0..5 {
        deep = deep.join(format!("{n}-{}", "x".repeat(58)));
    }
    std::fs::create_dir_all(&deep).unwrap();
    let file = deep.join("deep note.md");
    std::fs::write(&file, b"far down").unwrap();
    assert!(file.as_os_str().len() > 300, "{}", file.as_os_str().len());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut kept = None;
    while std::time::Instant::now() < end {
        kept = cloud::identity_of(&file);
        if kept.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let said = log.lock().unwrap().clone();
    assert!(
        said.iter().any(|s| s.starts_with("upload deep note.md")),
        "sent to iCloud: {said:?}"
    );
    assert!(
        kept.is_some(),
        "and marked in sync, its identity kept on it: {said:?}"
    );
    drop(up);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// A Drive folder put where Windows keeps no cloud folder is refused by name, not with the
/// registration's error: on this PC's NTFS drive it is taken, on a network share it is not.
#[test]
fn a_folder_off_a_local_ntfs_drive_is_refused_plainly() {
    assert_eq!(
        cloud::volume_refusal(&std::env::temp_dir()),
        None,
        "this PC's own drive"
    );
    // The runner's own C: reached as a share: what a mapped network drive is.
    let share = Path::new(r"\\localhost\C$\Windows");
    let why = cloud::volume_refusal(share).expect("a network share is refused");
    assert!(
        why.contains("network drive") && why.contains(r"\\localhost\C$\Windows"),
        "{why}"
    );
}

/// A file saved again while its last save was still going to Apple goes over that version, which
/// is ours: never beside it as a conflicted copy. Obsidian saves a note every few seconds while
/// it is typed in, and each save that landed during an upload made a new "(conflicted copy …)"
/// beside it (#169): the second send named the version before the first as the one it replaced.
#[test]
fn a_save_during_an_upload_makes_no_conflicted_copy() {
    /// Apple as it is: what was sent shows in the next listing, a replacement is a new version,
    /// and an upload takes long enough for the app to save again meanwhile.
    struct Live(
        std::sync::Mutex<HashMap<String, Vec<DriveItem>>>,
        Arc<std::sync::Mutex<Vec<String>>>,
    );
    impl Remote for Live {
        fn mkdir(&self, _: &DriveItem, _: &str) -> Fut<DriveItem> {
            Box::pin(async { Err("no folders here".to_string()) })
        }
        fn upload(&self, folder: &DriveItem, path: &Path) -> Fut<DriveItem> {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let len = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
            self.1.lock().unwrap().push(format!("upload {name} {len}"));
            let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
            let mut sent = file(&format!("up-{name}"), stem, ext, len);
            sent.parent_id = Some(folder.drivewsid.clone());
            self.0
                .lock()
                .unwrap()
                .entry(folder.drivewsid.clone())
                .or_default()
                .push(sent.clone());
            Box::pin(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                Ok(sent)
            })
        }
        fn replace(&self, item: &DriveItem, path: &Path) -> Fut<Option<DriveItem>> {
            let len = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
            self.1
                .lock()
                .unwrap()
                .push(format!("replace {} {len}", item.display_name()));
            let mut now = item.clone();
            now.size = Some(len);
            now.etag = Some(format!("{}+", item.etag.clone().unwrap_or_default()));
            for items in self.0.lock().unwrap().values_mut() {
                for i in items.iter_mut().filter(|i| i.drivewsid == now.drivewsid) {
                    *i = now.clone();
                }
            }
            Box::pin(async move { Ok(Some(now)) })
        }
        fn rename(&self, _: &DriveItem, _: &str) -> Fut<()> {
            Box::pin(async { Ok(()) })
        }
        fn move_to(&self, _: &DriveItem, _: &DriveItem) -> Fut<()> {
            Box::pin(async { Ok(()) })
        }
        fn delete(&self, _: &DriveItem) -> Fut<()> {
            Box::pin(async { Ok(()) })
        }
        fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
            let items = self
                .0
                .lock()
                .unwrap()
                .get(folder)
                .cloned()
                .unwrap_or_default();
            Box::pin(async move { Ok(items) })
        }
        fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
            let end = (offset + len).min(item.size.unwrap_or(0) as u64);
            Box::pin(async move { Ok((offset..end).map(byte).collect()) })
        }
    }
    struct Known(HashMap<String, Vec<DriveItem>>);
    impl Store for Known {
        fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
            self.0.get(folder).cloned()
        }
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep saves {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-saves-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let log = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let apple = Arc::new(Live(
        std::sync::Mutex::new([(ROOT.to_string(), Vec::new())].into()),
        Arc::clone(&log),
    ));
    let id = cloud::sync_root_id(&format!("saves{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep saves",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let folder = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::clone(&apple) as Arc<dyn Remote>,
            Arc::new(Known([(ROOT.to_string(), Vec::new())].into())),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("the cloud folder registers and connects");
    let heard = |what: &str| log.lock().unwrap().iter().any(|l| l.starts_with(what));
    let wait = |what: &str| {
        for _ in 0..150 {
            if heard(what) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        panic!(
            "never reached Apple: {what}; Apple heard {:?}",
            log.lock().unwrap()
        );
    };

    let note = place.join("note.md");
    std::fs::write(&note, "a").unwrap();
    // Saved again while the first save is on its way (the upload takes three seconds).
    wait("upload note.md");
    std::thread::sleep(std::time::Duration::from_millis(500));
    std::fs::write(&note, "abc").unwrap();
    wait("replace note.md 3");
    // And once more after that, as typing goes on.
    std::fs::write(&note, "abcdef").unwrap();
    wait("replace note.md 6");
    std::thread::sleep(std::time::Duration::from_secs(3));

    let said = log.lock().unwrap().clone();
    println!("Apple heard: {said:?}");
    assert!(
        !said.iter().any(|l| l.contains("conflicted copy")),
        "a save during an upload made a conflicted copy: {said:?}"
    );
    let here = dir(&place);
    assert_eq!(here, ["note.md"], "only the note, no copies beside it");
    let at_apple = apple
        .0
        .lock()
        .unwrap()
        .get(ROOT)
        .cloned()
        .unwrap_or_default();
    assert_eq!(at_apple.len(), 1, "one file at Apple: {at_apple:?}");
    assert_eq!(at_apple[0].size, Some(6), "Apple has the latest save");

    drop(folder);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// A file landing as a slow download writes it: a piece every two seconds for 80 seconds, then
/// moved into place, as the app's photo cache downloads a long video on a slow line.
struct Slow {
    part: PathBuf,
    kept: PathBuf,
    total: u64,
    have: std::sync::atomic::AtomicU64,
    done: std::sync::atomic::AtomicBool,
    more: tokio::sync::Notify,
}

impl Landing for Slow {
    fn total(&self) -> u64 {
        self.total
    }
    fn wait_for(
        &self,
        upto: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<u64>> + Send + '_>> {
        use std::sync::atomic::Ordering;
        Box::pin(async move {
            loop {
                let wake = self.more.notified();
                let have = self.have.load(Ordering::SeqCst);
                if self.done.load(Ordering::SeqCst) || (upto < self.total && have >= upto) {
                    return Some(have);
                }
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), wake).await;
            }
        })
    }
    fn path(&self) -> PathBuf {
        if self.done.load(std::sync::atomic::Ordering::SeqCst) {
            self.kept.clone()
        } else {
            self.part.clone()
        }
    }
}

/// Apple for a photo kept by the app, its download taking longer than a minute.
struct SlowPhotos(
    HashMap<String, Vec<DriveItem>>,
    Vec<u8>,
    PathBuf,
    std::sync::Mutex<Option<Arc<Slow>>>,
);

impl Remote for SlowPhotos {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let items = self.0.get(folder).cloned().unwrap_or_default();
        Box::pin(async move { Ok(items) })
    }
    fn read(&self, _: &DriveItem, _: u64, _: u64) -> Fut<Vec<u8>> {
        Box::pin(async { Err("read through the photo cache".to_string()) })
    }
    fn arriving(&self, _: &DriveItem) -> Option<Fut<Arriving>> {
        let mut started = self.3.lock().unwrap();
        let slow = started
            .get_or_insert_with(|| {
                let slow = Arc::new(Slow {
                    part: self.2.join("film.part"),
                    kept: self.2.join("film.mov"),
                    total: self.1.len() as u64,
                    have: 0.into(),
                    done: false.into(),
                    more: tokio::sync::Notify::new(),
                });
                std::fs::write(&slow.part, b"").unwrap();
                let (s, bytes) = (Arc::clone(&slow), self.1.clone());
                tokio::spawn(async move {
                    use std::io::Write;
                    use std::sync::atomic::Ordering;
                    let mut out = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&s.part)
                        .unwrap();
                    // Pieces that are not whole 4 KB blocks, as a network delivers them.
                    for piece in bytes.chunks(bytes.len().div_ceil(40)) {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        out.write_all(piece).unwrap();
                        out.flush().unwrap();
                        s.have.fetch_add(piece.len() as u64, Ordering::SeqCst);
                        s.more.notify_waiters();
                    }
                    drop(out);
                    std::fs::rename(&s.part, &s.kept).unwrap();
                    s.done.store(true, Ordering::SeqCst);
                    s.more.notify_waiters();
                });
                slow
            })
            .clone();
        Some(Box::pin(async move { Ok(Arriving::Landing(slow)) }))
    }
}

/// Windows fails a file request it hears nothing about for 60 seconds, and only a transfer
/// counts. A video fetched whole into the photo cache before its first byte was sent took
/// longer than that on a slow line and never opened: its bytes now go to Windows as they land.
#[test]
fn a_photo_downloading_for_over_a_minute_still_opens() {
    let bytes: Vec<u8> = (0..300_000u64).map(byte).collect();
    let tree: HashMap<String, Vec<DriveItem>> = [(
        ROOT.to_string(),
        vec![file("slow", "IMG_0001", "MOV", bytes.len() as i64)],
    )]
    .into();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep slow {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-slow-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("slow{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep slow test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let up = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(SlowPhotos(
                tree.clone(),
                bytes.clone(),
                out.clone(),
                std::sync::Mutex::new(None),
            )),
            Arc::new(Library(Arc::new(std::sync::Mutex::new(tree)))),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("the cloud folder registers and connects");

    let copy = out.join("copied.mov");
    let started = std::time::Instant::now();
    let ok = Command::new("cmd")
        .args(["/c", "copy", "/b", "/y"])
        .arg(place.join("IMG_0001.MOV"))
        .arg(&copy)
        .status()
        .unwrap()
        .success();
    let took = started.elapsed();
    println!("opened after {took:?}");
    assert!(ok, "the video never opened: Windows gave up waiting on it");
    assert!(
        took.as_secs() >= 70,
        "its download really took over a minute"
    );
    assert_eq!(std::fs::read(&copy).unwrap(), bytes, "and arrived whole");

    drop(up);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// Apple whose first listing of a folder is held until the test lets it go: the moment a folder
/// is being listed, made to last.
struct Held {
    tree: HashMap<String, Vec<DriveItem>>,
    /// Set once the first listing has been asked for.
    asked: Arc<std::sync::atomic::AtomicBool>,
    /// Lets the first listing go.
    go: Arc<tokio::sync::Notify>,
    /// Set once the first listing has been answered.
    answered: Arc<std::sync::atomic::AtomicBool>,
}

impl Remote for Held {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        use std::sync::atomic::Ordering;
        let items = self.tree.get(folder).cloned().unwrap_or_default();
        let first = !self.asked.swap(true, Ordering::SeqCst);
        let (go, answered) = (Arc::clone(&self.go), Arc::clone(&self.answered));
        Box::pin(async move {
            if first {
                go.notified().await;
                answered.store(true, Ordering::SeqCst);
            }
            Ok(items)
        })
    }
    fn read(&self, _: &DriveItem, _: u64, _: u64) -> Fut<Vec<u8>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// A folder being listed when Skykeep stops (quit, or restarted for an update) opens whole the
/// next time. Windows takes such a listing back, and it was marked whole all the same with what
/// had arrived: the folder was never asked for again and showed some of its files for good.
#[test]
fn a_listing_taken_back_part_way_is_listed_again() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut sub = DriveItem {
        drivewsid: "FOLDER::com.apple.CloudDocs::held".into(),
        kind: "FOLDER".into(),
        name: "Held".into(),
        extension: None,
        size: None,
        ..file("x", "x", "x", 0)
    };
    sub.parent_id = Some(ROOT.into());
    let root = vec![sub.clone()];
    let inner: Vec<DriveItem> = (0..600)
        .map(|n| {
            let mut f = file(&format!("held{n}"), &format!("held {n}"), "bin", 10);
            f.parent_id = Some(sub.drivewsid.clone());
            f
        })
        .collect();
    let tree: HashMap<String, Vec<DriveItem>> = [
        (ROOT.to_string(), root.clone()),
        (sub.drivewsid.clone(), inner),
    ]
    .into();
    let library = Arc::new(std::sync::Mutex::new(HashMap::from([(
        ROOT.to_string(),
        root,
    )])));
    let (asked, answered) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let go = Arc::new(tokio::sync::Notify::new());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep held {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-held-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("held{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep held test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let provider = || {
        Provider::new(
            Arc::new(Held {
                tree: tree.clone(),
                asked: Arc::clone(&asked),
                go: Arc::clone(&go),
                answered: Arc::clone(&answered),
            }),
            Arc::new(Library(Arc::clone(&library))),
            rt.handle().clone(),
            out.join("sending"),
        )
    };
    let first = cloud::start(&place, &shown, provider()).expect("the folder registers");
    assert!(dir(&place).contains(&"Held".to_string()));

    // Listed by another program, and Skykeep stopping while Apple's answer is on its way.
    let mut lister = Command::new("cmd")
        .args(["/c", "dir", "/b"])
        .arg(place.join("Held"))
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..300 {
        if asked.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        asked.load(Ordering::SeqCst),
        "the folder's listing was asked for"
    );
    drop(first);
    // Apple answers after all: what is sent for the listing now goes nowhere.
    go.notify_one();
    for _ in 0..300 {
        if answered.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = lister.wait();
    // The listing's own work after its answer: placeholders, and the folder marked or not.
    std::thread::sleep(std::time::Duration::from_secs(2));

    let second = cloud::start(&place, &shown, provider()).expect("the next run takes it up");
    let listed = dir(&place.join("Held"));
    assert_eq!(
        listed.len(),
        600,
        "the folder opens whole after the listing was taken back: {} of 600",
        listed.len()
    );
    drop(second);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// Apple on a slow line: every read takes as long as its bytes would at 20 KB a second.
struct SlowLine(HashMap<String, Vec<DriveItem>>);

impl Remote for SlowLine {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let items = self.0.get(folder).cloned().unwrap_or_default();
        Box::pin(async move { Ok(items) })
    }
    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        let size = u64::try_from(item.size.unwrap_or(0)).unwrap();
        let end = offset.saturating_add(len).min(size);
        Box::pin(async move {
            let ms = (end - offset) * 1000 / (20 * 1024);
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            Ok((offset..end).map(byte).collect())
        })
    }
}

/// A file in iCloud Drive opens on a slow line. Windows fails a request that receives nothing
/// for a minute, and the file was read from Apple in fixed 4 MB pieces: at 20 KB a second one
/// piece took longer than that, and the file never opened. Pieces now follow the line.
#[test]
fn a_file_opens_on_a_slow_line() {
    // One and a half megabytes: one piece of 77 seconds before, a few seconds each now.
    let size = 1536 * 1024u64;
    let tree: HashMap<String, Vec<DriveItem>> = [(
        ROOT.to_string(),
        vec![file("slowline", "slow line", "bin", size as i64)],
    )]
    .into();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep slow line {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-slowline-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("slowline{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep slow line test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let up = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(SlowLine(tree.clone())),
            Arc::new(Library(Arc::new(std::sync::Mutex::new(tree)))),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("the cloud folder registers and connects");

    let copy = out.join("copied.bin");
    let started = std::time::Instant::now();
    let ok = Command::new("cmd")
        .args(["/c", "copy", "/b", "/y"])
        .arg(place.join("slow line.bin"))
        .arg(&copy)
        .status()
        .unwrap()
        .success();
    println!("opened after {:?}", started.elapsed());
    assert!(
        ok,
        "the file never opened: Windows gave up waiting on a piece"
    );
    let got = std::fs::read(&copy).unwrap();
    assert_eq!(got.len() as u64, size);
    assert!(
        got.iter().enumerate().all(|(i, &b)| b == byte(i as u64)),
        "and every byte is the file's"
    );

    drop(up);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}

/// Room made on a low disk: a downloaded file is given back to iCloud and stays listed (and
/// opens again whole); one kept on the device by choice, and one open in a program, are left.
#[test]
fn room_is_made_from_files_not_kept_or_open() {
    let bytes: Vec<u8> = (0..300_000u64).map(byte).collect();
    let tree: HashMap<String, Vec<DriveItem>> = [(
        ROOT.to_string(),
        vec![
            file("roomfree", "free", "bin", bytes.len() as i64),
            file("roomkept", "kept", "bin", bytes.len() as i64),
            file("roomopen", "open", "bin", bytes.len() as i64),
        ],
    )]
    .into();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let base = PathBuf::from(std::env::var("USERPROFILE").unwrap());
    let place = base.join(format!("Skykeep room {}", std::process::id()));
    let out = std::env::temp_dir().join(format!("skykeep-room-out-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let id = cloud::sync_root_id(&format!("room{}", std::process::id())).unwrap();
    let shown = Shown {
        id: &id,
        name: "Skykeep room test",
        icon: "%SystemRoot%\\system32\\imageres.dll,-1043",
    };
    let up = cloud::start(
        &place,
        &shown,
        Provider::new(
            Arc::new(Apple(
                tree.clone(),
                Arc::new(std::sync::Mutex::new(Vec::new())),
            )),
            Arc::new(Library(Arc::new(std::sync::Mutex::new(tree)))),
            rt.handle().clone(),
            out.join("sending"),
        ),
    )
    .expect("the cloud folder registers and connects");

    use std::os::windows::fs::MetadataExt;
    let here = |p: &Path| std::fs::metadata(p).unwrap().file_attributes() & 0x0040_0000 == 0;
    let (free, kept, open) = (
        place.join("free.bin"),
        place.join("kept.bin"),
        place.join("open.bin"),
    );
    // Listed and read by another program, as a person's would be: Windows asks nothing of a
    // folder's own provider for its own reads (this test is the provider).
    let names = dir(&place);
    let read_out = |f: &Path, to: &str| -> Vec<u8> {
        let copy = out.join(to);
        assert!(Command::new("cmd")
            .args(["/c", "copy", "/b", "/y"])
            .arg(f)
            .arg(&copy)
            .status()
            .unwrap()
            .success());
        std::fs::read(&copy).unwrap()
    };
    for (f, to) in [(&free, "a"), (&kept, "b"), (&open, "c")] {
        assert!(
            names.iter().any(|n| f.ends_with(n)),
            "{} listed: {names:?}",
            f.display()
        );
        assert_eq!(read_out(f, to), bytes, "{} downloads whole", f.display());
        assert!(here(f));
    }
    // Kept on this device by choice ("Always keep on this device").
    assert!(Command::new("cmd")
        .args(["/c", "attrib", "+P"])
        .arg(&kept)
        .status()
        .unwrap()
        .success());
    // Wait for every file to be marked in sync again after the reads.
    std::thread::sleep(std::time::Duration::from_secs(2));
    let held = std::fs::File::open(&open).unwrap();

    let given = quince_vfs::room::make_room_sparing(
        std::slice::from_ref(&place),
        0,
        u64::MAX,
        std::time::Duration::ZERO,
    );
    drop(held);
    assert!(!here(&free), "the downloaded file was given back");
    assert!(here(&kept), "the file kept on this device stayed");
    assert!(here(&open), "the file open in a program stayed");
    assert_eq!(given, bytes.len() as u64);
    assert!(
        dir(&place).contains(&"free.bin".to_string()),
        "still listed"
    );
    assert_eq!(read_out(&free, "again"), bytes, "and opens again whole");

    drop(up);
    cloud::unregister(&place, &id);
    let _ = std::fs::remove_dir_all(&place);
    let _ = std::fs::remove_dir_all(&out);
}
