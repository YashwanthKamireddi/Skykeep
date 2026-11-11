//! Photos in the app: the library's commands, the pictures served to the window from a disk
//! cache, and the download links Apple hands out (which expire) kept in memory only.
//!
//! A picture is fetched from Apple once, the first time it is drawn, and kept under the
//! app's state directory by size; after that the grid is drawn from disk. A link older than
//! LINK_LIFE, or one this run never saw, is asked of Apple again before a fetch.

use crate::{Auth, Data, UiResult};
use quince_core::photos::{Photo, View};
use quince_icloud::photos::Links;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

/// How long a download link from Apple is trusted before it is asked for again.
const LINK_LIFE: Duration = Duration::from_secs(45 * 60);

/// Every master's links seen this run, and when.
static LINKS: Mutex<Option<HashMap<String, (Links, Instant)>>> = Mutex::new(None);

/// Every link let go of: they were the last account's. Hidden locks with them.
pub fn forget_links() {
    *LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    lock_hidden();
}

/// The Hidden album, unlocked with the Apple ID password (as icloud.com asks for it) — for
/// this run, until the window locks it again (leaving Hidden, the window out of sight) or the
/// account changes. Until then nothing of Hidden is handed to the window at all: its lock is
/// not only the window's to keep.
pub(crate) static HIDDEN_OPEN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Wrong passwords in a row, and until when another is not taken: a few tries, then a wait
/// that doubles, so nobody at an unlocked computer can go through guesses (and none of them
/// reaches Apple fast enough to count against the account).
static HIDDEN_TRIES: Mutex<(u32, Option<Instant>)> = Mutex::new((0, None));

fn lock_hidden() {
    HIDDEN_OPEN.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Whether Hidden is unlocked now: what the library's reads are told, so a hidden photo in
/// Recently Deleted (and its count, and its picture made ahead) stays behind the same lock.
fn hidden_open() -> bool {
    HIDDEN_OPEN.load(std::sync::atomic::Ordering::SeqCst)
}

/// Hidden's photos are asked for: only once it is unlocked.
fn hidden_allowed(source: &quince_core::photos::Source) -> UiResult<()> {
    let hidden = matches!(source, quince_core::photos::Source::View(View::Hidden));
    if hidden && !hidden_open() {
        return Err("The Hidden album is locked.".into());
    }
    Ok(())
}

/// How long the next try waits after `wrong` wrong passwords in a row: none for the first
/// three, then 30 seconds, doubling to at most ten minutes.
fn hidden_wait(wrong: u32) -> Duration {
    if wrong < 3 {
        return Duration::ZERO;
    }
    Duration::from_secs((30u64 << (wrong - 3).min(5)).min(600))
}

/// The Hidden album unlocked with the account's Apple ID password. Wrong: said so, and after a
/// few, a wait before the next.
#[tauri::command]
pub async fn photos_unlock_hidden(password: String, auth: State<'_, Auth>) -> UiResult<()> {
    let waiting = {
        let tries = HIDDEN_TRIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tries
            .1
            .and_then(|until| until.checked_duration_since(Instant::now()))
    };
    if let Some(left) = waiting {
        let s = left.as_secs().max(1);
        return Err(format!(
            "Too many tries. Try again in {s} second{}.",
            if s == 1 { "" } else { "s" }
        ));
    }
    let right = auth
        .client
        .lock()
        .await
        .check_password(&password)
        .await
        .map_err(|e| e.to_string())?;
    let mut tries = HIDDEN_TRIES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if right {
        *tries = (0, None);
        HIDDEN_OPEN.store(true, std::sync::atomic::Ordering::SeqCst);
        return Ok(());
    }
    tries.0 = tries.0.saturating_add(1);
    let wait = hidden_wait(tries.0);
    tries.1 = (!wait.is_zero()).then(|| Instant::now() + wait);
    Err("That password isn't right.".into())
}

/// Whether Hidden can be unlocked with Windows Hello on this computer (never elsewhere).
#[tauri::command]
pub async fn photos_hello_available() -> bool {
    crate::hello::available().await
}

/// The Hidden album unlocked with Windows Hello — face, fingerprint or PIN — its prompt owned
/// by the window that asked. Answers what came of it; Verified unlocks.
#[tauri::command]
pub async fn photos_unlock_hidden_hello(
    window: tauri::WebviewWindow,
) -> UiResult<crate::hello::Hello> {
    #[cfg(windows)]
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    #[cfg(not(windows))]
    let hwnd = {
        let _ = &window;
        0
    };
    let outcome = crate::hello::verify(hwnd, "Unlock the Hidden album in Skykeep".into()).await;
    if outcome == crate::hello::Hello::Verified {
        *HIDDEN_TRIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (0, None);
        HIDDEN_OPEN.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(outcome)
}

/// The Hidden album locked again: the window left it, or went out of sight.
#[tauri::command]
pub fn photos_lock_hidden() {
    lock_hidden();
}

/// Links a pass brought, kept for the pictures it will be asked for.
pub fn remember(links: HashMap<String, Links>) {
    let now = Instant::now();
    let mut kept = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = kept.get_or_insert_with(HashMap::new);
    // Links past their life are of no use (each is asked of Apple again), so they go: kept,
    // every photo's six signed links stayed in memory for as long as the app ran.
    keep_fresh(map);
    for (master, l) in links {
        map.insert(master, (l, now));
    }
}

/// Links past their life let go of, and the room they took given back: a first pass over a
/// library of 100,000 photos brought links for every one — a few hundred megabytes — and a map
/// only pruned kept that size for as long as the app ran, long after the links had gone.
fn keep_fresh<T>(map: &mut HashMap<String, (T, Instant)>) {
    map.retain(|_, (_, at)| at.elapsed() < LINK_LIFE);
    if map.capacity() > 1024 && map.capacity() > map.len().saturating_mul(4) {
        map.shrink_to_fit();
    }
}

fn fresh_links(master: &str) -> Option<Links> {
    let kept = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    kept.as_ref()?
        .get(master)
        .filter(|(_, at)| at.elapsed() < LINK_LIFE)
        .map(|(l, _)| l.clone())
}

/// Which of a photo's pictures is wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Thumb,
    Medium,
    Original,
    /// The whole picture, displayable: for a photo zoomed in.
    Full,
    Video,
    /// A Live Photo's moving part as shot, the original `.MOV`.
    Live,
}

impl Size {
    fn name(self) -> &'static str {
        match self {
            Size::Thumb => "thumb",
            Size::Medium => "medium",
            Size::Original => "original",
            Size::Full => "full",
            Size::Video => "video",
            Size::Live => "live",
        }
    }
    fn pick(self, l: &Links) -> Option<String> {
        match self {
            Size::Thumb => l.thumb.clone(),
            Size::Medium => l.medium.clone().or_else(|| l.thumb.clone()),
            Size::Original => l.original.clone(),
            Size::Full => l.full.clone().or_else(|| l.medium.clone()),
            Size::Video => l.video.clone(),
            Size::Live => l.live.clone(),
        }
    }
}

/// A file on its way into Photos, as `quince://localhost/up/<upload id>` asks for it: the
/// upload panel's picture of it. Only a file in the upload queue is ever read — the id is
/// looked up there, never taken as a path.
pub fn requested_upload(path: &str) -> Option<String> {
    let path = crate::percent_decode(path.trim_start_matches('/'));
    let id = path.strip_prefix("up/")?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .then(|| id.to_string())
}

/// A small square of a queued file, made once and kept for the run: decoding a large photo
/// costs, so it is done off the async threads and never twice.
pub async fn upload_picture(app: &AppHandle, id: &str) -> UiResult<Vec<u8>> {
    static MADE: std::sync::OnceLock<Mutex<HashMap<String, Vec<u8>>>> = std::sync::OnceLock::new();
    let made = MADE.get_or_init(Default::default);
    if let Some(b) = made
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
    {
        return Ok(b.clone());
    }
    let u = app
        .state::<Data>()
        .0
        .lock()
        .await
        .upload(id)
        .map_err(|e| e.to_string())?
        .ok_or("No such upload.")?;
    let size = tokio::fs::metadata(&u.path)
        .await
        .map_err(|e| e.to_string())?
        .len();
    // Past this it is a video or something that would cost too much to decode for a row.
    if size > 80 * 1024 * 1024 {
        return Err("Too large to show.".into());
    }
    let bytes = tokio::fs::read(&u.path).await.map_err(|e| e.to_string())?;
    let small = tokio::task::spawn_blocking(move || quince_media::avatar(&bytes, None, 120))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    made.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id.to_string(), small.clone());
    Ok(small)
}

/// The photo and picture a `quince://` request asks for: `pic/<asset id>/<size>`, escaped as
/// Tauri escapes it — or `pic/<asset id>~<mark>/<size>`, marked as fitted pictures are (see
/// [`requested_fit`]): the same photo, whichever address the window built.
pub fn requested(path: &str) -> Option<(String, Size)> {
    let path = crate::percent_decode(path.trim_start_matches('/'));
    let rest = path.strip_prefix("pic/")?;
    let (id, size) = rest.rsplit_once('/')?;
    let (id, _) = unmarked(id)?;
    let size = match size {
        "thumb" => Size::Thumb,
        "medium" => Size::Medium,
        "original" => Size::Original,
        "full" => Size::Full,
        "video" => Size::Video,
        _ => return None,
    };
    // An asset id is a record name: letters, digits and dashes only, never a path.
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some((id.to_string(), size))
}

/// An asset id from an address's id part, and whether it carried a mark (`<id>~<mark>`, the
/// window's `pictureVersion`). A mark is a few lowercase letters and digits, never a path; an
/// address with a bad one asks for nothing.
fn unmarked(id: &str) -> Option<(&str, bool)> {
    match id.split_once('~') {
        Some((id, mark))
            if !mark.is_empty()
                && mark.len() <= 16
                && mark
                    .chars()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()) =>
        {
            Some((id, true))
        }
        Some(_) => None,
        None => Some((id, false)),
    }
}

/// The widths a picture is made at for the screen (its longer side), in steps — so a photo
/// is made a handful of times, not once per window size.
pub const FITS: [u32; 11] = [128, 256, 384, 512, 768, 1024, 1536, 2048, 2560, 3072, 3840];

/// A `quince://` request for a picture made the size it is shown at: `pic/<asset id>/fit<N>`,
/// N one of [`FITS`] — or `pic/<asset id>~<mark>/fit<N>`, the mark saying what the picture is
/// made from (the window's `pictureVersion`, of the photo's original). Whether it was marked
/// comes back too: a marked address is the same pixels for good, so it may be kept for good.
pub fn requested_fit(path: &str) -> Option<(String, u32, bool)> {
    let path = crate::percent_decode(path.trim_start_matches('/'));
    let (id, size) = path.strip_prefix("pic/")?.rsplit_once('/')?;
    let long: u32 = size.strip_prefix("fit")?.parse().ok()?;
    let (id, marked) = unmarked(id)?;
    if !FITS.contains(&long)
        || id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some((id.to_string(), long, marked))
}

/// How long the engine may keep a fitted picture: for good when its address is marked with
/// what it is made from (it cannot change without the address changing), a day when not.
pub fn fit_cache(marked: bool) -> &'static str {
    if marked {
        "private, max-age=31536000, immutable"
    } else {
        "private, max-age=86400"
    }
}

/// Pictures being made at once: one per core, so a grid filling in never starves the window.
static MAKING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(0);
static MAKING_READY: std::sync::Once = std::sync::Once::new();

/// A photo's picture made the size it is shown at, longer side `long` (see
/// `quince_media::fit`): from the smallest picture already kept that is at least that large —
/// the thumbnail for the grid, the screen-sized one or the whole one for the viewer — made
/// once and kept on disk.
pub async fn fitted(app: &AppHandle, id: &str, long: u32) -> Result<Vec<u8>, String> {
    let path = crate::state_dir(app)
        .join("photos")
        .join("fit")
        .join(long.to_string())
        .join(&*cache_name(id));
    let preview = long > crate::photo_cache::THUMB_STEP;
    if let Ok(bytes) = tokio::fs::read(&path).await {
        // The viewer's sizes are previews, let go of longest unseen first: seen now.
        if preview {
            tokio::task::spawn_blocking(move || {
                if let Ok(f) = std::fs::File::open(&path) {
                    let _ = f.set_modified(std::time::SystemTime::now());
                }
            });
        }
        return Ok(bytes);
    }
    MAKING_READY.call_once(|| {
        let cores = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
        MAKING.add_permits(cores);
    });
    let mut last = None;
    for size in [Size::Thumb, Size::Medium, Size::Full] {
        // A picture Apple has not got (a HEIC whose original cannot be read, a screenshot with
        // no screen-sized copy) is passed over: the largest there is serves, never an error
        // where a picture could be shown.
        let Ok(source) = picture(app, id, size).await else {
            continue;
        };
        let src = source.clone();
        // On-screen work takes a core each; work ahead has its own one low-priority thread.
        let _turn = if crate::cpu::is_ahead() {
            None
        } else {
            Some(MAKING.acquire().await.map_err(|e| e.to_string())?)
        };
        let made = crate::cpu::run(move || {
            let big = quince_media::long_side(&src).is_some_and(|l| l >= long);
            if !big && size != Size::Full {
                return Ok(None);
            }
            // Large enough (or the largest there is): made the size it is shown at, or, when it
            // is no larger already, shown as it is.
            quince_media::fit(&src, long).map(|out| Some(out.unwrap_or(src)))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
        if let Some(bytes) = made {
            // A grid picture is kept only while the disk has room (photo_cache::GRID_FLOOR);
            // short of it, it is shown and made again when next wanted.
            if !preview && !crate::photo_cache::grid_room(app) {
                crate::photo_cache::grid_grew(app);
                return Ok(bytes);
            }
            if let Some(dir) = path.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            let part = path.with_extension("part");
            if tokio::fs::write(&part, &bytes).await.is_ok()
                && tokio::fs::rename(&part, &path).await.is_ok()
            {
                if preview {
                    crate::photo_cache::grew(app);
                } else {
                    crate::photo_cache::grid_grew(app);
                }
            }
            return Ok(bytes);
        }
        last = Some(source);
    }
    last.ok_or_else(|| "No picture of that photo could be made.".into())
}

pub(crate) fn cache_path(app: &AppHandle, id: &str, size: Size) -> PathBuf {
    crate::state_dir(app)
        .join("photos")
        .join(size.name())
        .join(&*cache_name(id))
}

/// A record id as one name in a folder. Ids come from CloudKit and the window, and are checked
/// where a request comes in (`requested`) — and here too, where one becomes a path, so no other
/// way in can make one reach elsewhere. A record name is letters, digits, `-` and `_`, kept as
/// it is; anything else is spelled out in hex behind a `~` no record name has.
pub(crate) fn cache_name(id: &str) -> std::borrow::Cow<'_, str> {
    if !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return id.into();
    }
    let mut out = String::from("~");
    for b in id.bytes() {
        out.push_str(&format!("{b:02x}"));
    }
    out.into()
}

