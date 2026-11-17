//! Skykeep's own updates, from the project's GitHub Releases.
//!
//! Only a copy installed from the AppImage, the Windows installer (.exe) or the .msi updates
//! itself. A .deb, an .rpm, the AUR, a distribution's own package, Scoop or the Flatpak is
//! updated by its package manager and is never offered an update here ([`install_kind`]); nor
//! is the Microsoft Store's MSIX package, which the Store updates (package.rs).
//!
//! Shortly after launch, and about once a day after that, the latest release's `latest.json`
//! is read through a client that can reach GitHub's release servers and nothing else
//! (`quince_auth::releases`). A newer version is offered in one window, never in several. It
//! is downloaded only when the person asks, verified against Skykeep's public key by the
//! updater plugin (a minisign signature with the version signed into it, so an older release
//! cannot be passed off as a newer one), and installed only once nothing is waiting to reach
//! iCloud and nobody is typing. Then Skykeep restarts into it.
//!
//! A check made on its own that fails (offline, GitHub busy) is never said. One asked for in
//! Settings is, plainly. A download that fails its signature is never installed, and says so.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Error, Update, UpdaterExt};

/// Told to every window when anything here changes; each asks for its own [`Status`] again.
pub const CHANGED: &str = "quince://update";

/// The first check, after launch: late enough not to compete with the library opening.
const FIRST_CHECK: Duration = Duration::from_secs(45);
/// Then about once a day, counted on the wall clock so a night asleep counts.
const EVERY_SECS: i64 = 24 * 60 * 60;
/// After a check that failed by itself: again in a few hours, never sooner.
const AFTER_FAILURE_SECS: i64 = 3 * 60 * 60;
/// How often the schedule looks at the clock.
const TICK: Duration = Duration::from_secs(10 * 60);
/// A window's "someone is typing" lasts this long unless it is said again.
const TYPING_LASTS: Duration = Duration::from_secs(15);
/// After a download, a moment for every window to say whether it is being typed in.
const SETTLE: Duration = Duration::from_millis(1500);
/// How often a waiting restart looks again.
const WAIT_STEP: Duration = Duration::from_secs(2);

/// How this copy of Skykeep was installed, which decides who updates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Install {
    /// The AppImage, the Windows installer or the .msi: updated here.
    Here,
    /// A .deb, an .rpm, the AUR, a distribution's package or Scoop: its package manager
    /// updates it.
    PackageManager,
    /// Flatpak updates it.
    Flatpak,
    /// An MSIX package, from the Microsoft Store: the Store updates it.
    Store,
    /// A development build, run from the source.
    Development,
    /// Anything else, such as a program copied by hand: nothing here can replace it safely.
    Manual,
}

/// Who updates this copy. `bundle` is what Tauri's bundler wrote into the program when it
/// packaged it (none for a `cargo build`, which is how the AUR and the Flatpak build it);
/// `appimage` whether it runs as an AppImage now, which is the file the update replaces;
/// `msix` whether it is an MSIX package's — whatever bundle its program was taken from, a
/// package's files are Windows' to replace, never this program's; `scoop` whether Scoop
/// unpacked it (packaging/scoop): its installer, run here, would put a second copy beside
/// Scoop's.
pub fn install_kind(
    os: &str,
    bundle: Option<BundleType>,
    appimage: bool,
    flatpak: bool,
    msix: bool,
    scoop: bool,
    debug: bool,
) -> Install {
    if debug {
        return Install::Development;
    }
    if msix {
        return Install::Store;
    }
    if flatpak {
        return Install::Flatpak;
    }
    if os == "windows" && scoop {
        return Install::PackageManager;
    }
    match (os, bundle) {
        ("linux", Some(BundleType::AppImage)) if appimage => Install::Here,
        // Packaged as an AppImage but not run as one (extracted by hand): no file to replace.
        ("linux", Some(BundleType::AppImage)) => Install::Manual,
        ("linux", _) => Install::PackageManager,
        ("windows", Some(BundleType::Nsis | BundleType::Msi)) => Install::Here,
        _ => Install::Manual,
    }
}

/// This copy's [`Install`].
pub fn this_install(app: &AppHandle) -> Install {
    #[cfg(target_os = "linux")]
    let appimage = app.env().appimage.is_some();
    #[cfg(not(target_os = "linux"))]
    let appimage = {
        let _ = app;
        false
    };
    let flatpak = std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists();
    // Scoop records the install in the folder it unpacks an app to: scoop-install.json, or
    // install.json beside manifest.json in older Scoop.
    let scoop = cfg!(windows)
        && std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .is_some_and(|dir| {
                dir.join("scoop-install.json").is_file()
                    || (dir.join("install.json").is_file() && dir.join("manifest.json").is_file())
            });
    install_kind(
        std::env::consts::OS,
        tauri::utils::platform::bundle_type(),
        appimage,
        flatpak,
        crate::package::packaged(),
        scoop,
        cfg!(debug_assertions),
    )
}

