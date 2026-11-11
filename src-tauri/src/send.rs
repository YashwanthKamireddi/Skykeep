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
        // How much it is, before anything goes: a folder can hold a project's thousands of files.
        let sizing = paths.clone();
        let Ok(survey) = tokio::task::spawn_blocking(move || survey(&sizing)).await else {
            return;
        };
        if let Some(free) = icloud_free(&app).await {
            if survey.bytes > free {
                crate::alerts::tell(
                    &app,
                    "Not enough room in iCloud",
                    &format!(
                        "This is {}; your iCloud has {} free. Free up space in iCloud, or send less.",
                        bytes_words(survey.bytes),
                        bytes_words(free)
                    ),
                );
                return;
            }
        }
        if survey.files >= BIG_FILES || survey.bytes >= BIG_BYTES {
            let question = format!(
                "Send {} to your iPhone? {} {}, {}.",
                match paths.as_slice() {
                    [one] => format!("\u{201c}{}\u{201d}", name_of(one)),
                    many => format!("{} items", many.len()),
                },
                survey.files.to_string_with_commas(),
                if survey.files == 1 { "file" } else { "files" },
                bytes_words(survey.bytes),
            );
            let asking = app.clone();
            let yes = tokio::task::spawn_blocking(move || {
                use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
                asking
                    .dialog()
                    .message(question)
                    .title("Send to iPhone")
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Send".into(),
                        "Cancel".into(),
                    ))
                    .blocking_show()
            })
            .await
            .unwrap_or(false);
            if !yes {
                return;
            }
        }
        let ended = deliver(&app, &paths, survey).await;
        let (title, body) = ended.words();
        crate::alerts::tell(&app, &title, &body);
    });
}

/// Past either, a sending is asked about first, with its size.
const BIG_FILES: u64 = 500;
const BIG_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Files sent at once to iCloud Drive: a folder of thousands goes several times faster than one
/// by one, without asking Apple for dozens of uploads at once.
const AT_ONCE: usize = 3;

/// What a sending holds: its files (a system's own and editors' scratch files left out, as
/// `put` leaves them) and their bytes. Walked holding only the folders still to read, so a tree
/// of a hundred thousand files costs the memory of a few hundred names.
#[derive(Clone, Copy, Default)]
struct Survey {
    files: u64,
    bytes: u64,
}

fn survey(paths: &[PathBuf]) -> Survey {
    let mut s = Survey::default();
    let mut dirs = Vec::new();
    for path in paths {
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => dirs.push(path.clone()),
            Ok(m) if m.is_file() => {
                s.files += 1;
                s.bytes += m.len();
            }
            _ => {}
        }
    }
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            if not_sent(&entry.file_name().to_string_lossy()) {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(entry.path()),
                Ok(t) if t.is_file() => {
                    s.files += 1;
                    s.bytes += entry.metadata().map_or(0, |m| m.len());
                }
                _ => {}
            }
        }
    }
    s
}

/// iCloud's free space, when Apple says.
async fn icloud_free(app: &AppHandle) -> Option<u64> {
    let (http, setup, params) = {
        let auth = tauri::Manager::state::<crate::Auth>(app);
        let client = auth.client.lock().await;
        (
            client.http().clone(),
            client.setup_url().to_string(),
            client.service_params(),
        )
    };
    let s = quince_icloud::drive::storage(&http, &setup, params)
        .await
        .ok()?;
    (s.total_bytes > 0).then(|| s.total_bytes.saturating_sub(s.used_bytes))
}

fn bytes_words(bytes: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{} KB", (bytes / 1024).max(1))
    }
}

trait Commas {
    fn to_string_with_commas(&self) -> String;
}

impl Commas for u64 {
    fn to_string_with_commas(&self) -> String {
        let digits = self.to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        out
    }
}