/// One picture of a photo: from the disk cache, or fetched from Apple once and kept.
pub async fn picture(app: &AppHandle, id: &str, size: Size) -> Result<Vec<u8>, String> {
    let path = cached(app, id, size).await?;
    tokio::fs::read(&path).await.map_err(|e| e.to_string())
}

/// One picture (or video) of a photo in the disk cache, fetched from Apple once if it is not
/// there, and its path. Streamed to a part file and moved into place: a large video is never
/// held whole in memory, and a half-written file is never served.
pub async fn cached(app: &AppHandle, id: &str, size: Size) -> Result<PathBuf, String> {
    let path = cache_path(app, id, size);
    // Held for its reader as it is found, so no trim lets it go before it is read (see
    // photo_cache): found and held in one step, never found and then lost.
    let found = {
        let p = path.clone();
        tokio::task::spawn_blocking(move || crate::photo_cache::hold_if_cached(&p))
            .await
            .unwrap_or(false)
    };
    if found {
        // Seen now: the last to go when the cache is trimmed (see `photo_cache`).
        if size != Size::Thumb {
            let touched = path.clone();
            tokio::task::spawn_blocking(move || {
                if let Ok(f) = std::fs::File::open(&touched) {
                    let _ = f.set_modified(std::time::SystemTime::now());
                }
            });
        }
        return Ok(path);
    }
    if size == Size::Full && is_heic(app, id).await {
        return heic_full(app, id, &path).await;
    }
    from_apple(app, id, size, path).await
}

/// A photo whose pictures are asked for: the library's own, or one in a shared album (with
/// where that album lives, to ask Apple for its links). Read beside a sync pass, never behind
/// it (see `LibraryLock::read`).
async fn any_photo(
    app: &AppHandle,
    id: &str,
) -> Result<Option<(Photo, Option<quince_core::shared::SharedPlace>)>, String> {
    let data = app.state::<Data>();
    if let Some(found) = data
        .0
        .read(|l| {
            let Some(p) = l.photo(id)? else {
                return Ok(None);
            };
            // A Shared Library's photo is asked of its own zone, as a shared album's is.
            let place = l
                .shared_library_of(id)?
                .map(|s| quince_core::shared::SharedPlace {
                    zone: s.zone,
                    database: s.database,
                    owner: s.owner,
                });
            Ok(Some((p, place)))
        })
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Some(found));
    }
    Ok(data
        .0
        .read(|l| l.shared_photo(id))
        .await
        .map_err(|e| e.to_string())?
        .map(|(p, place)| (p, Some(place))))
}

/// Where a photo's library is in Apple's databases, for a write to it: none for the personal
/// library, else its Shared Library's zone (in the shared database, under its owner, for one
/// this account joined).
/// Whether a photo's original is HEIC (or another HEIF).
async fn is_heic(app: &AppHandle, id: &str) -> bool {
    any_photo(app, id)
        .await
        .ok()
        .flatten()
        .and_then(|(p, _)| p.uti)
        .is_some_and(|u| {
            let u = u.to_ascii_lowercase();
            u.contains("heic") || u.contains("heif")
        })
}

/// A HEIC photo's whole picture: Apple keeps no full-size JPEG of one, so its original is
/// fetched and decoded here (quince_media::heic_to_jpeg), once, and kept like any other.
async fn heic_full(app: &AppHandle, id: &str, path: &std::path::Path) -> Result<PathBuf, String> {
    let original = from_apple(app, id, Size::Original, cache_path(app, id, Size::Original)).await?;
    let bytes = tokio::fs::read(&original)
        .await
        .map_err(|e| e.to_string())?;
    let jpeg = crate::cpu::run(move || {
        if quince_media::is_heif(&bytes) {
            quince_media::heic_to_jpeg(&bytes)
        } else {
            // Said HEIC, sent as something the engine shows as it is.
            Ok(bytes)
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let part = path.with_extension("part");
    tokio::fs::write(&part, &jpeg)
        .await
        .map_err(|e| e.to_string())?;
    crate::move_into_place(&part, path)
        .await
        .map_err(|e| e.to_string())?;
    crate::photo_cache::hold(path);
    crate::photo_cache::grew(app);
    Ok(path.to_path_buf())
}

/// Every picture and video downloaded from Apple for the screens (crate::fetching).
static DOWNLOADS: crate::fetching::Gate = crate::fetching::Gate::new(crate::fetching::AT_ONCE);

/// A picture (or video) fetched from Apple once and kept: see `cached`.
async fn from_apple(
    app: &AppHandle,
    id: &str,
    size: Size,
    path: PathBuf,
) -> Result<PathBuf, String> {
    if tokio::fs::metadata(&path).await.is_ok() {
        return Ok(path);
    }
    // A few downloads at once, each file once (crate::fetching): a first look at a large
    // library asked for hundreds together and ran the process out of file descriptors.
    let Some(turn) = DOWNLOADS.turn(&path).await else {
        return Ok(path);
    };
    // Beside a sync pass on a reader: a picture asked for never waits behind a pass's store,
    // which holds the writer for seconds on a large library.
    let (photo, shared) = any_photo(app, id)
        .await?
        .ok_or("That photo is no longer in the library.")?;
    let auth = app.state::<Auth>();
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    // A cached link first; a fresh one from Apple when it has none or Apple refuses the old.
    let fetch = |url: String| {
        let (http, path) = (http.clone(), path.clone());
        async move {
            quince_icloud::photos::fetch_to_file(&http, &url, &path)
                .await
                .map_err(|e| e.to_string())
        }
    };
    let done = match fresh_links(&photo.master).and_then(|l| size.pick(&l)) {
        Some(url) => fetch(url).await.unwrap_or(false),
        None => false,
    };
    if !done && !fetch(new_link(&http, &root, params, &photo, shared.as_ref(), size).await?).await?
    {
        return Err("Apple did not send that picture.".into());
    }
    // Held for whoever asked, and the cache checked against its cap soon after (a thumbnail is
    // held only to the disk's room: photo_cache::GRID_FLOOR).
    crate::photo_cache::hold(&path);
    // The download is done: its turn goes to the next before any decoding below.
    drop(turn);
    if size == Size::Thumb {
        crate::photo_cache::grid_grew(app);
    } else {
        crate::photo_cache::grew(app);
    }
    // A video's picture that is black (Apple makes them from the first frame) is made again
    // from the first frame that shows something, before anyone sees it — every size of it.
    if photo.kind == quince_core::photos::Kind::Video
        && matches!(size, Size::Thumb | Size::Medium | Size::Full)
    {
        repair_poster(app, id).await;
    }
    Ok(path)
}

/// A video's own picture: its first frame that shows something (see `crate::poster`), upright,
/// read once from the film and kept (`photos/poster/<id>`). An empty file there says the film
/// is dark all through, so it is not read again. `None` when there is none to be had.
async fn seen_frame(app: &AppHandle, id: &str) -> Option<Vec<u8>> {
    let kept = crate::state_dir(app)
        .join("photos")
        .join("poster")
        .join(&*cache_name(id));
    if let Ok(bytes) = tokio::fs::read(&kept).await {
        return (!bytes.is_empty()).then_some(bytes);
    }
    // The film on disk already, whichever it is; otherwise Apple's streaming copy, a few
    // megabytes, which the player wants anyway.
    let film = [Size::Original, Size::Video]
        .into_iter()
        .map(|s| cache_path(app, id, s))
        .find(|p| p.exists());
    let film = match film {
        Some(path) => path,
        None => Box::pin(cached(app, id, Size::Video)).await.ok()?,
    };
    let frame = crate::cpu::run(move || crate::poster::first_seen_frame(&film))
        .await
        .ok()?
        .ok()?;
    if let Some(dir) = kept.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    let part = kept.with_extension("part");
    if tokio::fs::write(&part, frame.as_deref().unwrap_or_default())
        .await
        .is_ok()
    {
        let _ = tokio::fs::rename(&part, &kept).await;
    }
    frame
}

/// Every picture Apple made of a video that came out black — it makes them from the first
/// frame, and a screen recording's or a fade-in's is black — made again from the video's own
/// picture (`seen_frame`), at the size Apple's was, and every size made from a black one let
/// go. Each picture is looked at on its own: the thumbnail repaired once did not stop a black
/// screen-sized picture fetched later from being shown as the video's poster and slideshow
/// still. Answers whether any was remade.
pub async fn repair_poster(app: &AppHandle, id: &str) -> bool {
    let mut frame: Option<Vec<u8>> = None;
    let mut remade = false;
    for size in [Size::Thumb, Size::Medium, Size::Full] {
        let path = cache_path(app, id, size);
        let Ok(old) = tokio::fs::read(&path).await else {
            continue;
        };
        let looked = old.clone();
        if !crate::cpu::run(move || quince_media::is_blank(&looked))
            .await
            .unwrap_or(false)
        {
            continue;
        }
        if frame.is_none() {
            frame = seen_frame(app, id).await;
        }
        let Some(src) = frame.clone() else {
            break;
        };
        // Apple's own sizes: the long side the black one had.
        let fallback = match size {
            Size::Thumb => 512,
            Size::Medium => 2048,
            _ => 4096,
        };
        let long = quince_media::long_side(&old).unwrap_or(fallback);
        let Ok(Ok(made)) =
            crate::cpu::run(move || quince_media::fit(&src, long).map(|m| m.unwrap_or(src))).await
        else {
            continue;
        };
        let part = path.with_extension("part");
        if tokio::fs::write(&part, &made).await.is_ok()
            && tokio::fs::rename(&part, &path).await.is_ok()
        {
            remade = true;
        }
    }
    if remade {
        for step in FITS {
            let _ = tokio::fs::remove_file(
                crate::state_dir(app)
                    .join("photos")
                    .join("fit")
                    .join(step.to_string())
                    .join(&*cache_name(id)),
            )
            .await;
        }
    }
    remade
}

/// The photo cache held to its cap (Settings; see `photo_cache`), the longest unseen originals
/// and previews let go of first; thumbnails always stay. Off the async threads: it walks the
/// disk. Answers the room left under the cap.
pub fn trim_cache(app: &AppHandle) -> u64 {
    let (out, limit) = crate::photo_cache::trim_now(app);
    limit.saturating_sub(out.after)
}

/// Keeps the newest photos' whole pictures, and videos, on disk ahead of being opened — as
/// Photos on a Mac keeps originals — so a photo opens at full quality and a video plays at once,
/// never waiting on the network. Newest first, two at a time, at most 600 and 3 GB a pass, after
/// each sync; one pass at a time; what is on disk already is skipped.
pub fn keep_ahead(app: AppHandle) {
    // The pictures the window shows, for whatever the sync brought.
    prepare(app.clone());
    static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        use futures_util::StreamExt;
        // Read beside a pass, and only what is used: the newest 600 and the videos' ids. The
        // whole library read as photos, twice, under the writer's lock, was 1.3 GB at a
        // million and held up every pass behind it.
        let (newest, every_video) = {
            let data = app.state::<Data>();
            let newest = data
                .0
                .read(|l| l.newest_photos(600))
                .await
                .unwrap_or_default();
            let videos = data.0.read(|l| l.video_ids()).await.unwrap_or_default();
            (newest, videos)
        };
        // Videos whose black poster was kept before posters were remade: remade now, once —
        // once per video for the life of the process. Every pass decoded every video's poster
        // again to ask (1.7 ms each, measured), and a pass follows each read of the library.
        static LOOKED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);
        let unlooked: Vec<String> = {
            let looked = LOOKED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Hidden's videos too: their posters show in its grid and its slideshow.
            every_video
                .into_iter()
                .filter(|id| looked.as_ref().is_none_or(|l| !l.contains(id)))
                .collect()
        };
        for id in unlooked {
            // Looked at only once its poster is here to look at.
            if cache_path(&app, &id, Size::Thumb).exists() {
                crate::cpu::ahead(repair_poster(&app, &id)).await;
                LOOKED
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get_or_insert_with(Default::default)
                    .insert(id);
            }
        }
        // What is on disk already, asked of the disk off the runtime's workers: a stat each, and
        // on Windows a stat is slow enough to count.
        let ask = app.clone();
        let wanted: Vec<(String, Size)> = tauri::async_runtime::spawn_blocking(move || {
            newest
                .into_iter()
                .take(600)
                // A video's streaming copy as well as its original: the copy is what the
                // window plays (H.264, its index first, a few megabytes), so a video opens
                // playing from the disk, never waiting on Apple for its first frame.
                .flat_map(|p| {
                    if p.kind == quince_core::photos::Kind::Video {
                        vec![(p.id.clone(), Size::Video), (p.id, Size::Original)]
                    } else {
                        vec![(p.id, Size::Full)]
                    }
                })
                .filter(|(id, size)| !cache_path(&ask, id, *size).exists())
                .collect()
        })
        .await
        .unwrap_or_default();
        // At most 3 GB a pass, and never past the cache's budget: a computer short of space
        // keeps fewer ahead, not a full disk.
        let room = {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || trim_cache(&app))
                .await
                .unwrap_or(0)
        };
        let budget = std::sync::atomic::AtomicU64::new(room.min(3 * 1024 * 1024 * 1024));
        futures_util::stream::iter(wanted)
            .for_each_concurrent(2, |(id, size)| {
                let (app, budget) = (app.clone(), &budget);
                async move {
                    if budget.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                        return;
                    }
                    if let Ok(path) = crate::cpu::ahead(cached(&app, &id, size)).await {
                        let len = tokio::fs::metadata(&path).await.map_or(0, |m| m.len());
                        let mut b = budget.load(std::sync::atomic::Ordering::Relaxed);
                        while let Err(now) = budget.compare_exchange_weak(
                            b,
                            b.saturating_sub(len),
                            std::sync::atomic::Ordering::Relaxed,
                            std::sync::atomic::Ordering::Relaxed,
                        ) {
                            b = now;
                        }
                    }
                }
            })
            .await;
        let trim = app.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || trim_cache(&trim)).await;
        RUNNING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