/// Whether `offered` is an update to `current`: newer, and a full release. Never a downgrade,
/// never the same version again, never a prerelease (there is no prerelease channel).
pub fn is_update(current: &semver::Version, offered: &semver::Version) -> bool {
    offered.pre.is_empty() && offered > current
}

/// What a downloaded update is waiting for before Skykeep restarts to install it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Hold {
    /// Changes, photos or Drive files that have not reached iCloud yet.
    Changes,
    /// Someone is typing in a window.
    Typing,
}

/// Whether Skykeep may restart now, and if not, why. Never while anything made here is still on
/// its way to iCloud — the outbox's changes, photos being uploaded, files written in the Drive
/// folder — and never while someone is typing.
pub fn hold(changes: usize, uploads: usize, files: usize, typing: bool) -> Option<Hold> {
    if changes > 0 || uploads > 0 || files > 0 {
        Some(Hold::Changes)
    } else if typing {
        Some(Hold::Typing)
    } else {
        None
    }
}

/// Which window shows the update notice: the one already showing it while it is on screen, or
/// else the window asking, if it is on screen. Never two at once.
pub fn claim(
    holder: Option<&str>,
    holder_shown: bool,
    asker: &str,
    asker_shown: bool,
) -> Option<String> {
    match holder {
        Some(h) if h == asker || holder_shown => Some(h.to_string()),
        _ if asker_shown => Some(asker.to_string()),
        _ => None,
    }
}

/// Where things went wrong, for saying what it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Check,
    Download,
    Install,
}

/// Whether an error is the signature refusing the download.
pub fn is_signature(e: &Error) -> bool {
    matches!(
        e,
        Error::Minisign(_)
            | Error::Base64(_)
            | Error::SignatureUtf8(_)
            | Error::SignedVersionMismatch { .. }
            | Error::MissingSignedVersion
    )
}

/// What went wrong, said plainly: never the library's own words, which name nothing the
/// person can act on.
pub fn problem(stage: Stage, e: &Error) -> String {
    if is_signature(e) {
        return "The download didn't match Skykeep's signature, so it wasn't installed. \
                Your current version is unchanged."
            .into();
    }
    match (stage, e) {
        (Stage::Check, Error::Reqwest(_)) => {
            "Couldn't reach GitHub to check for updates. Check your connection and try again."
                .into()
        }
        (Stage::Check, Error::ReleaseNotFound | Error::Network(_)) => {
            "GitHub didn't answer just now. Try again later.".into()
        }
        (Stage::Check, _) => "Couldn't read the update information. Try again later.".into(),
        (Stage::Download, _) => {
            "The download didn't finish. Nothing was changed, so you can try again.".into()
        }
        (Stage::Install, Error::PreviousAppNotRestored(_)) => {
            "The update couldn't be installed. Download Skykeep again from its releases page."
                .into()
        }
        (Stage::Install, _) => {
            "The update couldn't be installed. Your current version is unchanged.".into()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Checking,
    /// Checked: no newer version.
    Current,
    /// A newer version, not downloaded.
    Available,
    Downloading,
    /// Downloaded and verified, kept for when the person says.
    Ready,
    /// Downloaded and verified, waiting for a moment it can restart.
    Waiting(Hold),
    Installing,
    /// `notice`: said in the update notice (it came of Update and Restart), not only Settings.
    Failed {
        notice: bool,
    },
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::Checking => "checking",
            Phase::Current => "current",
            Phase::Available => "available",
            Phase::Downloading => "downloading",
            Phase::Ready => "ready",
            Phase::Waiting(_) => "waiting",
            Phase::Installing => "installing",
            Phase::Failed { .. } => "failed",
        }
    }

    /// Whether Update and Restart is under way.
    fn applying(self) -> bool {
        matches!(
            self,
            Phase::Downloading | Phase::Waiting(_) | Phase::Installing
        )
    }
}

/// The updater's state, shared by every window.
pub struct Updates(Mutex<Inner>);

struct Inner {
    install: Install,
    auto: bool,
    phase: Phase,
    update: Option<Update>,
    /// The verified download on disk, kept across a restart.
    staged: Option<PathBuf>,
    received: u64,
    total: Option<u64>,
    problem: Option<String>,
    checked_at: Option<i64>,
    /// The version Later was chosen for: not offered again until the next check finds it.
    put_off: Option<String>,
    /// A version whose download failed its signature: never offered again in this run.
    refused: Option<String>,
    /// The window showing the notice.
    holder: Option<String>,
    /// Each window being typed in, and when it last said so.
    typing: HashMap<String, Instant>,
    /// Which Update and Restart is current; Later moves it on, ending a wait.
    attempt: u64,
}

