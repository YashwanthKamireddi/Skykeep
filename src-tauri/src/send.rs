//! Send to iPhone: files chosen in File Explorer (Send to → iPhone) or a Linux file manager
//! (Open With → Send to iPhone) delivered through iCloud itself. A photo or video goes into
//! Photos, where the iPhone shows it in Recents; anything else into iCloud Drive, in a folder
//! named for this computer, so it is in the Files app.
//!
//! No app on the iPhone, no pairing, no shared Wi-Fi: iOS takes it in the background, locked or
//! in a pocket, the way it takes everything from iCloud. A local transfer (LocalSend's) stops
//! the moment the phone locks; this one doesn't.
//!
//! Each file goes up through the uploads the apps themselves use, so it is proven intact and
//! retried as theirs are. What could not go is said by name, with why, never dropped quietly.

use crate::photos::add_from_folder;
use quince_core::drive::DriveItem;
use quince_vfs::{Remote, ROOT};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

/// The launch that asks for it: `skykeep --send-to-iphone <files…>`.
pub const ASK: &str = "--send-to-iphone";

/// The same, with the paths in a file (one per line), deleted once read: what File Explorer's
/// command hands over when a selection is too long for one command line (Windows takes about
/// 32,000 characters; a thousand photos is more).
pub const ASK_LIST: &str = "--send-to-iphone-list";

/// The files a launch asks to send, when it asks.
pub fn asked(args: &[String], cwd: &Path) -> Option<Vec<PathBuf>> {
    if let Some(at) = args.iter().position(|a| a == ASK_LIST) {
        let list = PathBuf::from(args.get(at + 1)?);
        let text = std::fs::read_to_string(&list).unwrap_or_default();
        let _ = std::fs::remove_file(&list);
        return Some(
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(PathBuf::from)
                .collect(),
        );
    }
    args.iter()
        .any(|a| a == ASK)
        .then(|| crate::apps::paths_asked(args, cwd))
}

/// Where a file goes: Photos takes photos and videos it accepts; everything else, a video too
/// large for Photos included, goes to iCloud Drive, so nothing sent is ever refused for its type.
#[derive(Debug, PartialEq, Eq)]
enum To {
    Photos,
    Files,
}

fn destination(path: &Path) -> To {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let size = std::fs::metadata(path).map_or(0, |m| m.len());
    if path.is_file() && quince_icloud::photos_upload::refusal(&name, size).is_none() {
        To::Photos
    } else {
        To::Files
    }
}

/// Send `paths` to the iPhone, saying when it starts and how it ended.
pub fn send(app: &AppHandle, paths: Vec<PathBuf>) {
    let paths: Vec<PathBuf> = paths.into_iter().filter(|p| p.exists()).collect();
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        crate::alerts::tell(
            &app,
            "Sending to your iPhone",
            &match paths.len() {
                1 => name_of(&paths[0]),
                n => format!("{n} items"),
            },
        );
        // A launch that only sends (nothing else open) takes up the saved session itself.
        let signed_in = matches!(
            crate::auth_resume(app.clone(), tauri::Manager::state(&app)).await,
            Ok("library")
        );
        if !signed_in {
            crate::alerts::tell(
                &app,
                "Sign in to send to your iPhone",
                "Open Skykeep and sign in with your Apple Account, then send it again.",
            );
            crate::show_window(&app);
            return;
        }
        let ended = deliver(&app, &paths).await;
        let (title, body) = ended.words();
        crate::alerts::tell(&app, &title, &body);
    });
}

/// How a sending ended.
#[derive(Default)]
struct Ended {
    photos: usize,
    /// Photos already in the library: on the iPhone already, nothing to send.
    already: usize,
    files: usize,
    failed: Vec<(String, String)>,
}

/// Failures named one by one in the notice; the rest counted.
const NAMED: usize = 3;

impl Ended {
    fn words(&self) -> (String, String) {
        let mut where_ = match (self.photos, self.files) {
            (0, 0) => String::new(),
            (p, 0) => format!("In Photos on your iPhone ({p})."),
            (0, f) => format!("In the Files app, iCloud Drive, {} ({f}).", folder_name()),
            (p, f) => format!(
                "{p} in Photos, {f} in the Files app (iCloud Drive, {}).",
                folder_name()
            ),
        };
        if self.already > 0 {
            let already = match self.already {
                1 => "1 photo was in Photos already.".to_string(),
                n => format!("{n} photos were in Photos already."),
            };
            where_ = if where_.is_empty() {
                already
            } else {
                format!("{where_} {already}")
            };
        }
        let sent = self.photos + self.files + self.already;
        match self.failed.as_slice() {
            [] => ("Sent to your iPhone".into(), where_),
            [(name, why)] if sent == 0 => (format!("{name} wasn't sent"), why.clone()),
            failed => {
                let mut lines: Vec<String> = failed
                    .iter()
                    .take(NAMED)
                    .map(|(n, why)| format!("{n}: {why}"))
                    .collect();
                if failed.len() > NAMED {
                    lines.push(format!("and {} more", failed.len() - NAMED));
                }
                (
                    format!("{sent} sent, {} not", failed.len()),
                    lines.join("\n"),
                )
            }
        }
    }
}