/// What the window is showing pictures at: a grid cell's width in device pixels, whether the
/// grid shows whole photos or squares, and the viewer's stage in CSS pixels with the screen's
/// pixel density. See `prepare`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub cell: f64,
    pub fit: bool,
    pub stage_w: f64,
    pub stage_h: f64,
    pub dpr: f64,
    /// The view the grid is on: its pictures are made first.
    #[serde(default)]
    pub view: Option<View>,
}

/// How many of the grid's photos `prepare` reads at a time.
const GRID_PAGE: usize = 2_000;

/// Of one page of the grid's photos (`stills_before` stills came in earlier pages), the
/// pictures to make: each still's grid picture while it is thumbnail-sized (kept always), the
/// larger grid and viewer pictures of the newest 600 stills only — previews the cache's cap
/// lets go of, made for every photo they were let go and made again after every sync — and
/// of those only the ones `missing`. Answers them and how many stills the page held.
fn wanted_steps(
    page: &[quince_core::photos::PhotoShape],
    shown: Shown,
    stills_before: usize,
    missing: impl Fn(&str, u32) -> bool,
) -> (Vec<(String, u32)>, usize) {
    let stills: Vec<&quince_core::photos::PhotoShape> = page
        .iter()
        .filter(|p| p.kind != quince_core::photos::Kind::Video)
        .collect();
    let mut out = Vec::new();
    for (i, p) in stills.iter().enumerate() {
        let (grid, viewer) = steps_of(p.width, p.height, shown);
        let newest = stills_before + i < 600;
        if grid <= crate::photo_cache::THUMB_STEP || newest {
            out.push((p.id.clone(), grid));
        }
        if newest {
            out.push((p.id.clone(), viewer));
        }
    }
    out.retain(|(id, step)| missing(id, *step));
    (out, stills.len())
}

/// The step a picture is made at for a longer side of `long` device pixels — the window's own
/// rule (fitSrc), so the picture made ahead is the one it will ask for.
fn step_for(long: f64) -> u32 {
    FITS.iter()
        .copied()
        .find(|&f| f64::from(f) >= long)
        .unwrap_or(FITS[FITS.len() - 1])
}

/// The steps one photo is shown at: in the grid (the whole cell when fitted, cropped to fill a
/// square otherwise) and in the viewer (as large as the stage allows, never past 1.5×).
fn steps_of(width: i64, height: i64, s: Shown) -> (u32, u32) {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let r = w / h;
    let tile = if s.fit {
        s.cell
    } else {
        s.cell * (w.max(h) / w.min(h))
    };
    let vw = s.stage_w.min(s.stage_h * r).min(w * 1.5);
    (step_for(tile), step_for(vw.max(vw / r) * s.dpr))
}

/// Which `prepare` is current: a newer one (the window resized, zoomed) stops the older.
static PREPARING: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LAST_SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

/// Makes every picture the window is about to show before it is shown, as Photos keeps its
/// thumbnails ready: the grid's for the whole library, the viewer's for the newest 600 — newest
/// first, what is on disk skipped, one core each. Made the first time a photo was seen, a tile
/// came in soft and turned sharp, and a photo opened on its thumbnail and snapped to the sharp
/// picture: the glitch in both. Started again whenever what is shown changes, and after a sync.
#[tauri::command]
pub fn photos_prepare(shown: Shown, app: AppHandle) {
    {
        let mut last = LAST_SHOWN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *last == Some(shown) {
            return;
        }
        *last = Some(shown);
    }
    prepare(app);
}

/// Prepare again with what the window last said it shows (after a sync brought new photos).
pub fn prepare(app: AppHandle) {
    let Some(shown) = *LAST_SHOWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    else {
        return;
    };
    let mine = PREPARING.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        use futures_util::StreamExt;
        let fit_dir = crate::state_dir(&app).join("photos").join("fit");
        // Shared albums' photos first: Apple keeps no screen-sized picture of one (only a small
        // thumbnail and the original), so made as each tile came on screen, every tile fetched
        // its original while the album was open and flickered in one by one. They are few
        // (Apple caps an album at a few thousand): all of them, before the library's.
        let shared = app
            .state::<Data>()
            .0
            .read(quince_core::Library::shared_photo_shapes)
            .await
            .unwrap_or_default();
        if !shared.is_empty() {
            let dir = fit_dir.clone();
            let (wanted, _) = tauri::async_runtime::spawn_blocking(move || {
                wanted_steps(&shared, shown, 0, |id, step| {
                    !dir.join(step.to_string()).join(&*cache_name(id)).exists()
                })
            })
            .await
            .unwrap_or_default();
            futures_util::stream::iter(wanted)
                .for_each_concurrent(4, |(id, step)| {
                    let app = app.clone();
                    async move {
                        if PREPARING.load(std::sync::atomic::Ordering::SeqCst) != mine {
                            return;
                        }
                        let _ = crate::cpu::ahead(fitted(&app, &id, step)).await;
                    }
                })
                .await;
        }
        // The grid's photos a page at a time, each read beside a pass on a reader and made
        // before the next is read: what is held at once is one page, however large the
        // library (the whole of it, read as photos under the writer's lock, was 665 MB and
        // seconds of every pass's wait at a million).
        let mut after = None;
        let mut stills_seen = 0usize;
        loop {
            if PREPARING.load(std::sync::atomic::Ordering::SeqCst) != mine {
                return;
            }
            let data = app.state::<Data>();
            let from = after.clone();
            let Ok((page, next)) = data
                .0
                .read(|l| l.grid_shapes_shown(shown.view, from.as_ref(), GRID_PAGE, hidden_open()))
                .await
            else {
                return;
            };
            let seen = stills_seen;
            let dir = fit_dir.clone();
            // A stat per picture, off the runtime's workers (see keep_ahead).
            let (wanted, stills): (Vec<(String, u32)>, usize) =
                tauri::async_runtime::spawn_blocking(move || {
                    wanted_steps(&page, shown, seen, |id, step| {
                        !dir.join(step.to_string()).join(&*cache_name(id)).exists()
                    })
                })
                .await
                .unwrap_or_default();
            stills_seen += stills;
            futures_util::stream::iter(wanted)
                .for_each_concurrent(4, |(id, step)| {
                    let app = app.clone();
                    async move {
                        if PREPARING.load(std::sync::atomic::Ordering::SeqCst) != mine {
                            return;
                        }
                        let _ = crate::cpu::ahead(fitted(&app, &id, step)).await;
                    }
                })
                .await;
            match next {
                Some(n) => after = Some(n),
                None => return,
            }
        }
    });
}

async fn new_link(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    photo: &Photo,
    shared: Option<&quince_core::shared::SharedPlace>,
    size: Size,
) -> Result<String, String> {
    let links = match shared {
        // A shared album's photo is asked of its album, where Apple keeps it.
        Some(place) => {
            let place = quince_icloud::shared_collections::Place {
                database: if place.database == "shared" {
                    quince_icloud::cloudkit::Database::Shared
                } else {
                    quince_icloud::cloudkit::Database::Private
                },
                zone: place.zone.clone(),
                owner: place.owner.clone(),
            };
            quince_icloud::shared_collections::links(
                http,
                root,
                params,
                &place,
                &photo.master,
                &photo.id,
            )
            .await
        }
        None => {
            quince_icloud::photos::links(http, root, params, None, &photo.master, &photo.id).await
        }
    }
    .map_err(|e| e.to_string())?
    .ok_or("Apple no longer has that photo.")?;
    remember(HashMap::from([(photo.master.clone(), links.clone())]));
    size.pick(&links)
        .ok_or_else(|| "That photo has no picture of that size.".into())
}

/// A film's bytes on their way to the cache: where they are landing, how many there will be,
/// how many have landed, and whether it ended (well or not). The player is served from it as it
/// grows, so a film starts within a moment instead of after its whole download.
pub struct Growth {
    pub part: PathBuf,
    /// Where it is kept once it is whole.
    pub kept: PathBuf,
    pub total: u64,
    have: std::sync::atomic::AtomicU64,
    pub done: std::sync::atomic::AtomicBool,
    failed: std::sync::atomic::AtomicBool,
    more: tokio::sync::Notify,
}

impl Growth {
    /// A film of `total` bytes about to land at `part`, to be kept at `kept`.
    pub fn new(part: PathBuf, kept: PathBuf, total: u64) -> Self {
        Growth {
            part,
            kept,
            total,
            have: std::sync::atomic::AtomicU64::new(0),
            done: std::sync::atomic::AtomicBool::new(false),
            failed: std::sync::atomic::AtomicBool::new(false),
            more: tokio::sync::Notify::new(),
        }
    }

    /// `n` more bytes are in the file.
    pub fn landed(&self, n: u64) {
        self.have.fetch_add(n, std::sync::atomic::Ordering::SeqCst);
        self.more.notify_waiters();
    }

    /// The download ended: whole and in place, or not.
    pub fn ended(&self, whole: bool) {
        use std::sync::atomic::Ordering;
        if whole {
            self.done.store(true, Ordering::SeqCst);
        } else {
            self.failed.store(true, Ordering::SeqCst);
        }
        self.more.notify_waiters();
    }

    /// Where its matrix is and what it says, once its header has landed — read a little more at
    /// a time. `None`: it has no matrix.
    pub async fn matrix(&self) -> Result<Option<(u64, [i32; 4])>, String> {
        let read = |path: PathBuf| async move {
            tokio::task::spawn_blocking(move || quince_media::video_matrix(&path))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())
        };
        let mut want = (256 * 1024).min(self.total);
        loop {
            let have = self.wait_for(want).await.ok_or("The film broke off.")?;
            let done = have >= self.total && self.done.load(std::sync::atomic::Ordering::SeqCst);
            // Renamed into place when it finished: read it there.
            let at = if done {
                self.kept.clone()
            } else {
                self.part.clone()
            };
            if let Ok(Some(found)) = read(at).await {
                return Ok(Some(found));
            }
            if done {
                return Ok(None);
            }
            want = (want * 2).min(self.total);
        }
    }

    /// Waits until the file holds `upto` bytes (or ended); answers how many it holds, or `None`
    /// when the download failed.
    pub async fn wait_for(&self, upto: u64) -> Option<u64> {
        use std::sync::atomic::Ordering;
        loop {
            let wake = self.more.notified();
            if self.failed.load(Ordering::SeqCst) {
                return None;
            }
            let have = self.have.load(Ordering::SeqCst);
            let done = self.done.load(Ordering::SeqCst);
            // All of it asked for: only once it is in place (renamed), never in that instant.
            if done || (upto < self.total && have >= upto) {
                return Some(have);
            }
            // Woken by every piece that lands; a second at most, never stuck on a missed wake.
            let _ = tokio::time::timeout(Duration::from_secs(1), wake).await;
        }
    }
}

/// Films being fetched now, by where they will be kept: one download each, however many ask.
static GROWING: Mutex<Option<HashMap<PathBuf, std::sync::Arc<Growth>>>> = Mutex::new(None);

/// Where a film is: kept whole, or on its way.
pub enum Film {
    Whole(PathBuf),
    Growing(std::sync::Arc<Growth>),
}

/// A film for the player: from the cache if it is kept, else fetched — once — with the player
/// served from the bytes as they land.
pub async fn film(app: &AppHandle, id: &str, size: Size) -> Result<Film, String> {
    let path = cache_path(app, id, size);
    let found = {
        let p = path.clone();
        tokio::task::spawn_blocking(move || crate::photo_cache::hold_if_cached(&p))
            .await
            .unwrap_or(false)
    };
    if found {
        return Ok(Film::Whole(path));
    }
    if let Some(g) = GROWING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(&path).cloned())
    {
        return Ok(Film::Growing(g));
    }
    let (photo, shared) = any_photo(app, id)
        .await?
        .ok_or("That photo is no longer in the library.")?;
    let auth = app.state::<Auth>();
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let ask = |url: String| {
        let http = http.clone();
        async move { http.get(url).send().await.map_err(|e| e.to_string()) }
    };
    let mut response = match fresh_links(&photo.master).and_then(|l| size.pick(&l)) {
        Some(url) => ask(url).await.ok().filter(|r| r.status().is_success()),
        None => None,
    };
    if response.is_none() {
        let url = new_link(&http, &root, params, &photo, shared.as_ref(), size).await?;
        response = Some(ask(url).await?).filter(|r| r.status().is_success());
    }
    let response = response.ok_or("Apple did not send that film.")?;
    let total = response.content_length().unwrap_or(0);
    if total == 0 {
        // No length said: fetched whole first, as any other picture.
        drop(response);
        return cached(app, id, size).await.map(Film::Whole);
    }
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| e.to_string())?;
    }
    let part = path.with_extension(format!("{}.part", uuid::Uuid::new_v4().simple()));
    let file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| e.to_string())?;
    let growth = std::sync::Arc::new(Growth::new(part.clone(), path.clone(), total));
    // Two asking at once: the first one's download is the one kept.
    {
        let mut growing = GROWING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let map = growing.get_or_insert_with(HashMap::new);
        if let Some(g) = map.get(&path) {
            let _ = std::fs::remove_file(&part);
            return Ok(Film::Growing(g.clone()));
        }
        map.insert(path.clone(), growth.clone());
    }
    let g = growth.clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use futures_util::StreamExt;
        use std::sync::atomic::Ordering;
        use tokio::io::AsyncWriteExt;
        let mut out = file;
        let mut body = response.bytes_stream();
        let mut ok = true;
        while let Some(chunk) = body.next().await {
            let Ok(chunk) = chunk else {
                ok = false;
                break;
            };
            if out.write_all(&chunk).await.is_err() {
                ok = false;
                break;
            }
            // Flushed as it lands, so a reader of the file finds what is counted here.
            let _ = out.flush().await;
            g.landed(chunk.len() as u64);
        }
        drop(out);
        ok = ok && g.have.load(Ordering::SeqCst) == g.total;
        let whole = ok && tokio::fs::rename(&g.part, &path).await.is_ok();
        if whole {
            // Still being played from: held, and the cap checked once it is whole.
            crate::photo_cache::hold(&path);
            crate::photo_cache::grew(&app);
        } else {
            let _ = tokio::fs::remove_file(&g.part).await;
        }
        g.ended(whole);
        GROWING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
            .map(|m| m.remove(&path));
    });
    Ok(Film::Growing(growth))
}