impl Updates {
    pub fn new(install: Install, prefs: &Prefs) -> Self {
        Self(Mutex::new(Inner {
            install,
            auto: prefs.auto,
            phase: Phase::Idle,
            update: None,
            staged: None,
            received: 0,
            total: None,
            problem: None,
            checked_at: prefs.checked_at,
            put_off: None,
            refused: None,
            holder: None,
            typing: HashMap::new(),
            attempt: 0,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// What a window shows of the updater: in Settings, and (when `prompt`) as its notice.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    install: Install,
    auto: bool,
    current: String,
    phase: &'static str,
    version: Option<String>,
    notes: Option<String>,
    received: u64,
    total: Option<u64>,
    waiting_for: Option<Hold>,
    problem: Option<String>,
    checked_at: Option<i64>,
    /// This window is the one that shows the notice.
    prompt: bool,
}

/// What is kept between runs, in `updates.json`.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prefs {
    #[serde(default = "on")]
    pub auto: bool,
    #[serde(default)]
    pub checked_at: Option<i64>,
}

fn on() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            auto: true,
            checked_at: None,
        }
    }
}

fn prefs_path(app: &AppHandle) -> PathBuf {
    crate::state_dir(app).join("updates.json")
}

pub fn load_prefs(app: &AppHandle) -> Prefs {
    std::fs::read(prefs_path(app))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_prefs(app: &AppHandle, prefs: &Prefs) {
    let path = prefs_path(app);
    let tmp = path.with_extension("json.tmp");
    let Ok(text) = serde_json::to_vec_pretty(prefs) else {
        return;
    };
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn remember(app: &AppHandle, i: &Inner) {
    save_prefs(
        app,
        &Prefs {
            auto: i.auto,
            checked_at: i.checked_at,
        },
    );
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn changed(app: &AppHandle) {
    app.emit(CHANGED, ()).ok();
}

/// The updater for this copy: the plugin's, with its endpoint and public key from
/// tauri.conf.json, its client locked to GitHub's release servers, and Skykeep's own version
/// policy. On Windows the installer takes over as Skykeep exits; the library is let go of
/// first, as Quit lets go of it.
fn updater(app: &AppHandle) -> tauri_plugin_updater::Result<tauri_plugin_updater::Updater> {
    let leaving = app.clone();
    app.updater_builder()
        .timeout(Duration::from_secs(30))
        .version_comparator(|current, release| is_update(&current, &release.version))
        .configure_client(|client| {
            quince_auth::releases::releases_only(
                client
                    .connect_timeout(Duration::from_secs(20))
                    .read_timeout(Duration::from_secs(60)),
            )
        })
        .on_before_exit(move || {
            crate::wind_down(&leaving);
            leaving.cleanup_before_exit();
        })
        .build()
}

/// Where a download is kept until it is installed: the cache folder, so clearing it costs a
/// download, never data.
fn staging_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("update"))
}

/// Keep a verified download, with the release it came from, for after a restart.
fn stage(app: &AppHandle, update: &Update, bytes: &[u8]) -> Option<PathBuf> {
    let dir = staging_dir(app)?;
    std::fs::create_dir_all(&dir).ok()?;
    let package = dir.join("package");
    let part = dir.join("package.part");
    std::fs::write(&part, bytes).ok()?;
    std::fs::rename(&part, &package).ok()?;
    let release = dir.join("release.json");
    let tmp = dir.join("release.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&update.raw_json).ok()?).ok()?;
    std::fs::rename(&tmp, &release).ok()?;
    Some(package)
}

fn unstage(app: &AppHandle) {
    if let Some(dir) = staging_dir(app) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// A download kept by an earlier run: taken up again if it is still an update and still
/// passes its signature, with the version signed into it (the plugin insists on that for a
/// release read back from disk). Anything else is thrown away — including the download of the
/// version now running, once the update is done.
fn restore(app: &AppHandle) {
    let Some(dir) = staging_dir(app) else {
        return;
    };
    let release = dir.join("release.json");
    let package = dir.join("package");
    if !release.exists() {
        unstage(app);
        return;
    }
    let restored = (|| {
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&release).ok()?).ok()?;
        let update = updater(app).ok()?.restore_update(raw).ok()??;
        let bytes = std::fs::read(&package).ok()?;
        update.verify(&bytes).ok()?;
        Some(update)
    })();
    match restored {
        Some(update) => {
            let updates = app.state::<Updates>();
            let mut i = updates.lock();
            i.update = Some(update);
            i.staged = Some(package);
            // Not offered yet where a check will run (#82): a download kept from days ago was
            // offered at launch, before the first check found a newer release, and installed
            // instead of it (0.1.25 to 0.1.26 while 0.1.27 was out). The check decides: a newer
            // release replaces it, the same one is offered as ready, and with no answer from
            // GitHub (or checks turned off) it is offered as it is.
            if !i.auto {
                i.phase = Phase::Ready;
            }
            drop(i);
            changed(app);
        }
        None => unstage(app),
    }
}

/// Look for a newer version. `asked`: from Settings, so a failure is said; otherwise a failure
/// is kept quiet. Answers whether the check reached GitHub.
async fn check(app: &AppHandle, asked: bool) -> bool {
    let updates = app.state::<Updates>();
    let before = {
        let mut i = updates.lock();
        if i.install != Install::Here || matches!(i.phase, Phase::Checking) || i.phase.applying() {
            return true;
        }
        let before = i.phase;
        i.phase = Phase::Checking;
        i.problem = None;
        before
    };
    changed(app);
    let found = match updater(app) {
        Ok(u) => u.check().await,
        Err(e) => Err(e),
    };
    let mut i = updates.lock();
    let reached = match found {
        Ok(Some(update)) => {
            i.checked_at = Some(unix_now());
            if i.refused.as_deref() == Some(update.version.as_str()) {
                // Its download failed its signature this run: not offered again.
                i.phase = if asked {
                    i.problem = Some(format!(
                        "Skykeep {} didn't match its signature, so it won't be installed.",
                        update.version
                    ));
                    Phase::Failed { notice: false }
                } else {
                    before
                };
            } else if i.staged.is_some()
                && i.update.as_ref().map(|u| &u.version) == Some(&update.version)
            {
                // Already downloaded.
                i.phase = Phase::Ready;
            } else {
                if i.staged.take().is_some() {
                    unstage(app);
                }
                // Found again by a check of its own: offered again, even after Later.
                if asked || i.put_off.as_deref() != Some(update.version.as_str()) {
                    i.put_off = None;
                }
                i.update = Some(update);
                i.phase = Phase::Available;
            }
            true
        }
        // No newer version, or none published for this kind of install yet.
        Ok(None) | Err(Error::TargetNotFound(_) | Error::TargetsNotFound(_)) => {
            i.checked_at = Some(unix_now());
            i.phase = if i.staged.is_some() {
                Phase::Ready
            } else {
                Phase::Current
            };
            true
        }
        Err(e) => {
            if asked {
                i.problem = Some(problem(Stage::Check, &e));
                i.phase = Phase::Failed { notice: false };
            } else if i.staged.is_some() {
                // GitHub didn't answer: the download kept from before is still an update.
                i.phase = Phase::Ready;
            } else {
                i.phase = before;
            }
            false
        }
    };
    remember(app, &i);
    drop(i);
    changed(app);
    reached
}

/// What is still on its way to iCloud: the outbox's changes, photos being uploaded, and files
/// written in the Drive folder.
async fn unsent(app: &AppHandle) -> (usize, usize, usize) {
    let (changes, uploads) = {
        let library = app.state::<crate::Data>();
        let library = library.0.lock().await;
        (
            library.outbox_count().unwrap_or(1),
            library.unfinished_uploads().map_or(1, |u| u.len()),
        )
    };
    let files = crate::mount::unsent_files(&crate::state_dir(app));
    (changes, uploads, files)
}

fn typing_now(i: &Inner) -> bool {
    i.typing.values().any(|at| at.elapsed() < TYPING_LASTS)
}

/// Update and Restart: download (unless already downloaded), wait for a moment nothing would be
/// lost, install, restart.
async fn apply(app: AppHandle) {
    let updates = app.state::<Updates>();
    let (attempt, update, staged) = {
        let mut i = updates.lock();
        if i.install != Install::Here || i.phase.applying() {
            return;
        }
        let Some(update) = i.update.clone() else {
            return;
        };
        i.attempt += 1;
        i.problem = None;
        i.put_off = None;
        (i.attempt, update, i.staged.clone())
    };

    // The package: kept from before, checked again, or downloaded now.
    let kept = match staged {
        Some(path) => {
            let check = update.clone();
            tauri::async_runtime::spawn_blocking(move || {
                std::fs::read(&path)
                    .ok()
                    .filter(|bytes| check.verify(bytes).is_ok())
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };
    let bytes = match kept {
        Some(bytes) => bytes,
        None => match download(&app, &update).await {
            Some(bytes) => bytes,
            None => return,
        },
    };

    // Waiting for a moment nothing would be lost: every window has a moment to say it is
    // being typed in, then it is looked at again every few seconds.
    tokio::time::sleep(SETTLE).await;
    loop {
        let (changes, uploads, files) = unsent(&app).await;
        let step = {
            let mut i = updates.lock();
            if i.attempt != attempt {
                // Later: the download is kept, for whenever the person says.
                return;
            }
            match hold(changes, uploads, files, typing_now(&i)) {
                None => {
                    i.phase = Phase::Installing;
                    None
                }
                Some(h) => {
                    let fresh = i.phase != Phase::Waiting(h);
                    i.phase = Phase::Waiting(h);
                    Some(fresh)
                }
            }
        };
        match step {
            None => break,
            Some(true) => changed(&app),
            Some(false) => {}
        }
        tokio::time::sleep(WAIT_STEP).await;
    }
    changed(&app);
    // A moment for the notice to say so before the windows go.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // Installed (verified once more first). On Windows the installer takes over and Skykeep
    // exits inside this call; on Linux the AppImage is replaced and Skykeep starts it again.
    let installing = update.clone();
    let done = tauri::async_runtime::spawn_blocking(move || installing.install(&bytes)).await;
    match done {
        Ok(Ok(())) => {
            unstage(&app);
            relaunch(&app);
        }
        Ok(Err(e)) => fail(&app, Stage::Install, &e, &update.version),
        Err(_) => fail(
            &app,
            Stage::Install,
            &Error::InvalidUpdaterFormat,
            &update.version,
        ),
    }
}

/// Download and verify, telling the windows how far it has come.
async fn download(app: &AppHandle, update: &Update) -> Option<Vec<u8>> {
    let updates = app.state::<Updates>();
    {
        let mut i = updates.lock();
        i.phase = Phase::Downloading;
        i.received = 0;
        i.total = None;
    }
    changed(app);
    // Only from GitHub's release servers; the client would refuse anywhere else too, but this
    // says why.
    if !quince_auth::releases::url_is_release(&update.download_url) {
        fail(
            app,
            Stage::Download,
            &Error::ReleaseNotFound,
            &update.version,
        );
        return None;
    }
    let mut told = Instant::now();
    let tell = app.clone();
    let got = update
        .download(
            |chunk, total| {
                let updates = tell.state::<Updates>();
                let mut i = updates.lock();
                i.received += chunk as u64;
                i.total = total;
                drop(i);
                // A few times a second, not once a chunk.
                if told.elapsed() >= Duration::from_millis(200) {
                    told = Instant::now();
                    changed(&tell);
                }
            },
            || {},
        )
        .await;
    match got {
        Ok(bytes) => {
            let staged = {
                let app = app.clone();
                let update = update.clone();
                let bytes = bytes.clone();
                tauri::async_runtime::spawn_blocking(move || stage(&app, &update, &bytes))
                    .await
                    .ok()
                    .flatten()
            };
            let mut i = updates.lock();
            i.staged = staged;
            i.phase = Phase::Ready;
            drop(i);
            changed(app);
            Some(bytes)
        }
        Err(e) => {
            fail(app, Stage::Download, &e, &update.version);
            None
        }
    }
}

fn fail(app: &AppHandle, stage: Stage, e: &Error, version: &str) {
    quiet!("update: {stage:?} failed: {e}");
    let updates = app.state::<Updates>();
    let mut i = updates.lock();
    if is_signature(e) {
        // Never installed, never offered again this run, and nothing of it kept.
        i.refused = Some(version.to_string());
        i.staged = None;
        unstage(app);
    }
    if i.put_off.as_deref() == Some(version) && !is_signature(e) {
        // Later was chosen while it downloaded: nothing to say, it can be tried again.
        i.phase = if i.staged.is_some() {
            Phase::Ready
        } else {
            Phase::Available
        };
    } else {
        i.problem = Some(problem(stage, e));
        i.phase = Phase::Failed { notice: true };
    }
    drop(i);
    changed(app);
}

/// Start the new version: the AppImage just written, told to wait until this one has gone
/// (`--updated=<pid>`, [`after_update`]) so it is not handed to this one as a second launch.
#[cfg(target_os = "linux")]
fn relaunch(app: &AppHandle) {
    let program = app
        .env()
        .appimage
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    crate::wind_down(app);
    if let Some(program) = program {
        let _ = std::process::Command::new(program)
            .arg(format!("--updated={}", std::process::id()))
            .spawn();
    }
    // SAFETY: ends the process at once; nothing after it runs.
    unsafe { libc::_exit(0) }
}

/// Elsewhere the installer has already ended Skykeep and starts the new version itself.
#[cfg(not(target_os = "linux"))]
fn relaunch(app: &AppHandle) {
    app.restart();
}

/// Started by an update (`--updated=<pid>`): wait, briefly, for the version that started this
/// one to finish exiting. Called first thing in `main`.
pub fn after_update() {
    let Some(pid) = std::env::args().find_map(|a| {
        a.strip_prefix("--updated=")
            .and_then(|p| p.parse::<u32>().ok())
    }) else {
        return;
    };
    #[cfg(target_os = "linux")]
    {
        let gone = format!("/proc/{pid}");
        let until = Instant::now() + Duration::from_secs(10);
        while Path::new(&gone).exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

/// Whether a check made on its own should go ahead: not offline, and not on a metered
/// connection (a phone's hotspot), as this desktop's network monitor says. Elsewhere, yes:
/// a check that fails offline is kept quiet anyway.
async fn network_allows(app: &AppHandle) -> bool {
    #[cfg(target_os = "linux")]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        let asked = app.run_on_main_thread(move || {
            use gtk::gio::prelude::NetworkMonitorExt;
            let monitor = gtk::gio::NetworkMonitor::default();
            let _ = tx.send(monitor.is_network_available() && !monitor.is_network_metered());
        });
        if asked.is_err() {
            return true;
        }
        tauri::async_runtime::spawn_blocking(move || {
            rx.recv_timeout(Duration::from_secs(5)).unwrap_or(true)
        })
        .await
        .unwrap_or(true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        true
    }
}

/// Take up a download kept from before, then check shortly after launch and about once a day.
/// Only a copy that updates itself does any of it.
pub fn spawn(app: AppHandle) {
    if app.state::<Updates>().lock().install != Install::Here {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let restoring = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || restore(&restoring)).await;
        tokio::time::sleep(FIRST_CHECK).await;
        // No check is allowed now (a metered connection): a kept download is offered as it is.
        if !network_allows(&app).await {
            let updates = app.state::<Updates>();
            let mut i = updates.lock();
            if i.staged.is_some() && !matches!(i.phase, Phase::Ready) && !i.phase.applying() {
                i.phase = Phase::Ready;
                drop(i);
                changed(&app);
            }
        }
        let mut next = 0;
        loop {
            let auto = app.state::<Updates>().lock().auto;
            if auto && unix_now() >= next && network_allows(&app).await {
                next = unix_now()
                    + if check(&app, false).await {
                        EVERY_SECS
                    } else {
                        AFTER_FAILURE_SECS
                    };
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// A window was put away or closed: if it showed the notice, another window on screen takes it.
pub fn put_away(app: &AppHandle, label: &str) {
    let Some(updates) = app.try_state::<Updates>() else {
        return;
    };
    let mut i = updates.lock();
    i.typing.remove(label);
    if i.holder.as_deref() == Some(label) {
        i.holder = None;
        drop(i);
        changed(app);
    }
}

fn status(app: &AppHandle, window: &tauri::Window) -> Status {
    let updates = app.state::<Updates>();
    // Whether each window is on screen is asked before the lock is taken: the answer comes
    // from the main thread.
    let holder = updates.lock().holder.clone();
    let holder_shown = holder
        .as_deref()
        .and_then(|l| app.get_webview_window(l))
        .is_some_and(|w| w.is_visible().unwrap_or(false));
    let asker_shown = window.is_visible().unwrap_or(false);
    let mut i = updates.lock();
    let offers = match i.phase {
        Phase::Available | Phase::Ready => {
            let version = i.update.as_ref().map(|u| u.version.as_str());
            version.is_some() && i.put_off.as_deref() != version
        }
        Phase::Downloading | Phase::Waiting(_) | Phase::Installing => true,
        Phase::Failed { notice } => notice,
        _ => false,
    };
    let prompt = if offers {
        i.holder = claim(
            i.holder.as_deref(),
            holder_shown && i.holder == holder,
            window.label(),
            asker_shown,
        );
        i.holder.as_deref() == Some(window.label())
    } else {
        i.holder = None;
        false
    };
    let update = i.update.as_ref();
    Status {
        install: i.install,
        auto: i.auto,
        current: app.package_info().version.to_string(),
        phase: i.phase.name(),
        version: update.map(|u| u.version.clone()),
        notes: update
            .and_then(|u| u.body.clone())
            .filter(|n| !n.trim().is_empty()),
        received: i.received,
        total: i.total,
        waiting_for: match i.phase {
            Phase::Waiting(h) => Some(h),
            _ => None,
        },
        problem: i.problem.clone(),
        checked_at: i.checked_at,
        prompt,
    }
}

/// The updater, as this window shows it.
#[tauri::command]
pub async fn update_status(app: AppHandle, window: tauri::Window) -> Result<Status, String> {
    Ok(status(&app, &window))
}

/// Check for Updates, from Settings: always reaches out, whatever the automatic setting.
#[tauri::command]
pub async fn update_check(app: AppHandle, window: tauri::Window) -> Result<Status, String> {
    refuse_unless_here(&app)?;
    check(&app, true).await;
    Ok(status(&app, &window))
}

/// Update and Restart.
#[tauri::command]
pub async fn update_apply(app: AppHandle) -> Result<(), String> {
    refuse_unless_here(&app)?;
    tauri::async_runtime::spawn(apply(app));
    Ok(())
}

/// Later: the notice goes, in every window; a download is kept, and a restart waiting for its
/// moment does not happen. The next check of its own offers it again.
#[tauri::command]
pub async fn update_later(app: AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let mut i = updates.lock();
    if i.phase == Phase::Installing {
        return Err("Skykeep is installing the update now.".into());
    }
    i.attempt += 1;
    i.put_off = i.update.as_ref().map(|u| u.version.clone());
    i.phase = match i.phase {
        Phase::Downloading => Phase::Available,
        Phase::Waiting(_) => Phase::Ready,
        Phase::Failed { .. } if i.staged.is_some() => Phase::Ready,
        Phase::Failed { .. } if i.update.is_some() => Phase::Available,
        Phase::Failed { .. } => Phase::Idle,
        other => other,
    };
    i.problem = None;
    drop(i);
    changed(&app);
    Ok(())
}

/// Check for Updates Automatically.
#[tauri::command]
pub async fn update_auto_set(on: bool, app: AppHandle) -> Result<(), String> {
    let updates = app.state::<Updates>();
    let mut i = updates.lock();
    i.auto = on;
    remember(&app, &i);
    drop(i);
    changed(&app);
    Ok(())
}

/// A window is being typed in, or no longer is: a restart waits for it.
#[tauri::command]
pub async fn update_typing(typing: bool, app: AppHandle, window: tauri::Window) {
    let updates = app.state::<Updates>();
    let mut i = updates.lock();
    if typing {
        i.typing.insert(window.label().to_string(), Instant::now());
    } else {
        i.typing.remove(window.label());
    }
}

fn refuse_unless_here(app: &AppHandle) -> Result<(), String> {
    match app.state::<Updates>().lock().install {
        Install::Here => Ok(()),
        Install::PackageManager => Err("Updates come through your package manager.".into()),
        Install::Flatpak => Err("Updates come through Flatpak.".into()),
        Install::Store => Err("Updates come through the Microsoft Store.".into()),
        Install::Development => {
            Err("This is a development build, so it doesn't update itself.".into())
        }
        Install::Manual => Err(
            "This copy wasn't installed by Skykeep's installer, so it doesn't update itself."
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> semver::Version {
        semver::Version::parse(s).unwrap()
    }

    #[test]
    fn only_a_newer_full_release_is_an_update() {
        assert!(is_update(&v("0.1.0"), &v("0.2.0")));
        assert!(is_update(&v("0.1.0"), &v("0.1.1")));
        assert!(!is_update(&v("0.2.0"), &v("0.2.0")), "the same version");
        assert!(!is_update(&v("0.2.0"), &v("0.1.9")), "never a downgrade");
        assert!(
            !is_update(&v("0.1.0"), &v("0.2.0-beta.1")),
            "no prereleases"
        );
        assert!(!is_update(&v("0.2.0-beta.1"), &v("0.2.0-beta.2")));
        assert!(is_update(&v("0.2.0-beta.1"), &v("0.2.0")));
    }

    #[test]
    fn who_updates_each_kind_of_install() {
        use BundleType::*;
        // Arguments after `appimage`: flatpak, msix, scoop, debug.
        let here = |os, b, ai| install_kind(os, b, ai, false, false, false, false);
        assert_eq!(here("linux", Some(AppImage), true), Install::Here);
        assert_eq!(here("linux", Some(AppImage), false), Install::Manual);
        assert_eq!(here("linux", Some(Deb), false), Install::PackageManager);
        assert_eq!(here("linux", Some(Rpm), false), Install::PackageManager);
        // The AUR and distribution packages are built with cargo: no bundle type.
        assert_eq!(here("linux", None, false), Install::PackageManager);
        // Even a .deb's program started with APPIMAGE set by something else is not replaced.
        assert_eq!(here("linux", Some(Deb), true), Install::PackageManager);
        assert_eq!(here("windows", Some(Nsis), false), Install::Here);
        assert_eq!(here("windows", Some(Msi), false), Install::Here);
        assert_eq!(here("windows", None, false), Install::Manual);
        // Unpacked by Scoop, which updates it; the same files seen on Linux mean nothing.
        assert_eq!(
            install_kind("windows", Some(Nsis), false, false, false, true, false),
            Install::PackageManager
        );
        assert_eq!(
            install_kind("linux", Some(AppImage), true, false, false, true, false),
            Install::Here
        );
        assert_eq!(here("macos", Some(App), false), Install::Manual);
        assert_eq!(
            install_kind("linux", Some(AppImage), true, true, false, false, false),
            Install::Flatpak
        );
        assert_eq!(
            install_kind("linux", None, false, true, false, false, false),
            Install::Flatpak
        );
        assert_eq!(
            install_kind("windows", Some(Nsis), false, false, false, false, true),
            Install::Development
        );
        // The Store's package never updates itself: its program is the .msi's, bundle type and
        // all, but the Store replaces it — and it wins over anything Scoop-like beside it.
        for bundle in [Some(Msi), Some(Nsis), None] {
            let bundle = || bundle.clone();
            assert_eq!(
                install_kind("windows", bundle(), false, false, true, false, false),
                Install::Store
            );
            assert_eq!(
                install_kind("windows", bundle(), false, false, true, true, false),
                Install::Store
            );
        }
    }

    #[test]
    fn never_restart_with_anything_unsent_or_mid_typing() {
        assert_eq!(hold(0, 0, 0, false), None);
        assert_eq!(hold(1, 0, 0, false), Some(Hold::Changes));
        assert_eq!(hold(0, 2, 0, false), Some(Hold::Changes));
        assert_eq!(hold(0, 0, 3, false), Some(Hold::Changes));
        assert_eq!(hold(0, 0, 0, true), Some(Hold::Typing));
        // Unsent changes are said first: they are what a restart would hold up.
        assert_eq!(hold(4, 0, 0, true), Some(Hold::Changes));
    }

    #[test]
    fn the_notice_is_in_one_window_at_a_time() {
        // The first window on screen to ask takes it.
        assert_eq!(claim(None, false, "main", true).as_deref(), Some("main"));
        // A window out of sight does not.
        assert_eq!(claim(None, false, "app-notes", false), None);
        // Another window asking while it shows: it stays where it is.
        assert_eq!(
            claim(Some("main"), true, "app-notes", true).as_deref(),
            Some("main")
        );
        // Its window put away: the next on screen takes it.
        assert_eq!(
            claim(Some("main"), false, "app-notes", true).as_deref(),
            Some("app-notes")
        );
        // Its own window asking keeps it.
        assert_eq!(
            claim(Some("main"), false, "main", false).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn failures_are_said_plainly() {
        let sig = problem(Stage::Download, &Error::MissingSignedVersion);
        assert!(sig.contains("wasn't installed"), "{sig}");
        let mismatch = Error::SignedVersionMismatch {
            signed: "0.1.0".into(),
            announced: "9.9.9".into(),
        };
        assert!(is_signature(&mismatch));
        assert!(problem(Stage::Install, &mismatch).contains("signature"));
        let busy = problem(Stage::Check, &Error::ReleaseNotFound);
        assert!(busy.contains("GitHub didn't answer"), "{busy}");
        let partial = problem(Stage::Download, &Error::Network("status 502".into()));
        assert!(partial.contains("didn't finish"), "{partial}");
        assert!(!is_signature(&Error::ReleaseNotFound));
        // Never the library's own words.
        for p in [busy, partial, sig] {
            assert!(!p.contains("endpoint") && !p.contains("JSON"), "{p}");
        }
    }

    /// The latest.json the release workflow writes (.github/workflows/release.yml, "The
    /// updater's latest.json"), as it wrote it in a dry run: one the updater reads, naming each
    /// kind of install that updates itself and no plain `linux-x86_64` a .deb could fall back on.
    #[test]
    fn the_release_workflows_latest_json_is_one_the_updater_reads() {
        let written = r#"{
          "version": "0.2.0",
          "notes": "Faster Photos.\n\nCalendar fixes.",
          "pub_date": "2026-10-23T13:53:28Z",
          "platforms": {
            "linux-x86_64-appimage": { "url": "https://github.com/YashwanthKamireddi/Skykeep/releases/download/v0.2.0/Skykeep_0.2.0_amd64.AppImage", "signature": "c2ln" },
            "windows-x86_64-nsis": { "url": "https://github.com/YashwanthKamireddi/Skykeep/releases/download/v0.2.0/Skykeep_0.2.0_x64-setup.exe", "signature": "c2ln" },
            "windows-x86_64-msi": { "url": "https://github.com/YashwanthKamireddi/Skykeep/releases/download/v0.2.0/Skykeep_0.2.0_x64_en-US.msi", "signature": "c2ln" }
          }
        }"#;
        let release: tauri_plugin_updater::RemoteRelease = serde_json::from_str(written).unwrap();
        assert_eq!(release.version, v("0.2.0"));
        assert!(release
            .notes
            .as_deref()
            .unwrap()
            .contains("Calendar fixes."));
        for target in [
            "linux-x86_64-appimage",
            "windows-x86_64-nsis",
            "windows-x86_64-msi",
        ] {
            let url = release.download_url(target).unwrap();
            assert!(quince_auth::releases::url_is_release(url), "{url}");
        }
        assert!(release.download_url("linux-x86_64").is_err());
        assert!(release.download_url("linux-x86_64-deb").is_err());
    }

    #[test]
    fn prefs_default_to_checking_automatically() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(p.auto);
        let p: Prefs = serde_json::from_str(r#"{"auto":false,"checkedAt":5}"#).unwrap();
        assert!(!p.auto);
        assert_eq!(p.checked_at, Some(5));
    }
}
