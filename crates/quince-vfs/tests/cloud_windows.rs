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
    fn file(&self, item: &DriveItem) -> Option<Fut<PathBuf>> {
        self.0
            .lock()
            .unwrap()
            .push(format!("fetch {}", item.display_name()));
        let path = self.1.clone();
        Some(Box::pin(async move { Ok(path) }))
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