/// A film's turn (its matrix), read as soon as its header has landed — a film from Apple's
/// streaming copies keeps its header at the front — never waiting for the whole film.
pub async fn film_turn(app: &AppHandle, id: &str, size: Size) -> Result<[i32; 4], String> {
    let found = match film(app, id, size).await? {
        Film::Whole(path) => tokio::task::spawn_blocking(move || quince_media::video_matrix(&path))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?,
        Film::Growing(g) => g.matrix().await?,
    };
    Ok(found.map_or([1, 0, 0, 1], |(_, m)| m))
}

/// How much a film is drawn larger than its box so only its clean aperture shows (see
/// `quince_media::video_zoom`): a Live Photo's moving part then lies exactly over its still.
pub async fn film_zoom(app: &AppHandle, id: &str, size: Size) -> Result<f64, String> {
    let path = match film(app, id, size).await? {
        Film::Whole(path) => path,
        Film::Growing(g) => {
            // Its header landed (a streaming copy keeps it at the front).
            g.matrix().await?;
            if g.kept.exists() {
                g.kept.clone()
            } else {
                g.part.clone()
            }
        }
    };
    tokio::task::spawn_blocking(move || quince_media::video_zoom(&path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// What a picture's bytes are, for the window to draw them.
/// The bytes a `Range: bytes=…` header asks of a file `len` long, as `(first, last)` inclusive,
/// at most `most` of them. No header is the whole file; one this file cannot satisfy is `None`.
pub fn byte_range(header: Option<&str>, len: u64, most: u64) -> Option<(u64, u64)> {
    if len == 0 || most == 0 {
        return None;
    }
    let (first, last) = match header {
        None => (0, len - 1),
        Some(h) => {
            let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
            let (a, b) = spec.split_once('-')?;
            match (a.trim(), b.trim()) {
                // The last n bytes: where a player looks for an index at the end.
                ("", n) => {
                    let n: u64 = n.parse().ok()?;
                    if n == 0 {
                        return None;
                    }
                    (len.saturating_sub(n), len - 1)
                }
                (a, "") => (a.parse().ok()?, len - 1),
                (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(len - 1)),
            }
        }
    };
    if first > last || first >= len {
        return None;
    }
    Some((first, last.min(first.saturating_add(most - 1))))
}

/// A video's type from its first bytes: a QuickTime film says so in its first box; everything
/// else Apple keeps is MPEG-4.
pub fn video_type(head: &[u8]) -> &'static str {
    if head.get(8..12) == Some(b"qt  ") {
        "video/quicktime"
    } else {
        "video/mp4"
    }
}

pub fn content_type(bytes: &[u8], size: Size) -> &'static str {
    if size == Size::Video || bytes.get(4..8) == Some(b"ftyp") {
        return "video/mp4";
    }
    crate::image_type(bytes)
}

/// A view's photos, newest first.
#[tauri::command]
pub async fn photos_list(view: View, data: State<'_, Data>) -> UiResult<Vec<Photo>> {
    hidden_allowed(&quince_core::photos::Source::View(view))?;
    let open = hidden_open();
    data.0
        .read(|library| library.photos_shown(view, open))
        .await
        .map_err(|e| e.to_string())
}

/// What the grid lays out before any photo comes: every photo's id, the date it is grouped
/// by and its marks, a few bytes each (see `PhotoIndex`). The photos follow a page at a time,
/// as the grid comes near them — a library of fifty thousand is never sent whole.
#[tauri::command]
pub async fn photos_index(
    source: quince_core::photos::Source,
    order: Option<quince_core::photos::Order>,
    data: State<'_, Data>,
) -> UiResult<quince_core::photos::PhotoIndex> {
    hidden_allowed(&source)?;
    let open = hidden_open();
    data.0
        .read(|library| library.photo_index_shown(&source, order, open))
        .await
        .map_err(|e| e.to_string())
}

/// The library's months, each with its count and the photos that stand for it: the Years and
/// Months views. Read from the library on this machine; nothing is asked of Apple.
#[tauri::command]
pub async fn photos_months(
    data: State<'_, Data>,
) -> UiResult<Vec<quince_core::photos::PhotoMonth>> {
    data.0
        .read(|library| library.photo_months())
        .await
        .map_err(|e| e.to_string())
}

/// `limit` photos of a source, in its index's order: its first, or those just after `after`.
#[tauri::command]
pub async fn photos_page(
    source: quince_core::photos::Source,
    order: Option<quince_core::photos::Order>,
    after: Option<quince_core::photos::After>,
    limit: usize,
    data: State<'_, Data>,
) -> UiResult<Vec<Photo>> {
    hidden_allowed(&source)?;
    let open = hidden_open();
    data.0
        .read(|library| library.photo_page_shown(&source, order, after.as_ref(), limit, open))
        .await
        .map_err(|e| e.to_string())
}

/// Photos by id — a selection reaching past the pages the window holds.
#[tauri::command]
pub async fn photos_get(ids: Vec<String>, data: State<'_, Data>) -> UiResult<Vec<Photo>> {
    data.0
        .read(|library| library.photos_by_ids(&ids))
        .await
        .map_err(|e| e.to_string())
}

/// Every shared album the account made or was invited to, with its photo count and cover: as
/// the last sync found them (each album is read again whenever Apple says it changed).
#[tauri::command]
pub async fn photos_shared_albums(
    data: State<'_, Data>,
) -> UiResult<Vec<quince_core::shared::SharedAlbum>> {
    data.0
        .read(quince_core::Library::shared_albums)
        .await
        .map_err(|e| e.to_string())
}

/// Which library the views show (Both, Personal or Shared), and whether there is a Shared
/// Library to choose: the switcher shows only when there is.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryChoice {
    pub showing: quince_core::photos::PhotoLibrary,
    pub has_shared: bool,
}

#[tauri::command]
pub async fn photos_library(data: State<'_, Data>) -> UiResult<LibraryChoice> {
    data.0
        .read(|l| {
            Ok(LibraryChoice {
                showing: l.photo_library()?,
                has_shared: !l.shared_libraries()?.is_empty(),
            })
        })
        .await
        .map_err(|e: quince_core::Error| e.to_string())
}

/// The library the views show from now on, as Photos' library switcher sets it.
#[tauri::command]
pub async fn photos_set_library(
    which: quince_core::photos::PhotoLibrary,
    data: State<'_, Data>,
) -> UiResult<()> {
    data.0
        .lock()
        .await
        .set_photo_library(which)
        .map_err(|e| e.to_string())
}

/// Every photo of one burst, in the order they were shot — what "View all" shows.
#[tauri::command]
pub async fn photos_burst(burst: String, data: State<'_, Data>) -> UiResult<Vec<Photo>> {
    data.0
        .lock()
        .await
        .burst_photos(&burst)
        .map_err(|e| e.to_string())
}

/// The albums the person made, each with how many photos it holds.
#[tauri::command]
pub async fn photos_albums(
    data: State<'_, Data>,
) -> UiResult<Vec<(quince_core::photos::Album, usize, Option<String>)>> {
    let counted = data
        .0
        .read(quince_core::Library::albums_counted)
        .await
        .map_err(|e| e.to_string())?;
    Ok(counted
        .into_iter()
        .map(|(a, n, cover)| (a, usize::try_from(n).unwrap_or(0), cover))
        .collect())
}

/// Photos found by what is typed in the search field, from the library on this machine: every
/// word in a name, title, caption, keyword, year or month.
#[tauri::command]
pub async fn photos_search(query: String, data: State<'_, Data>) -> UiResult<Vec<Photo>> {
    data.0
        .read(|library| library.search_photos(&query, 20_000))
        .await
        .map_err(|e| e.to_string())
}

/// One album's photos, in its order.
#[tauri::command]
pub async fn photos_in_album(album: String, data: State<'_, Data>) -> UiResult<Vec<Photo>> {
    data.0
        .read(|library| library.album_photos(&album))
        .await
        .map_err(|e| e.to_string())
}

/// A new album — or folder, `folder` — first in its list as Photos puts one, at the top or in
/// the folder `parent`. Answers it as Apple made it.
#[tauri::command]
pub async fn photos_album_create(
    name: String,
    parent: Option<String>,
    folder: Option<bool>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<quince_core::photos::Album> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An album needs a name.".into());
    }
    let _one = crate::PHOTOS_PASS.lock().await;
    let first = data
        .0
        .lock()
        .await
        .albums()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|a| a.position)
        .min();
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let album = quince_icloud::photos::create_album(
        &http,
        &root,
        params,
        name,
        first,
        parent.as_deref(),
        folder.unwrap_or(false),
    )
    .await
    .map_err(|e| e.to_string())?;
    data.0
        .lock()
        .await
        .put_album(&album)
        .map_err(|e| e.to_string())?;
    Ok(album)
}

/// An album renamed, against the change tag last seen.
#[tauri::command]
pub async fn photos_album_rename(
    id: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An album needs a name.".into());
    }
    let _one = crate::PHOTOS_PASS.lock().await;
    let mut album = album_of(&data, &id).await?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let tag = quince_icloud::photos::rename_album(
        &http,
        &root,
        params,
        &id,
        album.change_tag.as_deref(),
        name,
    )
    .await
    .map_err(|e| e.to_string())?;
    album.name = name.to_string();
    album.change_tag = tag.or(album.change_tag);
    data.0
        .lock()
        .await
        .put_album(&album)
        .map_err(|e| e.to_string())
}