/// One sending at a time: two started together would each make "From <computer>" in iCloud
/// Drive, and leave two. The second waits for the first.
static SENDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn deliver(app: &AppHandle, paths: &[PathBuf]) -> Ended {
    let _one = SENDING.lock().await;
    let mut ended = Ended::default();
    let mut folder: Option<DriveItem> = None;
    let remote = crate::mount::drive_remote(app);
    for path in paths {
        let name = name_of(path);
        let sent = match destination(path) {
            To::Photos => add_from_folder(app, path, None).await.map(|_| To::Photos),
            To::Files => {
                if folder.is_none() {
                    folder = match ours(remote.as_ref()).await {
                        Ok(f) => Some(f),
                        Err(e) => {
                            ended.failed.push((name, e));
                            continue;
                        }
                    };
                }
                let into = folder.as_ref().expect("made just above");
                put(remote.as_ref(), into, path, &mut ended).await;
                continue;
            }
        };
        match sent {
            Ok(_) => ended.photos += 1,
            Err(why) if why == quince_icloud::photos_upload::Refusal::Duplicate.words() => {
                ended.already += 1;
            }
            Err(why) => ended.failed.push((name, why)),
        }
    }
    ended
}

/// The folder in iCloud Drive that sent files land in: found by name, or made.
async fn ours(remote: &dyn Remote) -> Result<DriveItem, String> {
    let name = folder_name();
    let root = remote.list(ROOT).await?;
    if let Some(found) = root
        .iter()
        .find(|i| i.is_container() && i.display_name().eq_ignore_ascii_case(&name))
    {
        return Ok(found.clone());
    }
    let parent = DriveItem {
        drivewsid: ROOT.to_string(),
        parent_id: None,
        name: String::new(),
        extension: None,
        kind: "FOLDER".into(),
        size: None,
        child_count: None,
        created_at: None,
        modified_at: None,
        etag: None,
    };
    remote.mkdir(&parent, &name).await
}

/// A file, or a folder and everything in it, into `into`: each file counted as sent or failed
/// on its own, so one that can't go never stops the rest. Left out: a system's own files
/// (`Thumbs.db`, `.DS_Store`, …) and editors' scratch files, and links to folders, which can
/// lead back into themselves.
async fn put(remote: &dyn Remote, into: &DriveItem, path: &Path, ended: &mut Ended) {
    let name = name_of(path);
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        ended.failed.push((name, "It's no longer there.".into()));
        return;
    };
    if meta.file_type().is_symlink() && path.is_dir() {
        return;
    }
    if !path.is_dir() {
        match remote.upload(into, path).await {
            Ok(_) => ended.files += 1,
            Err(why) => ended.failed.push((name, why)),
        }
        return;
    }
    let made = match remote.mkdir(into, &name).await {
        Ok(made) => made,
        Err(why) => {
            ended.failed.push((name, why));
            return;
        }
    };
    let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| !not_sent(&name_of(p)))
        .collect();
    entries.sort();
    for entry in entries {
        Box::pin(put(remote, &made, &entry, ended)).await;
    }
}

/// A file no one means to send: a system's own, or an editor's scratch file.
fn not_sent(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "thumbs.db" | "desktop.ini" | ".ds_store" | ".localized" | "ehthumbs.db"
    ) || lower.starts_with("._")
        || lower.starts_with("~$")
        || quince_vfs::writes::stays_local(name)
}

/// The entry in File Explorer's Send to menu, as it shows there.
#[cfg(windows)]
const SEND_TO_ENTRY: &str = "iPhone (Skykeep).lnk";

/// File Explorer's Send to → iPhone (Skykeep): a shortcut in this person's Send To folder,
/// made each time Skykeep starts if it is missing or points at another copy, so a new install,
/// an update and a moved install all have it; the uninstallers take it away. Named for
/// Skykeep, so it never writes over a shortcut of anyone else's. Not inside an MSIX package,
/// whose writes there would be the package's alone, never seen by Explorer.
#[cfg(windows)]
pub fn offer_in_send_to() {
    if crate::package::packaged() {
        return;
    }
    std::thread::spawn(|| {
        if let Err(e) = make_send_to() {
            eprintln!("Send to iPhone in File Explorer: {e}");
        }
    });
}