/// How a sending ended.
#[derive(Default)]
struct Ended {
    photos: usize,
    /// Photos already in the library: on the iPhone already, nothing to send.
    already: usize,
    /// Photos still on their way when the sending stopped being followed.
    pending: usize,
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
        if self.pending > 0 {
            let on_way = format!(
                "{} still on {} way and arriving by {}.",
                match self.pending {
                    1 => "1 photo is".to_string(),
                    n => format!("{n} photos are"),
                },
                if self.pending == 1 { "its" } else { "their" },
                if self.pending == 1 {
                    "itself"
                } else {
                    "themselves"
                },
            );
            where_ = if where_.is_empty() {
                on_way
            } else {
                format!("{where_} {on_way}")
            };
        }
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

async fn deliver(app: &AppHandle, paths: &[PathBuf], survey: Survey) -> Ended {
    let _one = SENDING.lock().await;
    let mut ended = Ended::default();
    let (photos, others): (Vec<PathBuf>, Vec<PathBuf>) = paths
        .iter()
        .cloned()
        .partition(|p| destination(p) == To::Photos);
    // Every photo queued at once, as one batch: the Photos queue sends them back to back while
    // Apple makes each, shows the batch in its panel, and carries on after a quit.
    let batch = if photos.is_empty() {
        None
    } else {
        let data = tauri::Manager::state::<crate::Data>(app);
        let batch = uuid::Uuid::new_v4().to_string();
        match crate::photos::queue_files(app, &data, batch, photos.clone(), None).await {
            Ok(batch) => Some(batch),
            Err(why) => {
                for p in &photos {
                    ended.failed.push((name_of(p), why.to_string()));
                }
                None
            }
        }
    };
    if !others.is_empty() {
        let remote = crate::mount::drive_remote(app);
        match ours(remote.as_ref()).await {
            Ok(folder) => {
                for path in &others {
                    put(app, remote.as_ref(), &folder, path, &mut ended, survey).await;
                }
            }
            Err(why) => {
                for p in &others {
                    ended.failed.push((name_of(p), why.clone()));
                }
            }
        }
    }
    if let Some(batch) = batch {
        follow(app, &batch, &mut ended).await;
    }
    ended
}

/// The photo batch followed to its end, by its own uploads' states: each made in Photos, already
/// there, or failed with why. Never waited on for ever: twenty minutes without any change and
/// the rest are said to be on their way, as they are (the queue goes on by itself).
async fn follow(app: &AppHandle, batch: &str, ended: &mut Ended) {
    const QUIET: std::time::Duration = std::time::Duration::from_secs(20 * 60);
    let duplicate = quince_icloud::photos_upload::Refusal::Duplicate.words();
    let mut seen = String::new();
    let mut changed = std::time::Instant::now();
    let mut told = 0u64;
    loop {
        let uploads = {
            let data = tauri::Manager::state::<crate::Data>(app);
            let library = data.0.lock().await;
            library.batch_uploads(batch).unwrap_or_default()
        };
        let open = uploads
            .iter()
            .filter(|u| matches!(u.state.as_str(), "waiting" | "sending" | "verifying"))
            .count();
        let now: String = uploads.iter().map(|u| u.state.as_str()).collect();
        if now != seen {
            seen = now;
            changed = std::time::Instant::now();
        }
        let total = uploads.len() as u64;
        let gone = total - open as u64;
        let quarter = gone * 4 / total.max(1);
        if total >= 50 && open > 0 && quarter > told && quarter < 4 {
            told = quarter;
            crate::alerts::tell(
                app,
                "Sending to your iPhone",
                &format!(
                    "{} of {} photos",
                    gone.to_string_with_commas(),
                    total.to_string_with_commas()
                ),
            );
        }
        if open == 0 || changed.elapsed() > QUIET {
            for u in uploads {
                match (u.state.as_str(), u.error.as_deref()) {
                    ("done", None) => ended.photos += 1,
                    ("done", Some(e)) if e == duplicate => ended.already += 1,
                    ("done", Some(_)) => ended.photos += 1,
                    ("waiting" | "sending" | "verifying", _) => ended.pending += 1,
                    (_, error) => ended.failed.push((
                        u.name.clone(),
                        error.unwrap_or("It was stopped.").to_string(),
                    )),
                }
            }
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
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
/// on its own, so one that can't go never stops the rest. The folders are made first (a file
/// needs its folder), then the files go [`AT_ONCE`] at a time; a sending of 50 files or more says
/// how far it is at each quarter. Left out: a system's own files (`Thumbs.db`, `.DS_Store`, …),
/// editors' scratch files, and links to folders, which can lead back into themselves.
async fn put(
    app: &AppHandle,
    remote: &dyn Remote,
    into: &DriveItem,
    path: &Path,
    ended: &mut Ended,
    survey: Survey,
) {
    use futures_util::StreamExt;
    let mut files: Vec<(DriveItem, PathBuf)> = Vec::new();
    let mut folders = vec![(into.clone(), path.to_path_buf())];
    while let Some((parent, path)) = folders.pop() {
        let name = name_of(&path);
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            ended.failed.push((name, "It's no longer there.".into()));
            continue;
        };
        if meta.is_file() {
            files.push((parent, path));
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        let made = match remote.mkdir(&parent, &name).await {
            Ok(made) => made,
            Err(why) => {
                ended.failed.push((name, why));
                continue;
            }
        };
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| !not_sent(&e.file_name().to_string_lossy()))
            .filter(|e| {
                e.file_type()
                    .is_ok_and(|t| !t.is_symlink() || e.path().is_file())
            })
            .map(|e| e.path())
            .collect();
        entries.sort();
        for entry in entries.into_iter().rev() {
            folders.push((made.clone(), entry));
        }
    }
    let total = survey.files.max(files.len() as u64);
    let mut told = 0u64;
    let mut done = futures_util::stream::iter(files.into_iter().map(|(folder, path)| async move {
        let sent = remote.upload(&folder, &path).await.map(|_| ());
        (name_of(&path), sent)
    }))
    .buffer_unordered(AT_ONCE);
    while let Some((name, sent)) = done.next().await {
        match sent {
            Ok(()) => ended.files += 1,
            Err(why) => ended.failed.push((name, why)),
        }
        let gone = (ended.files + ended.failed.len()) as u64;
        let quarter = gone * 4 / total.max(1);
        if total >= 50 && quarter > told && quarter < 4 {
            told = quarter;
            crate::alerts::tell(
                app,
                "Sending to your iPhone",
                &format!(
                    "{} of {} files",
                    gone.to_string_with_commas(),
                    total.to_string_with_commas()
                ),
            );
        }
    }
}

/// A file no one means to send: a system's own, or an editor's scratch file.
pub(crate) fn not_sent(name: &str) -> bool {
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
    // Send to iPhone's own icon, installed beside the program (packaging/windows/bundle-windows.json,
    // msix/pack.ps1); the program's own where an older install has none.
    let icon = Some(exe.with_file_name("icons").join("send.ico"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| exe.clone());
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
                // Kept as it is only while it still opens this program with this icon: an
                // update that brings the icon rewrites it.
                let text = |buf: &[u16]| {
                    let end = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
                    PathBuf::from(String::from_utf16_lossy(&buf[..end]))
                };
                let (mut target, mut shown, mut index) = ([0u16; 1024], [0u16; 1024], 0i32);
                if link.GetPath(&mut target, std::ptr::null_mut(), 0).is_ok()
                    && link.GetIconLocation(&mut shown, &mut index).is_ok()
                    && text(&target) == exe
                    && text(&shown) == icon
                {
                    return Ok(());
                }
            }
            let program = HSTRING::from(exe.as_os_str());
            link.SetPath(&program)?;
            link.SetArguments(&HSTRING::from(ASK))?;
            link.SetIconLocation(&HSTRING::from(icon.as_os_str()), 0)?;
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
    fn a_sending_is_sized_as_it_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("Project");
        std::fs::create_dir_all(project.join("src/deep")).unwrap();
        std::fs::write(project.join("README.md"), vec![0u8; 100]).unwrap();
        std::fs::write(project.join("src/main.rs"), vec![0u8; 200]).unwrap();
        std::fs::write(project.join("src/deep/a.txt"), vec![0u8; 300]).unwrap();
        std::fs::write(project.join(".DS_Store"), vec![0u8; 999]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&project, project.join("src/back")).unwrap();
        let loose = dir.path().join("photo.jpg");
        std::fs::write(&loose, vec![0u8; 50]).unwrap();
        let s = survey(&[project, loose]);
        assert_eq!(
            (s.files, s.bytes),
            (4, 650),
            "system files and a link back left out"
        );
    }

    #[test]
    fn sizes_and_counts_read_as_people_read_them() {
        assert_eq!(
            bytes_words(3 * 1024 * 1024 * 1024 + 300 * 1024 * 1024),
            "3.3 GB"
        );
        assert_eq!(bytes_words(512 * 1024 * 1024), "512 MB");
        assert_eq!(bytes_words(10), "1 KB");
        assert_eq!(12_340u64.to_string_with_commas(), "12,340");
        assert_eq!(999u64.to_string_with_commas(), "999");
        assert_eq!(1_234_567u64.to_string_with_commas(), "1,234,567");
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