/// An album, or a folder with all it holds, deleted. Their photos stay in the library.
#[tauri::command]
pub async fn photos_album_delete(
    id: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = crate::PHOTOS_PASS.lock().await;
    album_of(&data, &id).await?;
    // A folder goes with everything in it, as icloud.com says it will ("…will also delete all
    // albums inside this folder"). Apple's server does not do that itself — checked live: an
    // album inside a deleted folder stayed, its folder gone — so the ones inside are deleted
    // here, the deepest first, and the folder last.
    let all = data
        .0
        .read(|l| l.albums())
        .await
        .map_err(|e| e.to_string())?;
    let mut order: Vec<quince_core::photos::Album> = Vec::new();
    fn within(
        all: &[quince_core::photos::Album],
        id: &str,
        out: &mut Vec<quince_core::photos::Album>,
    ) {
        for a in all.iter().filter(|a| a.parent.as_deref() == Some(id)) {
            within(all, &a.id, out);
            out.push(a.clone());
        }
    }
    within(&all, &id, &mut order);
    if let Some(me) = all.iter().find(|a| a.id == id) {
        order.push(me.clone());
    }
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    for a in order {
        quince_icloud::photos::delete_album(
            &http,
            &root,
            params.clone(),
            &a.id,
            a.change_tag.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;
        data.0
            .lock()
            .await
            .forget_album(&a.id)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Photos put in an album after what is there. One already in it is not added twice.
#[tauri::command]
pub async fn photos_album_add(
    album: String,
    ids: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = crate::PHOTOS_PASS.lock().await;
    album_of(&data, &album).await?;
    let (fresh, last) = {
        let library = data.0.lock().await;
        let inside: std::collections::HashSet<String> = library
            .album_photos(&album)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let fresh: Vec<String> = ids.into_iter().filter(|i| !inside.contains(i)).collect();
        let last = library
            .album_last_position(&album)
            .map_err(|e| e.to_string())?;
        (fresh, last)
    };
    if fresh.is_empty() {
        return Ok(());
    }
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let (placed, added) =
        quince_icloud::photos::add_to_album(&http, &root, params, &album, &fresh, last).await;
    // What went in before a failure is in the album on Apple; the library says so too.
    data.0
        .lock()
        .await
        .put_album_items(
            &placed
                .into_iter()
                .map(|(asset, at)| (album.clone(), asset, at))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?;
    added.map_err(|e| e.to_string())
}

/// Photos taken out of an album. They stay in the library.
#[tauri::command]
pub async fn photos_album_remove(
    album: String,
    ids: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let _one = crate::PHOTOS_PASS.lock().await;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    quince_icloud::photos::remove_from_album(&http, &root, params, &album, &ids)
        .await
        .map_err(|e| e.to_string())?;
    data.0
        .lock()
        .await
        .forget_album_items(
            &ids.into_iter()
                .map(|i| (album.clone(), i))
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())
}

async fn album_of(data: &State<'_, Data>, id: &str) -> UiResult<quince_core::photos::Album> {
    data.0
        .lock()
        .await
        .albums()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| "That album is no longer there.".into())
}

/// Favorite, Hide, Delete (to Recently Deleted), Recover, or (from Recently Deleted only)
/// delete for good, on one photo: [`photos_set_flag`] of it alone. Answers the photo as it now
/// is, or (deleted for good, gone from view) as it was.
#[tauri::command]
pub async fn photo_set_flag(
    id: String,
    flag: quince_icloud::photos::Flag,
    on: bool,
    data: State<'_, Data>,
) -> UiResult<Photo> {
    let was = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let now = photos_set_flag(vec![id], flag, on, data).await?;
    Ok(now.into_iter().next().unwrap_or(was))
}

/// One flag on many photos (a selection deleted, recovered, hidden, favorited, or deleted for
/// good from Recently Deleted) as Photos does it: recorded and shown at once in every view, and
/// sent to Apple behind by the outbox worker, in batches, each library's photos to its own
/// zone; whatever Apple refuses is put back and kept to be sent again or let go
/// (quince_sync::photos, the phone's own code). Answers the photos as they now are.
#[tauri::command]
pub async fn photos_set_flag(
    ids: Vec<String>,
    flag: quince_icloud::photos::Flag,
    on: bool,
    data: State<'_, Data>,
) -> UiResult<Vec<Photo>> {
    let now = {
        let mut library = data.0.lock().await;
        quince_sync::photos::record_flags(&mut library, &ids, flag, on)?
    };
    crate::outbox::kick();
    Ok(now)
}

/// A photo's title and caption, as typed in its Info: Apple's answer taken at once. Empty clears one.
#[tauri::command]
pub async fn photo_set_text(
    id: String,
    title: String,
    caption: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Photo> {
    let _one = crate::PHOTOS_PASS.lock().await;
    let mut photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let (title, caption) = (title.trim().to_string(), caption.trim().to_string());
    // Only what was changed here goes: the other field may have been changed on another device.
    let title_changed = photo.title.as_deref().unwrap_or("") != title;
    let caption_changed = photo.caption.as_deref().unwrap_or("") != caption;
    let place = quince_sync::photos::library_place(&*data.0.lock().await, &id)?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let answer = quince_icloud::photos::set_title_and_caption(
        &http,
        &root,
        params,
        place.as_ref(),
        &id,
        title_changed.then_some(title.as_str()),
        caption_changed.then_some(caption.as_str()),
    )
    .await
    .map_err(|e| e.to_string())?;
    let Some(answer) = answer else {
        return Ok(photo);
    };
    if title_changed {
        photo.title = (!title.is_empty()).then_some(title);
    }
    if caption_changed {
        photo.caption = (!caption.is_empty()).then_some(caption);
    }
    photo.change_tag = answer
        .get("recordChangeTag")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(photo.change_tag);
    data.0
        .lock()
        .await
        .put_photos(std::slice::from_ref(&photo))
        .map_err(|e| e.to_string())?;
    Ok(photo)
}

/// A photo's star rating, 0 to 5, as set in its Info: Apple's answer taken at once.
#[tauri::command]
pub async fn photo_set_rating(
    id: String,
    rating: i64,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Photo> {
    if !(0..=5).contains(&rating) {
        return Err("A rating is from none to five stars.".into());
    }
    let _one = crate::PHOTOS_PASS.lock().await;
    let mut photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let place = quince_sync::photos::library_place(&*data.0.lock().await, &id)?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let answer = quince_icloud::photos::set_rating(
        &http,
        &root,
        params,
        place.as_ref(),
        &id,
        photo.change_tag.as_deref(),
        rating,
    )
    .await
    .map_err(|e| e.to_string())?;
    photo.rating = rating;
    photo.change_tag = answer
        .get("recordChangeTag")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(photo.change_tag);
    data.0
        .lock()
        .await
        .put_photos(std::slice::from_ref(&photo))
        .map_err(|e| e.to_string())?;
    Ok(photo)
}

/// How a video's picture is turned, as its file records it (`[a, b, c, d]`, see
/// quince_media::video_matrix): the window draws the turn itself, since the film is served
/// upright (video.rs). `which` is the film the window plays: "original" (a video) or "video"
/// (a Live Photo's moving part). Fetched once if it is not on disk yet, as playing it would.
#[tauri::command]
pub async fn photo_video_turn(id: String, which: String, app: AppHandle) -> UiResult<[i32; 4]> {
    let size = if which == "video" {
        Size::Video
    } else {
        Size::Original
    };
    film_turn(&app, &id, size).await
}

/// How much larger than its box a film is drawn (see `film_zoom`).
#[tauri::command]
pub async fn photo_video_zoom(id: String, which: String, app: AppHandle) -> UiResult<f64> {
    let size = if which == "video" {
        Size::Video
    } else {
        Size::Original
    };
    film_zoom(&app, &id, size).await
}

/// A video opened in this computer's own player. WebKitGTK plays nothing without GStreamer's
/// codec plugins, which many systems (his among them) do not have, so a video is fetched once,
/// kept beside the other pictures under its own name, and handed to the system's player.
#[tauri::command]
pub async fn photo_play(id: String, app: AppHandle, data: State<'_, Data>) -> UiResult<()> {
    let photo = data
        .0
        .lock()
        .await
        .photo_to_view(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let size = if photo.kind == quince_core::photos::Kind::Video {
        Size::Original
    } else {
        Size::Video
    };
    let source = cached(&app, &id, size).await?;
    // Cleaned as a note attachment's name is: a shared album's item is named by whoever
    // shared it, and a name is no place for a path, a hidden file or a reversed extension.
    let name = match photo.filename.as_deref() {
        Some(given) => crate::attachment_name(Some(given), &id),
        None => format!("{id}.mp4"),
    };
    let dir = crate::state_dir(&app)
        .join("photos")
        .join("open")
        .join(&*cache_name(&id));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let path = dir.join(&name);
    // Under its own name for the player, linked to the cached file rather than copied.
    if tokio::fs::metadata(&path).await.is_err()
        && tokio::fs::hard_link(&source, &path).await.is_err()
    {
        tokio::fs::copy(&source, &path)
            .await
            .map_err(|e| e.to_string())?;
    }
    use tauri_plugin_opener::OpenerExt;
    // A video plays in a player; anything that would run instead (a program dressed as a
    // film) is shown in its folder, never started.
    let head = {
        use tokio::io::AsyncReadExt;
        let mut head = vec![0u8; 512];
        let n = match tokio::fs::File::open(&path).await {
            Ok(mut f) => f.read(&mut head).await.unwrap_or(0),
            Err(_) => 0,
        };
        head.truncate(n);
        head
    };
    crate::saving::mark_from_internet(&path);
    if crate::runs_code(&name, &head) {
        return app
            .opener()
            .reveal_item_in_dir(&path)
            .map_err(|e| e.to_string());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("No app on this computer opens that video: {e}"))
}

/// A photo's keywords, as typed in its Info: each trimmed, empty and repeated ones dropped,
/// Apple's answer taken at once.
#[tauri::command]
pub async fn photo_set_keywords(
    id: String,
    keywords: Vec<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Photo> {
    let mut words: Vec<String> = Vec::new();
    for k in keywords {
        let k = k.trim().to_string();
        if !k.is_empty() && !words.iter().any(|w| w.eq_ignore_ascii_case(&k)) {
            words.push(k);
        }
    }
    let _one = crate::PHOTOS_PASS.lock().await;
    let mut photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let place = quince_sync::photos::library_place(&*data.0.lock().await, &id)?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let (answer, kept) = quince_icloud::photos::set_keywords(
        &http,
        &root,
        params,
        place.as_ref(),
        &id,
        &photo.keywords,
        &words,
    )
    .await
    .map_err(|e| e.to_string())?;
    // Apple's list now: this change, and any word another device added meanwhile.
    photo.keywords = kept;
    photo.change_tag = answer
        .get("recordChangeTag")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(photo.change_tag);
    data.0
        .lock()
        .await
        .put_photos(std::slice::from_ref(&photo))
        .map_err(|e| e.to_string())?;
    Ok(photo)
}

/// When a photo was taken, as Adjust Date and Time sets it (unix seconds): Apple's answer taken
/// at once.
#[tauri::command]
pub async fn photo_set_date(
    id: String,
    taken_at: i64,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Photo> {
    // Photos takes nothing before 1900 or past the end of 9999 — and neither does a camera.
    if !(-2_208_988_800..253_402_300_800).contains(&taken_at) {
        return Err("That date is out of range.".into());
    }
    let _one = crate::PHOTOS_PASS.lock().await;
    let mut photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let place = quince_sync::photos::library_place(&*data.0.lock().await, &id)?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let answer = quince_icloud::photos::set_date(
        &http,
        &root,
        params,
        place.as_ref(),
        &id,
        photo.change_tag.as_deref(),
        taken_at * 1000,
    )
    .await
    .map_err(|e| e.to_string())?;
    photo.taken_at = taken_at;
    photo.change_tag = answer
        .get("recordChangeTag")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or(photo.change_tag);
    data.0
        .lock()
        .await
        .put_photos(std::slice::from_ref(&photo))
        .map_err(|e| e.to_string())?;
    Ok(photo)
}

/// Where a photo was taken, opened in a map in the browser: OpenStreetMap at its coordinates.
/// Apple's place names come from Apple Maps, which is not here; the place itself is, opened in
/// this computer's own map app.
#[tauri::command]
pub async fn photo_open_map(id: String, app: AppHandle, data: State<'_, Data>) -> UiResult<()> {
    let photo = data
        .0
        .lock()
        .await
        .photo(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    let (Some(lat), Some(lon)) = (photo.latitude, photo.longitude) else {
        return Err("This photo has no place.".into());
    };
    // The place handed to this computer's own map app (a geo: link, RFC 5870, as GNOME Maps
    // and the like take it) — never to a website: the photo's location goes nowhere else.
    #[cfg(target_os = "linux")]
    {
        let handled = std::process::Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/geo"])
            .output()
            .is_ok_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty());
        if !handled {
            return Err(format!(
                "No map app on this computer opens places. The place is {lat:.6}, {lon:.6}."
            ));
        }
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(format!("geo:{lat:.6},{lon:.6}"), None::<&str>)
        .map_err(|e| format!("The map app did not open: {e}"))
}

/// How many photos each view holds. Asked after every sync, which is also when an upload queue
/// paused for a session coming back starts again.
#[tauri::command]
pub async fn photos_counts(app: AppHandle, data: State<'_, Data>) -> UiResult<Vec<(View, i64)>> {
    work(app.clone());
    keep_ahead(app);
    let open = hidden_open();
    data.0
        .read(|library| library.photo_counts_shown(open))
        .await
        .map_err(|e| e.to_string())
}

/// A photo put on the clipboard as a PNG, as Photos' Copy puts one, to paste into any app with
/// nothing saved first. Made from the best picture already on this computer — the original when
/// it is here (a HEIC decoded, never put there as HEIC), else the whole picture kept for the
/// viewer — and fetched only when neither is. Read-only: nothing is asked of iCloud but the
/// picture itself. Decoded and made a PNG off the window's thread.
///
/// On Linux it is offered on the clipboard here, through GTK — the window's own toolkit, which
/// speaks Wayland's clipboard and X11's alike — and nothing comes back; on Windows it is set
/// natively, as a PNG and a bitmap (`win_clipboard`). Elsewhere the PNG comes back for the
/// window to put there (the webview's own clipboard, which on macOS takes a PNG as the
/// system's picture type).
#[tauri::command]
pub async fn photo_copy(
    id: String,
    app: AppHandle,
    data: State<'_, Data>,
) -> UiResult<tauri::ipc::Response> {
    let photo = data
        .0
        .lock()
        .await
        .photo_to_view(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    if photo.kind == quince_core::photos::Kind::Video {
        return Err("A video can’t be copied as a picture.".into());
    }
    let (original, full) = (
        cache_path(&app, &id, Size::Original),
        cache_path(&app, &id, Size::Full),
    );
    let png_of = |path: PathBuf| async move {
        crate::photo_cache::hold(&path);
        let bytes = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
        crate::cpu::run(move || picture_for_clipboard(&bytes)).await?
    };
    if tokio::fs::metadata(&full).await.is_err() && tokio::fs::metadata(&original).await.is_err() {
        // Neither here yet: the whole picture, as the viewer would fetch it (a HEIC's comes with
        // its original, which is then the one used).
        cached(&app, &id, Size::Full).await?;
    }
    // The original first; one this cannot decode (a RAW) gives way to the whole picture.
    let from_original = if tokio::fs::metadata(&original).await.is_ok() {
        png_of(original).await
    } else {
        Err(String::new())
    };
    let png = match from_original {
        Ok(png) => png,
        Err(_) => png_of(cached(&app, &id, Size::Full).await?).await?,
    };
    #[cfg(target_os = "linux")]
    {
        offer_png(&app, png).await?;
        Ok(tauri::ipc::Response::new(Vec::new()))
    }
    #[cfg(windows)]
    {
        let _ = app;
        crate::cpu::run(move || win_clipboard::put(&png.0, &png.1)).await??;
        Ok(tauri::ipc::Response::new(Vec::new()))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    Ok(tauri::ipc::Response::new(png))
}

/// What Copy puts on the clipboard, made from a photo's bytes: a PNG; on Windows the same
/// picture as a bitmap besides (`CF_DIB`), the type every Windows app pastes.
#[cfg(not(windows))]
fn picture_for_clipboard(bytes: &[u8]) -> Result<Vec<u8>, String> {
    quince_media::to_png(bytes)
}
#[cfg(windows)]
fn picture_for_clipboard(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    quince_media::to_png_and_dib(bytes)
}

/// The one type Copy offers on Linux: the PNG, as a picture. No text type is offered, so an
/// app that pastes text gets nothing rather than the picture's bytes as characters.
#[cfg(any(target_os = "linux", test))]
const PNG_TARGET: &str = "image/png";

/// The bytes to answer a paste that asked for `target`: the PNG only when it asked for one.
#[cfg(any(target_os = "linux", test))]
fn answer_for<'a>(target: &str, png: &'a [u8]) -> Option<&'a [u8]> {
    (target == PNG_TARGET).then_some(png)
}

/// Windows' clipboard, set from here rather than by the window: the PNG under the registered
/// "PNG" format (browsers, chat apps, Office) and the same picture as `CF_DIB` (everything
/// else; Windows makes `CF_BITMAP` and `CF_DIBV5` from it for apps that ask for those). Set
/// natively, it needs no permission from the webview and is there however long the photo took
/// to convert.
#[cfg(windows)]
mod win_clipboard {
    use windows::core::w;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Ole::CF_DIB;

    const FAILED: &str = "The clipboard didn’t take the photo.";

    /// `bytes` in memory the clipboard can own.
    fn global(bytes: &[u8]) -> Result<HGLOBAL, String> {
        unsafe {
            let mem = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).map_err(|_| FAILED)?;
            let at = GlobalLock(mem).cast::<u8>();
            if at.is_null() {
                let _ = GlobalFree(Some(mem));
                return Err(FAILED.into());
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), at, bytes.len());
            let _ = GlobalUnlock(mem);
            Ok(mem)
        }
    }

    /// One format set; the memory is the clipboard's once it is taken, freed here if not.
    fn set(format: u32, bytes: &[u8]) -> Result<(), String> {
        let mem = global(bytes)?;
        unsafe {
            if SetClipboardData(format, Some(HANDLE(mem.0))).is_err() {
                let _ = GlobalFree(Some(mem));
                return Err(FAILED.into());
            }
        }
        Ok(())
    }

    pub fn put(png: &[u8], dib: &[u8]) -> Result<(), String> {
        unsafe {
            // Another app may hold the clipboard for a moment: tried for half a second.
            let mut opened = false;
            for _ in 0..20 {
                if OpenClipboard(None).is_ok() {
                    opened = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if !opened {
                return Err(FAILED.into());
            }
            let result = EmptyClipboard()
                .map_err(|_| FAILED.to_string())
                .and_then(|()| set(u32::from(CF_DIB.0), dib))
                .and_then(|()| match RegisterClipboardFormatW(w!("PNG")) {
                    0 => Err(FAILED.into()),
                    png_format => set(png_format, png),
                });
            let _ = CloseClipboard();
            result
        }
    }
}

/// A PNG offered on the desktop's clipboard through GTK, on the thread GTK lives on. Held by
/// the clipboard until something else is copied; a clipboard manager keeps it past that.
#[cfg(target_os = "linux")]
async fn offer_png(app: &AppHandle, png: Vec<u8>) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
        let targets = [gtk::TargetEntry::new(
            PNG_TARGET,
            gtk::TargetFlags::empty(),
            0,
        )];
        let offered = clipboard.set_with_data(&targets, move |_, selection, _| {
            if let Some(bytes) = answer_for(&selection.target().name(), &png) {
                selection.set(&gdk::Atom::intern(PNG_TARGET), 8, bytes);
            }
        });
        let _ = tx.send(offered);
    })
    .map_err(|e| e.to_string())?;
    match rx.await {
        Ok(true) => Ok(()),
        _ => Err("The clipboard didn’t take the photo.".into()),
    }
}

/// A photo saved to Downloads, as icloud.com's Download Options choose the file: the original
/// as captured, the highest quality (edits included), or the most compatible (JPEG or MP4). Both
/// records are read fresh — their links expire — and the file is streamed to a part file, named
/// only once whole: a large video never sits in memory, and a broken download leaves no half file
/// under the photo's name. Answers where it went.
#[tauri::command]
pub async fn photo_download(
    id: String,
    want: Option<quince_icloud::photos::Want>,
    app: AppHandle,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<String> {
    let want = want.unwrap_or(quince_icloud::photos::Want::Originals);
    let photo = data
        .0
        .lock()
        .await
        .photo_to_view(&id)
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in the library.")?;
    // Asked of the zone the photo lives in: its Shared Library's, its shared album's, or the
    // personal library's.
    let place = {
        let library = data.0.lock().await;
        match quince_sync::photos::library_place(&library, &id)? {
            Some(p) => Some(p),
            None => library
                .shared_photo(&id)
                .map_err(|e| e.to_string())?
                .map(|(_, s)| quince_icloud::shared_collections::Place {
                    database: if s.database == "shared" {
                        quince_icloud::cloudkit::Database::Shared
                    } else {
                        quince_icloud::cloudkit::Database::Private
                    },
                    zone: s.zone,
                    owner: s.owner,
                }),
        }
    };
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        &http,
        &root,
        quince_icloud::photos::CONTAINER,
        params,
    );
    let (zc, zone) = match &place {
        Some(p) => (
            zc.in_database(p.database).owned_by(p.owner.clone()),
            p.zone.clone(),
        ),
        None => {
            let zone = quince_icloud::photos::library_zone(&zc).await;
            (zc, zone)
        }
    };
    let asset = zc
        .lookup_record(&zone, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("That photo is no longer in iCloud.")?;
    let master = zc
        .lookup_record(&zone, &photo.master)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("That photo's file is no longer in iCloud.")?;
    let video = photo.kind == quince_core::photos::Kind::Video;
    let (url, name) = quince_icloud::photos::download_choice(&asset, &master, want, video)
        .ok_or("iCloud has no file of that kind for this photo.")?;
    let dir = app
        .path()
        .download_dir()
        .map_err(|_| "There is no Downloads folder to save to.")?;
    // A shared album's file names are other people's: one plain name in Downloads, or none.
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    let fallback = format!("Photo {}", id.chars().take(8).collect::<String>());
    let path = crate::saving::save_path(&dir, &name, &fallback)?;
    stream_to_new_file(&http, &url, &path)
        .await
        .map_err(String::from)?;
    crate::saving::mark_from_internet(&path);
    Ok(path.display().to_string())
}

/// Why a file from iCloud didn't arrive: Apple's answer when it gave one, or the disk's
/// trouble, so a caller can tell one photo's problem from an outage.
#[derive(Debug)]
pub(crate) struct Fetch {
    /// The HTTP status Apple answered with; `None` when the connection broke or never formed.
    pub status: Option<u16>,
    /// The disk refused the write (full, gone, not ours to write).
    pub disk: Option<std::io::ErrorKind>,
    pub message: String,
}

impl From<Fetch> for String {
    fn from(f: Fetch) -> String {
        f.message
    }
}

fn disk(e: &std::io::Error) -> Fetch {
    Fetch {
        status: None,
        disk: Some(e.kind()),
        message: e.to_string(),
    }
}

/// A file from iCloud written to `path`, which must not exist: streamed into a `.part` beside
/// it (a large video never sits in memory) and named only once whole, so a broken download
/// leaves no half file under the real name. Answers how many bytes it holds.
pub(crate) async fn stream_to_new_file(
    http: &reqwest::Client,
    url: &str,
    path: &std::path::Path,
) -> Result<u64, Fetch> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let part = path.with_extension(format!(
        "{}.part",
        path.extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    let response = http.get(url).send().await.map_err(|e| Fetch {
        status: e.status().map(|s| s.as_u16()),
        disk: None,
        message: format!("iCloud did not send the file: {e}"),
    })?;
    if let Err(e) = response.error_for_status_ref() {
        return Err(Fetch {
            status: e.status().map(|s| s.as_u16()),
            disk: None,
            message: format!("iCloud did not send the file: {e}"),
        });
    }
    // A file of its own, never one already there (nor a link left in its place).
    let mut out = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&part)
        .await
        .map_err(|e| disk(&e))?;
    let mut body = response.bytes_stream();
    let mut len = 0u64;
    let written: Result<(), Fetch> = async {
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| Fetch {
                status: None,
                disk: None,
                message: format!("The download broke off: {e}"),
            })?;
            len += chunk.len() as u64;
            out.write_all(&chunk).await.map_err(|e| disk(&e))?;
        }
        out.flush().await.map_err(|e| disk(&e))
    }
    .await;
    if let Err(e) = written {
        tokio::fs::remove_file(&part).await.ok();
        return Err(e);
    }
    drop(out);
    crate::move_into_place(&part, path)
        .await
        .map_err(|e| disk(&e))?;
    Ok(len)
}

/// Where Photos uploads tell the window how they are going: the same shape Drive's tray reads.
pub const PHOTO_UPLOAD: &str = "quince://photo-upload";

#[derive(Clone, serde::Serialize)]
struct UploadEvent {
    id: String,
    batch: String,
    name: String,
    within: String,
    sent: u64,
    total: u64,
    state: &'static str,
    folder: bool,
    error: Option<String>,
}

fn say(
    app: &AppHandle,
    u: &quince_core::photos::Upload,
    state: &'static str,
    sent: u64,
    total: u64,
    error: Option<String>,
) {
    use tauri::Emitter;
    app.emit(
        PHOTO_UPLOAD,
        UploadEvent {
            id: u.id.clone(),
            batch: u.batch.clone(),
            name: u.name.clone(),
            within: String::new(),
            sent,
            total,
            state,
            folder: false,
            error,
        },
    )
    .ok();
}

/// One worker at a time drains the queue, [`SENDERS`] files at once.
static WORKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Batches, and single uploads, asked to stop.
static CANCELLED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The uploads on their way (up to [`SENDERS`]), so Cancel can stop each mid-file rather than
/// after it lands.
static CURRENT: Mutex<Vec<(String, String, futures_util::future::AbortHandle)>> =
    Mutex::new(Vec::new());
/// Files sent at once: each upload is three round trips to Apple, so a few at a time keep the
/// connection busy, as icloud.com sends an import.
const SENDERS: usize = 3;

fn cancelled(u: &quince_core::photos::Upload) -> bool {
    CANCELLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|b| *b == u.batch || *b == u.id)
}

/// Files (and the photos and videos inside folders) added to Photos. Each is checked against
/// Photos' limits first; a file Photos would refuse is answered at once with why, and never
/// sent. Answers the batch, whose progress comes as `quince://photo-upload` events.
#[tauri::command]
pub async fn photos_upload(
    paths: Vec<String>,
    album: Option<String>,
    app: AppHandle,
    data: State<'_, Data>,
) -> UiResult<String> {
    for p in &paths {
        crate::grants::take_read(p)?;
    }
    let batch = uuid::Uuid::new_v4().to_string();
    let mut files = Vec::new();
    for p in paths {
        let path = PathBuf::from(&p);
        if path.is_dir() {
            let mut stack = vec![path];
            while let Some(dir) = stack.pop() {
                let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
                    continue;
                };
                while let Ok(Some(e)) = entries.next_entry().await {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else {
                        files.push(p);
                    }
                }
            }
        } else {
            files.push(path);
        }
    }
    files.sort();
    queue_files(&app, &data, batch, files, album).await
}

/// Files queued for Photos as one batch, each checked against Photos' limits first (a file
/// Photos would refuse is failed at once with why), and the queue started: what the window's
/// uploads and Send to iPhone share. Answers the batch.
pub(crate) async fn queue_files(
    app: &AppHandle,
    data: &Data,
    batch: String,
    files: Vec<PathBuf>,
    album: Option<String>,
) -> UiResult<String> {
    let now = jiff::Timestamp::now().as_second();
    let mut queued = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let size = tokio::fs::metadata(&path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let mut u = quince_core::photos::Upload {
            id: uuid::Uuid::new_v4().to_string(),
            batch: batch.clone(),
            path: path.to_string_lossy().into_owned(),
            name: name.clone(),
            state: "waiting".into(),
            error: None,
            asset: None,
            added_at: now,
            album: album.clone(),
        };
        if let Some(r) = quince_icloud::photos_upload::refusal(&name, size) {
            u.state = "failed".into();
            u.error = Some(r.words());
        }
        queued.push((u, size));
    }
    if queued.is_empty() {
        return Err("There are no photos or videos there to add.".into());
    }
    data.0
        .lock()
        .await
        .queue_uploads(&queued.iter().map(|(u, _)| u.clone()).collect::<Vec<_>>())
        .map_err(|e| e.to_string())?;
    for (u, size) in &queued {
        say(
            app,
            u,
            if u.state == "failed" {
                "failed"
            } else {
                "waiting"
            },
            0,
            *size,
            u.error.clone(),
        );
    }
    if let Some((u, _)) = queued.first() {
        say(app, u, "planned", 0, 0, None);
    }
    work(app.clone());
    Ok(batch)
}

/// A file put in the Photos folder, added to Photos (and to `album`, when it was put in one)
/// through the same queue as the window's own uploads — the tray shows it, a quit resumes it,
/// Photos' limits refuse it with the reason — answering the photo Apple made once it is there.
///
/// Sent from a copy named by its bytes, so the folder sending it again after a restart finds
/// the upload already made, or under way, and waits on that one instead of making a second.
pub async fn add_from_folder(
    app: &AppHandle,
    path: &std::path::Path,
    album: Option<String>,
) -> Result<String, String> {
    use sha2::Digest;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("no name")?;
    // Read in pieces: a video can be many gigabytes, never held whole in memory.
    let (digest, len) = {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || -> std::io::Result<(String, u64)> {
            use std::io::Read;
            let mut f = std::fs::File::open(&path)?;
            let (mut hash, mut buf, mut len) = (sha2::Sha256::new(), vec![0u8; 1 << 20], 0u64);
            loop {
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
                len += n as u64;
            }
            Ok((format!("{:x}", hash.finalize()), len))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
    };
    let kept = crate::state_dir(app)
        .join("photos-folder-writes")
        .join("outgoing")
        .join(digest)
        .join(&name);
    let at = kept.to_string_lossy().into_owned();
    let data = app.state::<Data>();
    let known = data
        .0
        .lock()
        .await
        .upload_at(&at)
        .map_err(|e| e.to_string())?;
    let id = match known {
        Some(u) if u.state != "failed" && u.state != "cancelled" => u.id,
        _ => {
            if let Some(r) = quince_icloud::photos_upload::refusal(&name, len) {
                return Err(r.words());
            }
            if let Some(dir) = kept.parent() {
                tokio::fs::create_dir_all(dir)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tokio::fs::copy(path, &kept)
                .await
                .map_err(|e| e.to_string())?;
            let u = quince_core::photos::Upload {
                id: uuid::Uuid::new_v4().to_string(),
                batch: uuid::Uuid::new_v4().to_string(),
                path: at,
                name,
                state: "waiting".into(),
                error: None,
                asset: None,
                added_at: jiff::Timestamp::now().as_second(),
                album,
            };
            data.0
                .lock()
                .await
                .queue_uploads(std::slice::from_ref(&u))
                .map_err(|e| e.to_string())?;
            say(app, &u, "waiting", 0, len, None);
            u.id
        }
    };
    work(app.clone());
    // Waited on as the folder waits on any send: it retries a failure with its own backoff.
    let outcome = loop {
        let now = data.0.lock().await.upload(&id).map_err(|e| e.to_string())?;
        match now {
            Some(u) if u.state == "done" => {
                break u.asset.ok_or("Apple named no photo".to_string())
            }
            Some(u) if u.state == "failed" => {
                break Err(u.error.unwrap_or_else(|| "It could not be added.".into()))
            }
            Some(u) if u.state == "cancelled" => break Err("The upload was cancelled.".into()),
            None => break Err("The upload is no longer there.".into()),
            Some(_) => {
                // A session lapsed stops the queue; started again here, it carries on.
                work(app.clone());
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    };
    if let Some(dir) = kept.parent() {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
    outcome
}

/// Stop uploads, by id or a whole batch: what is waiting is not sent, and the file on its way
/// is abandoned mid-send. A photo Apple has already made stays (it is in the library, and can
/// be deleted there).
#[tauri::command]
pub async fn photos_upload_cancel(
    ids: Vec<String>,
    batch: Option<String>,
    app: AppHandle,
    data: State<'_, Data>,
) -> UiResult<()> {
    let wanted = |id: &str, b: &str| ids.iter().any(|i| i == id) || batch.as_deref() == Some(b);
    CANCELLED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .extend(ids.iter().cloned().chain(batch.clone()));
    for (id, b, handle) in CURRENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
    {
        if wanted(id, b) {
            handle.abort();
        }
    }
    let library = data.0.lock().await;
    for u in library.unfinished_uploads().map_err(|e| e.to_string())? {
        if wanted(&u.id, &u.batch) && u.state == "waiting" {
            library
                .set_upload(&u.id, "cancelled", None, None)
                .map_err(|e| e.to_string())?;
            say(&app, &u, "cancelled", 0, 0, None);
        }
    }
    Ok(())
}

/// The queue drained, one file at a time; started on demand and at launch to resume what a
/// quit left half done.
pub fn work(app: AppHandle) {
    // Photos sent before a quit, still being made: asked about again, never sent twice.
    confirm(app.clone());
    if WORKING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let next: Vec<_> = {
                let data = app.state::<Data>();
                let library = data.0.lock().await;
                library
                    .uploads_to_send()
                    .unwrap_or_default()
                    .into_iter()
                    .take(SENDERS)
                    .collect()
            };
            if next.is_empty() {
                break;
            }
            // A few at once; the queue pauses (the session lapsed) if any of them says so.
            let went = futures_util::future::join_all(next.into_iter().map(|u| one(&app, u))).await;
            if went.contains(&false) {
                break;
            }
        }
        WORKING.store(false, std::sync::atomic::Ordering::SeqCst);
    });
}

/// One upload, to its end. False when the queue must pause (the session lapsed).
async fn one(app: &AppHandle, u: quince_core::photos::Upload) -> bool {
    let set = |state: &str, error: Option<&str>, asset: Option<&str>| {
        let data = app.state::<Data>();
        let (id, state, error, asset) = (
            u.id.clone(),
            state.to_string(),
            error.map(str::to_string),
            asset.map(str::to_string),
        );
        async move {
            data.0
                .lock()
                .await
                .set_upload(&id, &state, error.as_deref(), asset.as_deref())
                .ok();
        }
    };
    if cancelled(&u) {
        set("cancelled", None, None).await;
        say(app, &u, "cancelled", 0, 0, None);
        return true;
    }
    // Never sent with a session that is not the library's own account's.
    if !crate::same_account(app).await {
        return false;
    }
    let auth = app.state::<Auth>();
    let (http, ck, params, upload_root) = {
        let client = auth.client.lock().await;
        let s = client.session();
        (
            client.http().clone(),
            s.service_url("ckdatabasews"),
            client.service_params(),
            s.service_url("photosupload"),
        )
    };
    // Not signed in yet (a launch resumes before the session is back): the queue waits, and
    // is started again by the next sync.
    let (Some(ck), Some(upload_root)) = (ck, upload_root) else {
        return false;
    };
    // Resumed after a quit: a photo Apple had already named is not made twice.
    match already_made(app, &http, &ck, &params, &u.id).await {
        Made::Yes(asset) => {
            set("done", None, Some(&asset)).await;
            into_album(app, &http, &ck, &params, &u, &asset).await;
            say(app, &u, "done", 1, 1, None);
            refresh_photos(app).await;
            return true;
        }
        // Not known yet: asked again later, never taken for "not uploaded" and sent twice.
        Made::Unknown => return false,
        Made::No => {}
    }
    let path = PathBuf::from(&u.path);
    let Ok(meta) = tokio::fs::metadata(&path).await else {
        set("failed", Some("That file is no longer there."), None).await;
        say(
            app,
            &u,
            "failed",
            0,
            0,
            Some("That file is no longer there.".into()),
        );
        return true;
    };
    let size = meta.len();
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
    let zone = jiff::tz::TimeZone::system();
    let zone_name = zone.iana_name().unwrap_or("UTC").to_string();
    let offset = -i64::from(jiff::Zoned::now().offset().seconds()) / 60;
    let dsid = params
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    set("sending", None, None).await;
    say(app, &u, "sending", 0, size, None);

    let mut attempt = 0;
    loop {
        attempt += 1;
        // Cancelled while waiting to try again.
        if attempt > 1 && cancelled(&u) {
            set("cancelled", None, None).await;
            say(app, &u, "cancelled", 0, size, None);
            return true;
        }
        let app2 = app.clone();
        let u2 = u.clone();
        let last = std::sync::Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
        let progress: std::sync::Arc<dyn Fn(quince_icloud::photos_upload::Stage) + Send + Sync> =
            std::sync::Arc::new(move |stage| {
                use quince_icloud::photos_upload::Stage;
                match stage {
                    Stage::Sending(sent, total) => {
                        // A few times a second at most: every piece would flood the window.
                        let mut at = last
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if sent == total || at.elapsed() > Duration::from_millis(120) {
                            *at = Instant::now();
                            say(&app2, &u2, "sending", sent, total, None);
                        }
                    }
                    Stage::Named(made) => {
                        let (app3, id) = (app2.clone(), u2.id.clone());
                        tauri::async_runtime::spawn(async move {
                            let data = app3.state::<Data>();
                            data.0
                                .lock()
                                .await
                                .set_upload(&id, "verifying", None, Some(&made.asset))
                                .ok();
                        });
                    }
                    Stage::Processing(done) => {
                        say(
                            &app2,
                            &u2,
                            "verifying",
                            (done.clamp(0.0, 100.0) as u64) * size / 100,
                            size,
                            None,
                        );
                    }
                }
            });
        let (handle, registration) = futures_util::future::AbortHandle::new_pair();
        CURRENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((u.id.clone(), u.batch.clone(), handle));
        let library =
            quince_icloud::photos::library_zone(&quince_icloud::cloudkit::ZoneClient::new(
                &http,
                &ck,
                quince_icloud::photos::CONTAINER,
                params.clone(),
            ))
            .await;
        let result = futures_util::future::Abortable::new(
            quince_icloud::photos_upload::send(
                &http,
                &upload_root,
                &dsid,
                &library,
                &path,
                &u.name,
                modified,
                &zone_name,
                offset,
                &u.batch,
                progress,
            ),
            registration,
        )
        .await;
        CURRENT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(id, ..)| *id != u.id);
        let Ok(result) = result else {
            set("cancelled", None, None).await;
            say(app, &u, "cancelled", 0, size, None);
            // Apple may already have made it; the library shows what is true.
            refresh_photos(app).await;
            return true;
        };
        use quince_icloud::photos_upload::{Refusal, Sending};
        match result {
            // Sent: Apple makes the photo while the next file goes; the checker (`confirm`)
            // asks about every photo being made at once, and finishes each as it is ready.
            Ok(Sending::Sent(sent)) => {
                {
                    let data = app.state::<Data>();
                    let library = data.0.lock().await;
                    library
                        .set_upload_job(&u.id, &sent.job, &sent.made.asset)
                        .ok();
                }
                say(app, &u, "verifying", 0, size, None);
                confirm(app.clone());
                return true;
            }
            // Already in the library: finished — never sent again — but not uploaded, which is
            // how icloud.com lists it ("Duplicate item.").
            Ok(Sending::Refused(Refusal::Duplicate)) => {
                let words = Refusal::Duplicate.words();
                set("done", Some(&words), None).await;
                say(app, &u, "failed", 0, size, Some(words));
                return true;
            }
            Ok(Sending::Refused(r)) => {
                set("failed", Some(&r.words()), None).await;
                say(app, &u, "failed", 0, size, Some(r.words()));
                return true;
            }
            Err(quince_icloud::Error::ReauthRequired) => {
                // Not a failure of this file: the queue waits for the session to come back.
                set("waiting", None, None).await;
                say(
                    app,
                    &u,
                    "waiting",
                    0,
                    size,
                    Some("Waiting for iCloud to sign back in.".into()),
                );
                return false;
            }
            Err(e) if attempt < 3 && !cancelled(&u) => {
                quiet!("photos upload: try {attempt} failed: {e}");
                tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
                // Apple may have made the photo before the try failed (waiting on its
                // processing, say): then it is done, and the file is never sent again.
                match already_made(app, &http, &ck, &params, &u.id).await {
                    Made::Yes(asset) => {
                        set("done", None, Some(&asset)).await;
                        into_album(app, &http, &ck, &params, &u, &asset).await;
                        say(app, &u, "done", size, size, None);
                        refresh_photos(app).await;
                        return true;
                    }
                    Made::Unknown => return false,
                    Made::No => {}
                }
            }
            Err(e) => {
                let words = e.to_string();
                set("failed", Some(&words), None).await;
                say(app, &u, "failed", 0, size, Some(words));
                return true;
            }
        }
    }
}

/// One checker at a time asks Apple about every photo being made.
static CONFIRMING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// A photo whose making hasn't moved for this long is one Apple won't finish.
const STALLED: Duration = Duration::from_secs(600);

/// Every upload sent whose photo Apple is still making, asked about together every two seconds
/// (one request for all of them, as icloud.com asks about an import), and each finished as it
/// is ready: put in its album, said done. Nothing waits behind a slow one; one that hasn't moved
/// in ten minutes is failed with why. Runs while any are being made, and again after a launch.
fn confirm(app: AppHandle) {
    if CONFIRMING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        use quince_icloud::photos_upload::JobState;
        let mut moved: HashMap<String, (f64, Instant)> = HashMap::new();
        // Ended for having none left (not for a sign-in it waits for): only then looked again.
        let mut emptied = false;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let being_made = {
                let data = app.state::<Data>();
                let library = data.0.lock().await;
                library.uploads_being_made().unwrap_or_default()
            };
            if being_made.is_empty() {
                emptied = true;
                break;
            }
            let auth = app.state::<Auth>();
            let (http, ck, params, upload_root) = {
                let client = auth.client.lock().await;
                let s = client.session();
                (
                    client.http().clone(),
                    s.service_url("ckdatabasews"),
                    client.service_params(),
                    s.service_url("photosupload"),
                )
            };
            let (Some(ck), Some(upload_root)) = (ck, upload_root) else {
                // Not signed in yet: the next sync starts the queue, and this, again.
                break;
            };
            let dsid = params
                .iter()
                .find(|(k, _)| k == "dsid")
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            let jobs: Vec<String> = being_made.iter().map(|(_, j)| j.clone()).collect();
            let states =
                match quince_icloud::photos_upload::job_states(&http, &upload_root, &dsid, &jobs)
                    .await
                {
                    Ok(states) => states,
                    Err(quince_icloud::Error::ReauthRequired) => break,
                    // Offline or Apple busy: asked again on the next round.
                    Err(_) => continue,
                };
            let mut finished = false;
            for (u, job) in being_made {
                let set = |state: &'static str, error: Option<String>| {
                    let (app, id, asset) = (app.clone(), u.id.clone(), u.asset.clone());
                    async move {
                        let data = app.state::<Data>();
                        data.0
                            .lock()
                            .await
                            .set_upload(&id, state, error.as_deref(), asset.as_deref())
                            .ok();
                    }
                };
                match states.get(&job).copied().unwrap_or(JobState::Working(0.0)) {
                    JobState::Done => {
                        set("done", None).await;
                        if let Some(asset) = &u.asset {
                            into_album(&app, &http, &ck, &params, &u, asset).await;
                        }
                        // Said with the file's real size: the panel shows it beside the photo.
                        let size = std::fs::metadata(&u.path).map_or(0, |m| m.len());
                        say(&app, &u, "done", size, size, None);
                        moved.remove(&u.id);
                        finished = true;
                    }
                    JobState::Failed(code) => {
                        let words =
                            format!("iCloud couldn\u{2019}t make that photo (error {code}).");
                        set("failed", Some(words.clone())).await;
                        say(&app, &u, "failed", 0, 0, Some(words));
                        moved.remove(&u.id);
                    }
                    JobState::Working(done) => {
                        let now = Instant::now();
                        let (last, since) = *moved.entry(u.id.clone()).or_insert((done, now));
                        if done > last {
                            moved.insert(u.id.clone(), (done, now));
                        } else if now.duration_since(since) > STALLED {
                            let words = "iCloud stopped making that photo.".to_string();
                            set("failed", Some(words.clone())).await;
                            say(&app, &u, "failed", 0, 0, Some(words));
                            moved.remove(&u.id);
                            continue;
                        }
                        say(
                            &app,
                            &u,
                            "verifying",
                            done.clamp(0.0, 100.0) as u64,
                            100,
                            None,
                        );
                    }
                }
            }
            // The library read once for every photo finished this round, never once each.
            if finished {
                refresh_photos(&app).await;
            }
        }
        CONFIRMING.store(false, std::sync::atomic::Ordering::SeqCst);
        // A photo sent just as this ended would wait for the next sync: looked for once more.
        let more = {
            let data = app.state::<Data>();
            let library = data.0.lock().await;
            library.uploads_being_made().is_ok_and(|m| !m.is_empty())
        };
        if emptied && more {
            confirm(app);
        }
    });
}