#[cfg(windows)]
fn make_send_to() -> windows::core::Result<()> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::*;
    let exe = std::env::current_exe().map_err(|_| windows::core::Error::from_thread())?;
    // SAFETY: COM on this thread of our own; every object released before it is uninitialized.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let made = (|| {
            let folder = SHGetKnownFolderPath(&FOLDERID_SendTo, KNOWN_FOLDER_FLAG(0), None)?;
            let dir = PathBuf::from(folder.to_string().unwrap_or_default());
            CoTaskMemFree(Some(folder.0 as *const std::ffi::c_void));
            let at = HSTRING::from(dir.join(SEND_TO_ENTRY).as_os_str());
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let file: IPersistFile = link.cast()?;
            if file.Load(&at, STGM_READ).is_ok() {
                let mut target = [0u16; 1024];
                if link.GetPath(&mut target, std::ptr::null_mut(), 0).is_ok() {
                    let end = target.iter().position(|&u| u == 0).unwrap_or(target.len());
                    if Path::new(&String::from_utf16_lossy(&target[..end])) == exe {
                        return Ok(());
                    }
                }
            }
            let program = HSTRING::from(exe.as_os_str());
            link.SetPath(&program)?;
            link.SetArguments(&HSTRING::from(ASK))?;
            link.SetIconLocation(&program, 0)?;
            link.SetDescription(&HSTRING::from(
                "Send to your iPhone through iCloud, with Skykeep",
            ))?;
            file.Save(&at, true)
        })();
        CoUninitialize();
        made
    }
}

/// "From <this computer's name>", as the iPhone's Files app shows it.
fn folder_name() -> String {
    format!("From {}", computer_name())
}

fn computer_name() -> String {
    #[cfg(windows)]
    let name = std::env::var("COMPUTERNAME").ok();
    #[cfg(not(windows))]
    let name = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok());
    name.map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "this computer".into())
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_go_to_photos_and_everything_else_to_files() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("IMG_1.HEIC");
        std::fs::write(&photo, vec![1u8; 4096]).unwrap();
        let pdf = dir.path().join("Lease.pdf");
        std::fs::write(&pdf, vec![1u8; 4096]).unwrap();
        // Too small to be a photo Photos takes: Files, never refused.
        let tiny = dir.path().join("tiny.jpg");
        std::fs::write(&tiny, b"x").unwrap();
        assert_eq!(destination(&photo), To::Photos);
        assert_eq!(destination(&pdf), To::Files);
        assert_eq!(destination(&tiny), To::Files);
        assert_eq!(destination(dir.path()), To::Files, "a folder");
    }

    #[test]
    fn the_ending_says_where_each_went() {
        let mut e = Ended {
            photos: 2,
            files: 1,
            ..Ended::default()
        };
        assert_eq!(e.words().0, "Sent to your iPhone");
        assert!(e.words().1.starts_with("2 in Photos, 1 in the Files app"));
        e.failed
            .push(("a.zip".into(), "Your iCloud storage is full.".into()));
        assert_eq!(e.words().0, "3 sent, 1 not");
        // Many failures: three named, the rest counted. Photos already there are sent.
        e.failed = (0..10).map(|i| (format!("f{i}"), "why".into())).collect();
        e.already = 2;
        let (title, body) = e.words();
        assert_eq!(title, "5 sent, 10 not");
        assert_eq!(body.lines().count(), 4);
        assert!(body.ends_with("and 7 more"));
        let lone = Ended {
            failed: vec![("a.zip".into(), "Your iCloud storage is full.".into())],
            ..Ended::default()
        };
        assert_eq!(
            lone.words(),
            (
                "a.zip wasn't sent".into(),
                "Your iCloud storage is full.".into()
            )
        );
    }

    #[test]
    fn a_systems_own_files_are_never_sent() {
        for name in [
            "Thumbs.db",
            "desktop.ini",
            ".DS_Store",
            "._IMG_1.JPG",
            "~$Report.docx",
        ] {
            assert!(not_sent(name), "{name}");
        }
        for name in ["IMG_1.JPG", "Report.docx", ".bashrc"] {
            assert!(!not_sent(name), "{name}");
        }
    }

    #[test]
    fn a_long_selection_comes_as_a_list_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("send.txt");
        std::fs::write(&list, "/a/one.jpg\n/b/two words.pdf\n\n").unwrap();
        let args = vec![
            "skykeep".to_string(),
            ASK_LIST.to_string(),
            list.display().to_string(),
        ];
        assert_eq!(
            asked(&args, Path::new("/")),
            Some(vec![
                PathBuf::from("/a/one.jpg"),
                PathBuf::from("/b/two words.pdf")
            ])
        );
        assert!(!list.exists(), "deleted once read");
    }

    #[test]
    fn only_a_send_launch_asks() {
        let cwd = Path::new("/home/a");
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(asked(&args(&["skykeep", "a.pdf"]), cwd), None);
        assert_eq!(
            asked(&args(&["skykeep", ASK, "/tmp/a.pdf"]), cwd),
            Some(vec![PathBuf::from("/tmp/a.pdf")])
        );
    }
}