/// Whether Apple has already made the photo an upload was sending.
enum Made {
    /// Yes: the photo's record, by name.
    Yes(String),
    /// No photo was named for it, or Apple says there is none.
    No,
    /// Apple could not be asked just now.
    Unknown,
}

/// Asked of Apple by the name it gave the photo when the upload was first sent (kept with the
/// upload as soon as Apple names it), never assumed: a failed check read as "not made" sent the
/// whole file again, and Apple kept a second copy.
async fn already_made(
    app: &AppHandle,
    http: &reqwest::Client,
    ck: &str,
    params: &[(String, String)],
    id: &str,
) -> Made {
    let asset = {
        let data = app.state::<Data>();
        let library = data.0.lock().await;
        library.upload(id).ok().flatten().and_then(|u| u.asset)
    };
    let Some(asset) = asset else {
        return Made::No;
    };
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        http,
        ck,
        quince_icloud::photos::CONTAINER,
        params.to_vec(),
    );
    let zone = quince_icloud::photos::library_zone(&zc).await;
    match zc.lookup_record(&zone, &asset).await {
        Ok(Some(_)) => Made::Yes(asset),
        Ok(None) => Made::No,
        Err(_) => Made::Unknown,
    }
}

/// A photo added while an album was open goes into that album too, after what is there, as
/// Photos files an import. The upload itself has already succeeded: if the album is gone, or
/// Apple refuses, the photo stays in the library and only this step is skipped.
async fn into_album(
    app: &AppHandle,
    http: &reqwest::Client,
    ck: &str,
    params: &[(String, String)],
    u: &quince_core::photos::Upload,
    asset: &str,
) {
    let Some(album) = u.album.as_deref() else {
        return;
    };
    let _one = crate::PHOTOS_PASS.lock().await;
    let data = app.state::<Data>();
    let last = {
        let library = data.0.lock().await;
        if !library
            .albums()
            .map(|a| a.iter().any(|x| x.id == album))
            .unwrap_or(false)
        {
            return;
        }
        library.album_last_position(album).ok().flatten()
    };
    match quince_icloud::photos::add_to_album(
        http,
        ck,
        params.to_vec(),
        album,
        &[asset.to_string()],
        last,
    )
    .await
    {
        (placed, Ok(())) => {
            data.0
                .lock()
                .await
                .put_album_items(
                    &placed
                        .into_iter()
                        .map(|(a, at)| (album.to_string(), a, at))
                        .collect::<Vec<_>>(),
                )
                .ok();
        }
        (_, Err(e)) => quiet!("photos upload: not put in its album: {e}"),
    }
}

async fn refresh_photos(app: &AppHandle) {
    use tauri::Emitter;
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    crate::refresh::some(&auth.client, &data.0, &["photos"]).await;
    // What it stored was told as it landed (refresh::on_landed); this brings the freshness.
    app.emit(
        crate::SYNCED,
        crate::refresh::status_changed(&data.0, &[]).await,
    )
    .ok();
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn links_past_their_life_give_their_room_back() {
        let mut map: HashMap<String, ((), Instant)> = HashMap::new();
        let Some(old) = Instant::now().checked_sub(LINK_LIFE * 2) else {
            return;
        };
        for i in 0..100_000 {
            map.insert(format!("master-{i}"), ((), old));
        }
        map.insert("fresh".into(), ((), Instant::now()));
        let before = map.capacity();
        keep_fresh(&mut map);
        assert_eq!(map.len(), 1);
        assert!(
            map.capacity() < before / 100,
            "{} of {before}",
            map.capacity()
        );
    }
}

#[cfg(test)]
mod tests {

    /// Hidden's photos are handed out only while it is unlocked, and locking it (the window
    /// leaving it, or the account changing) takes that back; nothing else is held back.
    #[test]
    fn hidden_is_handed_out_only_while_unlocked() {
        use quince_core::photos::Source;
        super::lock_hidden();
        assert!(super::hidden_allowed(&Source::View(View::Hidden)).is_err());
        assert!(super::hidden_allowed(&Source::View(View::Library)).is_ok());
        assert!(super::hidden_allowed(&Source::Shared("SC".into())).is_ok());
        super::HIDDEN_OPEN.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(super::hidden_allowed(&Source::View(View::Hidden)).is_ok());
        super::forget_links();
        assert!(
            super::hidden_allowed(&Source::View(View::Hidden)).is_err(),
            "an account change locks it"
        );
    }

    /// Three tries free, then a wait that doubles and stops at ten minutes.
    #[test]
    fn wrong_passwords_wait_longer_each_time() {
        let secs: Vec<u64> = (0..12).map(|n| super::hidden_wait(n).as_secs()).collect();
        assert_eq!(secs, [0, 0, 0, 30, 60, 120, 240, 480, 600, 600, 600, 600]);
    }
    use super::*;

    /// Copy on Linux answers a paste only with the PNG, and only when a picture was asked
    /// for: an app asking for text (a terminal, a text field) gets nothing, never the
    /// picture's bytes as characters.
    #[test]
    fn copy_answers_only_a_paste_that_asks_for_a_picture() {
        let png = b"\x89PNG\r\n\x1a\nrest";
        assert_eq!(answer_for("image/png", png), Some(&png[..]));
        for text in [
            "text/plain",
            "text/plain;charset=utf-8",
            "UTF8_STRING",
            "STRING",
            "TEXT",
            "text/uri-list",
            "text/html",
        ] {
            assert_eq!(answer_for(text, png), None, "{text}");
        }
    }

    /// A record id becomes one name in the cache's folder, whatever it holds.
    #[test]
    fn a_record_id_is_one_name_in_its_folder() {
        assert_eq!(cache_name("AbC-12_x"), "AbC-12_x");
        let dir = std::path::Path::new("/data/photos/original");
        for id in [
            "../../cookies.json",
            "..",
            "a/b",
            "a\\b",
            "C:x",
            "",
            "x\0y",
            "AW+/=",
        ] {
            let name = cache_name(id);
            assert!(name.starts_with('~'), "{id:?}");
            assert!(name[1..].chars().all(|c| c.is_ascii_hexdigit()), "{id:?}");
            assert_eq!(dir.join(&*name).parent(), Some(dir), "{id:?}");
        }
        assert_ne!(cache_name("a/b"), cache_name("a\\b"));
    }

    #[test]
    fn ranges_as_a_player_asks_for_them() {
        let len = 10 * 1024 * 1024;
        let all = u64::MAX;
        assert_eq!(byte_range(None, len, all), Some((0, len - 1)));
        assert_eq!(byte_range(Some("bytes=0-"), len, all), Some((0, len - 1)));
        assert_eq!(byte_range(Some("bytes=0-"), len, 4096), Some((0, 4095)));
        assert_eq!(byte_range(Some("bytes=0-1"), len, all), Some((0, 1)));
        assert_eq!(
            byte_range(Some("bytes=100-199"), len, all),
            Some((100, 199))
        );
        assert_eq!(
            byte_range(Some("bytes=-500"), len, all),
            Some((len - 500, len - 1))
        );
        assert_eq!(
            byte_range(Some("bytes=9000000-"), len, all),
            Some((9_000_000, len - 1))
        );
        assert_eq!(
            byte_range(Some("bytes=5-999999999"), 100, all),
            Some((5, 99))
        );
        assert_eq!(byte_range(Some("bytes=100-"), 100, all), None);
        assert_eq!(byte_range(Some("bytes=9-3"), 100, all), None);
        assert_eq!(byte_range(Some("bytes=-0"), 100, all), None);
        assert_eq!(byte_range(Some("items=0-1"), 100, all), None);
        assert_eq!(byte_range(None, 0, all), None);
    }

    /// Pictures are made ahead for every photo a grid can show — Hidden and Recently Deleted
    /// too, not the library alone — each once, the view being looked at first; Hidden's (and a
    /// hidden photo's in Recently Deleted) only while Hidden is unlocked.
    #[test]
    fn hidden_and_deleted_are_made_ahead_too_the_view_shown_first() {
        let photo = |id: &str, t: i64, hidden: bool, deleted: bool| -> Photo {
            serde_json::from_value(serde_json::json!({
                "id": id, "master": format!("m{id}"), "takenAt": t, "kind": "image", "width": 4032,
                "height": 3024, "favorite": false, "hidden": hidden, "deleted": deleted, "subtype": 0
            }))
            .unwrap()
        };
        let mut lib = quince_core::Library::in_memory().unwrap();
        lib.put_photos(&[
            photo("L1", 30, false, false),
            photo("H1", 20, true, false),
            photo("H2", 10, true, false),
            photo("D1", 5, false, true),
            photo("D2", 4, true, true),
        ])
        .unwrap();
        // Read a page at a time — one photo at a time here — as `prepare` reads them.
        let shapes = |first, open| -> Vec<String> {
            let mut out = Vec::new();
            let mut after = None;
            loop {
                let (page, next) = lib
                    .grid_shapes_shown(first, after.as_ref(), 1, open)
                    .unwrap();
                out.extend(page.into_iter().map(|p| p.id));
                match next {
                    Some(n) => after = Some(n),
                    None => return out,
                }
            }
        };
        let ids = |first| shapes(first, true);
        assert_eq!(ids(None), ["L1", "H1", "H2", "D1", "D2"]);
        assert_eq!(ids(Some(View::Hidden)), ["H1", "H2", "L1", "D1", "D2"]);
        assert_eq!(ids(Some(View::Library)), ["L1", "H1", "H2", "D1", "D2"]);
        // A view inside the library (Favorites) adds nothing the library does not have.
        assert_eq!(ids(Some(View::Favorites)), ["L1", "H1", "H2", "D1", "D2"]);
        // Locked: nothing hidden is made into a picture on disk, whichever view is shown.
        for first in [None, Some(View::Hidden), Some(View::RecentlyDeleted)] {
            let locked = shapes(first, false);
            assert!(
                locked.iter().all(|id| id == "L1" || id == "D1"),
                "{locked:?}"
            );
            assert_eq!(locked.len(), 2);
        }
    }

    /// The steps made ahead are the ones the window asks for (fitSrc, tileSrc, viewSrc).
    #[test]
    fn steps_are_the_window_s() {
        let p: Photo = serde_json::from_value(serde_json::json!({
            "id": "a", "master": "m", "takenAt": 0, "kind": "image", "width": 4032, "height": 3024,
            "favorite": false, "hidden": false, "deleted": false, "subtype": 0
        }))
        .unwrap();
        let shown = Shown {
            cell: 180.0,
            fit: false,
            stage_w: 1300.0,
            stage_h: 800.0,
            dpr: 1.0,
            view: None,
        };
        // A square crop of a 4:3 photo reaches 240 along its long side; the stage draws it
        // 1066 wide (800 high × 4/3).
        assert_eq!(steps_of(p.width, p.height, shown), (256, 1536));
        assert_eq!(
            steps_of(p.width, p.height, Shown { fit: true, ..shown }).0,
            256
        );
        assert_eq!(step_for(5000.0), 3840);
        assert_eq!(step_for(100.0), 128);
    }

    #[test]
    fn a_fitted_picture_is_asked_for_by_its_step_only() {
        assert_eq!(
            requested_fit("/pic/A1-b/fit1024"),
            Some(("A1-b".into(), 1024, false))
        );
        assert_eq!(
            requested_fit("pic%2FA1%2Ffit256"),
            Some(("A1".into(), 256, false))
        );
        // Not a step (the cache would grow without end), not a picture, not a path.
        assert_eq!(requested_fit("/pic/A1/fit1000"), None);
        assert_eq!(requested_fit("/pic/A1/full"), None);
        assert_eq!(requested_fit("/pic/../x/fit256"), None);
        assert_eq!(requested_fit("/pic/A1/fitx"), None);
    }

    /// A picture's address marked with what it is made from is the same photo's picture, and
    /// may be kept for good; the unmarked one still works, kept for a day as before. A mark is
    /// a few letters and digits, never a way out of the cache.
    #[test]
    fn a_marked_picture_is_the_same_photo_kept_for_good() {
        assert_eq!(
            requested_fit("pic%2FA1-b~1x9z%2Ffit256"),
            Some(("A1-b".into(), 256, true))
        );
        assert_eq!(
            requested_fit("/pic/A1~k3/fit1024"),
            Some(("A1".into(), 1024, true))
        );
        assert_eq!(requested_fit("/pic/A1~/fit256"), None);
        assert_eq!(requested_fit("/pic/A1~X/fit256"), None);
        assert_eq!(requested_fit("/pic/A1~a~b/fit256"), None);
        assert_eq!(requested_fit("/pic/~ab/fit256"), None);
        assert_eq!(requested_fit("/pic/..~ab/fit256"), None);
        assert_eq!(requested_fit("/pic/A1~0123456789abcdefg/fit256"), None);
        // Every other picture of the photo takes the mark too, and the old address still works.
        assert_eq!(
            requested("pic%2FA1-b~1x9z%2Fthumb"),
            Some(("A1-b".into(), Size::Thumb))
        );
        assert_eq!(
            requested("/pic/A1~k3/video"),
            Some(("A1".into(), Size::Video))
        );
        assert_eq!(
            requested("/pic/A1/original"),
            Some(("A1".into(), Size::Original))
        );
        assert_eq!(requested("/pic/A1~X/full"), None);
        assert_eq!(requested("/pic/~ab/full"), None);
        assert_eq!(requested("/pic/..~ab/full"), None);
        assert!(fit_cache(true).contains("immutable"));
        assert!(!fit_cache(false).contains("immutable"));
    }

    #[test]
    fn quicktime_or_mpeg4() {
        assert_eq!(video_type(b"\0\0\0\x14ftypqt  "), "video/quicktime");
        assert_eq!(video_type(b"\0\0\0\x18ftypmp42"), "video/mp4");
        assert_eq!(video_type(b""), "video/mp4");
    }
}
