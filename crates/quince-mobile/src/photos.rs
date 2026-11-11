//! iCloud Photos on the phone: the library read from Apple a few pages at a time into the phone's
//! own library (`quince_sync::photos`, the desktop's own steps), a grid read from it as one
//! compact index (ids, dates, marks: what a list of a million needs), and each picture fetched
//! once, at the size shown, into the app's cache, from the link Apple gave (asked again when old).

use crate::{failed, runtime, Account, EngineError};
use quince_core::photos::{Kind, PhotoLibrary, Source, View};
use quince_icloud::photos::Links;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long Apple's download link is trusted before it is asked for again (as the desktop's).
const LINK_LIFE: Duration = Duration::from_secs(45 * 60);

/// Apple's download links by master id, with when they came.
static LINKS: Mutex<Option<HashMap<String, (Links, Instant)>>> = Mutex::new(None);

pub(crate) fn remember(links: HashMap<String, Links>) {
    let mut held = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = held.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    map.retain(|_, (_, at)| at.elapsed() < LINK_LIFE);
    for (master, l) in links {
        map.insert(master, (l, now));
    }
}

fn fresh(master: &str) -> Option<Links> {
    let held = LINKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    held.as_ref()?
        .get(master)
        .filter(|(_, at)| at.elapsed() < LINK_LIFE)
        .map(|(l, _)| l.clone())
}

/// Which photos a grid shows.
#[derive(uniffi::Enum)]
pub enum PView {
    Library,
    Favorites,
    Videos,
    Screenshots,
    Recents,
    RecentlyDeleted,
    /// Shown only after the person proves it is them (fingerprint, face or screen lock).
    Hidden,
    Panoramas,
    Portrait,
    Album {
        id: String,
    },
    /// A shared album, by its zone: its photos are other people's too, and only looked at.
    Shared {
        zone: String,
    },
}

/// A shared album, as the collections list shows it.
#[derive(uniffi::Record)]
pub struct PSharedAlbum {
    pub zone: String,
    pub title: Option<String>,
    pub count: u32,
    /// Its newest photo, for the album's picture.
    pub cover: Option<String>,
    /// Made by this account, rather than joined.
    pub owned: bool,
}

/// Which photos the library's views show, as Photos' library switcher chooses.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum PLibrary {
    Both,
    Personal,
    Shared,
}

/// The library shown, and whether there is a Shared Library to choose at all.
#[derive(uniffi::Record)]
pub struct PLibraryChoice {
    pub showing: PLibrary,
    pub has_shared: bool,
}

/// A flag set on photos, as Photos sets them.
#[derive(uniffi::Enum)]
pub enum PFlag {
    Favorite,
    Hidden,
    /// Into Recently Deleted; off, recovered from it.
    Deleted,
    /// Deleted for good, from Recently Deleted only. It cannot be undone.
    Expunged,
}

/// The photos pass: a pass from Apple and a flag change being sent never run at once, so no
/// pass is stored between Apple taking a change and the change coming off the queue.
static PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A grid's photos as one compact index: ids in order, each one's date (unix seconds), and its
/// marks packed (bits 0–1 the kind: 0 image, 1 video, 2 Live; bit 2 a favorite).
#[derive(uniffi::Record)]
pub struct PIndex {
    pub ids: Vec<String>,
    pub taken: Vec<i64>,
    pub marks: Vec<u8>,
}

/// One photo's details, for the viewer.
#[derive(uniffi::Record)]
pub struct PItem {
    pub id: String,
    pub taken_at: i64,
    pub width: i64,
    pub height: i64,
    pub video: bool,
    pub live: bool,
    pub duration: Option<f64>,
    pub filename: Option<String>,
    pub size: Option<i64>,
    pub favorite: bool,
    /// In a shared album: looked at and saved only, never changed from here (Apple's web
    /// service lets an album's photos be read, not marked).
    pub shared: bool,
}

/// An album.
#[derive(uniffi::Record)]
pub struct PAlbum {
    pub id: String,
    pub name: String,
    pub folder: bool,
    pub parent: Option<String>,
}

/// Which picture of a photo: the grid's, the viewer's, the whole one for zooming, or its film.
#[derive(uniffi::Enum)]
pub enum PSize {
    Thumb,
    Medium,
    Full,
    Video,
}

/// How a photo backed up from the phone went.
#[derive(uniffi::Enum)]
pub enum PBackup {
    /// In iCloud Photos now (or found there already from an earlier try).
    Sent,
    /// iCloud already has this photo: nothing sent.
    Duplicate,
    /// Not taken, for good: the reason, for the person (a type Photos takes none of, too large…).
    Refused { reason: String },
}

/// What a pass from Apple came to.
#[derive(uniffi::Record)]
pub struct PSync {
    /// Something the screens show changed: the library, a shared album, the Shared Library.
    pub changed: bool,
    /// Apple has more: the next pass carries on (a large library is read over several).
    pub more: bool,
    /// Apple is still preparing the library: nothing read yet.
    pub preparing: bool,
    /// Advanced Data Protection holds Photos until the person approves on one of their
    /// devices: nothing was asked of Apple, and nothing is retried before then.
    pub paused: bool,
    pub photos: u32,
    /// Flag changes made here still on their way (Apple out of reach): sent on a later sync.
    pub waiting: u32,
    /// Why Apple turned down each change this sync sent and did not take: those photos are
    /// back as they were.
    pub refused: Vec<String>,
}

fn source(v: PView) -> Source {
    match v {
        PView::Library => Source::View(View::Library),
        PView::Favorites => Source::View(View::Favorites),
        PView::Videos => Source::View(View::Videos),
        PView::Screenshots => Source::View(View::Screenshots),
        PView::Recents => Source::View(View::Recents),
        PView::RecentlyDeleted => Source::View(View::RecentlyDeleted),
        PView::Hidden => Source::View(View::Hidden),
        PView::Panoramas => Source::View(View::Panoramas),
        PView::Portrait => Source::View(View::Portrait),
        PView::Album { id } => Source::Album(id),
        PView::Shared { zone } => Source::Shared(zone),
    }
}

/// Where Apple keeps a photo, to ask it for the photo's pictures: the personal library, a
/// Shared Library's zone, or a shared album.
enum Place {
    Library(Option<quince_icloud::shared_collections::Place>),
    Album(quince_icloud::shared_collections::Place),
}

fn album_place(s: quince_core::shared::SharedPlace) -> quince_icloud::shared_collections::Place {
    quince_icloud::shared_collections::Place {
        database: if s.database == "shared" {
            quince_icloud::cloudkit::Database::Shared
        } else {
            quince_icloud::cloudkit::Database::Private
        },
        zone: s.zone,
        owner: s.owner,
    }
}

/// A photo to look at — the library's own or a shared album's — and where Apple keeps it.
fn any_photo(
    l: &quince_core::Library,
    id: &str,
) -> Result<Option<(quince_core::photos::Photo, Place)>, String> {
    if let Some(p) = l.photo(id).map_err(|e| e.to_string())? {
        let place = quince_sync::photos::library_place(l, id)?;
        return Ok(Some((p, Place::Library(place))));
    }
    Ok(l.shared_photo(id)
        .map_err(|e| e.to_string())?
        .map(|(p, s)| (p, Place::Album(album_place(s)))))
}

/// What a call to Apple's Photos needs: the client, CloudKit's address and the session's parameters.
type Context = (reqwest::Client, String, Vec<(String, String)>);

impl Account {
    fn photos_context(&self) -> Result<Context, EngineError> {
        let c = self.client();
        let root = c
            .session()
            .service_url("ckdatabasews")
            .ok_or_else(|| failed("Apple hasn't offered Photos for this account."))?;
        Ok((c.http().clone(), root, c.service_params()))
    }
}

#[uniffi::export]
impl Account {
    /// The flag changes made here sent to Apple, oldest first, then a pass from Apple's library
    /// into the phone's, laid under whatever is still on its way, as the desktop does both.
    pub fn photos_sync(&self) -> Result<PSync, EngineError> {
        use quince_sync::photos::{self as shared, PHOTOS};
        // The session and whose library it is, read in one look; then the desktop's pass:
        // readiness, a few pages a round, a whole walk once a day, then Shared Albums and the
        // Shared Library (quince_sync::pass::photos).
        let apple = self.apple("photos")?;
        let lock = self.library_of(&apple)?;
        // Sent first: Android may stop the app at any point, and what is queued waits safely in
        // the library until a pass gets this far. Held while Advanced Data Protection waits for
        // an approval, as on the desktop: Apple would refuse it, and nothing is retried on a timer.
        let refused = match apple.cloudkit.as_deref() {
            Some(root) if !apple.waiting.contains(&"photos") => {
                runtime()
                    .block_on(quince_sync::queue::drain_as(
                        &lock,
                        PHOTOS,
                        &PASS,
                        &apple.account,
                        |p| shared::send(&lock, &apple.http, root, apple.params.clone(), p),
                    ))
                    .refused
            }
            _ => Vec::new(),
        };
        let pass = runtime().block_on(self.pass(quince_sync::pass::PHOTOS, &apple, &lock, &PASS));
        let paused = crate::pass::settled(&pass, "Apple hasn't offered Photos for this account.")?;
        let waiting = runtime()
            .block_on(async { lock.lock().await.outbox_pending(PHOTOS) })
            .map_err(failed)?
            .len();
        let photos = runtime()
            .block_on(async {
                lock.lock()
                    .await
                    .photo_index(&Source::View(View::Library), None)
            })
            .map_err(failed)?
            .ids
            .len();
        Ok(PSync {
            changed: pass.changed,
            more: pass.more,
            preparing: pass.outcome == quince_sync::pass::Outcome::Preparing,
            paused,
            photos: u32::try_from(photos).unwrap_or(u32::MAX),
            waiting: u32::try_from(waiting).unwrap_or(u32::MAX),
            refused: refused.into_iter().map(|(_, why)| why).collect(),
        })
    }

    /// Whether a backup under `key` has already reached iCloud Photos (or found its photo there),
    /// from the phone's library alone: a photo edited on the phone is seen again by the backup,
    /// and this answers it without asking Apple.
    pub fn photo_backup_done(&self, key: String) -> bool {
        let Ok((lock, _)) = self.library() else {
            return false;
        };
        runtime()
            .block_on(async { lock.lock().await.upload(&key) })
            .ok()
            .flatten()
            .is_some_and(|u| u.state == "done")
    }

    /// Whether iCloud Photos takes a file of this name and size (Send to iPhone sends it to
    /// Photos when it does, to iCloud Drive when it doesn't, as the desktop decides).
    pub fn photo_accepts(&self, name: String, size: u64) -> bool {
        quince_icloud::photos_upload::refusal(&name, size).is_none()
    }

    /// Favorite, Hide, Delete, Recover or (from Recently Deleted) delete for good, on these
    /// photos: recorded and shown at once, and sent behind by the next Photos sync (which sends
    /// what waits before it reads), offline or not; the desktop's exact code. Answers how many
    /// photos are now so (none for a delete for good: gone from view); an error says why
    /// nothing was recorded.
    pub fn photos_set_flag(
        &self,
        ids: Vec<String>,
        flag: PFlag,
        on: bool,
    ) -> Result<u32, EngineError> {
        let (lock, _) = self.library()?;
        let flag = match flag {
            PFlag::Favorite => quince_icloud::photos::Flag::Favorite,
            PFlag::Hidden => quince_icloud::photos::Flag::Hidden,
            PFlag::Deleted => quince_icloud::photos::Flag::Deleted,
            PFlag::Expunged => quince_icloud::photos::Flag::Expunged,
        };
        let changed = runtime()
            .block_on(async {
                let mut l = lock.lock().await;
                quince_sync::photos::record_flags(&mut l, &ids, flag, on)
            })
            .map_err(failed)?;
        Ok(u32::try_from(changed.len()).unwrap_or(u32::MAX))
    }

    /// A grid's photos, newest first, as one compact index.
    pub fn photo_index(&self, view: PView) -> Result<PIndex, EngineError> {
        let (lock, _) = self.library()?;
        let index = runtime()
            .block_on(async { lock.lock().await.photo_index(&source(view), None) })
            .map_err(failed)?;
        Ok(PIndex {
            ids: index.ids,
            taken: index.keys,
            marks: index.marks,
        })
    }

    /// One photo's details.
    pub fn photo_item(&self, id: String) -> Result<Option<PItem>, EngineError> {
        let (lock, _) = self.library()?;
        let found = runtime()
            .block_on(async { any_photo(&*lock.lock().await, &id) })
            .map_err(failed)?;
        Ok(found.map(|(p, place)| PItem {
            shared: matches!(place, Place::Album(_)),
            video: p.kind == Kind::Video,
            live: p.kind == Kind::Live,
            id: p.id,
            taken_at: p.taken_at,
            width: p.width,
            height: p.height,
            duration: p.duration,
            filename: p.filename,
            size: p.size,
            favorite: p.favorite,
        }))
    }

    /// The albums, in Apple's order.
    pub fn photo_albums(&self) -> Result<Vec<PAlbum>, EngineError> {
        let (lock, _) = self.library()?;
        Ok(runtime()
            .block_on(async { lock.lock().await.albums() })
            .map_err(failed)?
            .into_iter()
            .map(|a| PAlbum {
                id: a.id,
                name: a.name,
                folder: a.folder,
                parent: a.parent,
            })
            .collect())
    }

    /// Every shared album this account made or was invited to, as the last sync found them,
    /// each with its photo count and cover (the desktop's `photos_shared_albums`).
    pub fn photo_shared_albums(&self) -> Result<Vec<PSharedAlbum>, EngineError> {
        let (lock, _) = self.library()?;
        Ok(runtime()
            .block_on(async { lock.lock().await.shared_albums() })
            .map_err(failed)?
            .into_iter()
            .map(|a| PSharedAlbum {
                owned: a.owned(),
                zone: a.zone,
                title: a.title,
                count: u32::try_from(a.count).unwrap_or(0),
                cover: a.cover,
            })
            .collect())
    }

    /// Which library the views show, and whether there is a Shared Library to choose: the
    /// switcher shows only when there is (the desktop's `photos_library`).
    pub fn photo_library(&self) -> Result<PLibraryChoice, EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async {
            let l = lock.lock().await;
            Ok(PLibraryChoice {
                showing: match l.photo_library().map_err(failed)? {
                    PhotoLibrary::Both => PLibrary::Both,
                    PhotoLibrary::Personal => PLibrary::Personal,
                    PhotoLibrary::Shared => PLibrary::Shared,
                },
                has_shared: !l.shared_libraries().map_err(failed)?.is_empty(),
            })
        })
    }

    /// The library the views show from now on: kept in the library, as on the desktop.
    pub fn photo_set_library(&self, which: PLibrary) -> Result<(), EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async {
            lock.lock()
                .await
                .set_photo_library(match which {
                    PLibrary::Both => PhotoLibrary::Both,
                    PLibrary::Personal => PhotoLibrary::Personal,
                    PLibrary::Shared => PhotoLibrary::Shared,
                })
                .map_err(failed)
        })
    }

    /// A photo or film from the phone (`key`: the phone's own id for it) backed up to iCloud
    /// Photos, as the desktop uploads one (`photos_upload::send`). Never twice: as soon as Apple
    /// names the photo it is making, the name is kept in the phone's library under `key`, and a
    /// later try (after the app was stopped mid-way) asks Apple for that photo before sending
    /// again; Apple itself also answers a photo it already has as a duplicate.
    pub fn photo_backup(
        &self,
        key: String,
        path: String,
        name: String,
    ) -> Result<PBackup, EngineError> {
        use quince_icloud::photos_upload::{self as up, Refusal, Sending, Stage};
        let (lock, _) = self.library()?;
        let (http, ck, params, upload_root) = {
            let c = self.client();
            let s = c.session();
            (
                c.http().clone(),
                s.service_url("ckdatabasews")
                    .ok_or_else(|| failed("Apple hasn't offered Photos for this account."))?,
                c.service_params(),
                s.service_url("photosupload").ok_or_else(|| {
                    failed("Apple hasn't offered photo uploads for this account.")
                })?,
            )
        };
        let dsid = params
            .iter()
            .find(|(k, _)| k == "dsid")
            .map(|(_, v)| v.clone())
            .ok_or_else(|| failed("Sign in to back up photos."))?;
        runtime().block_on(async {
            let zc = quince_icloud::cloudkit::ZoneClient::new(
                &http,
                &ck,
                quince_icloud::photos::CONTAINER,
                params.clone(),
            );
            let zone = quince_icloud::photos::library_zone(&zc).await;
            // Named by Apple on an earlier try: there already, or sent again.
            let known = lock.lock().await.upload(&key).map_err(failed)?;
            if let Some(asset) = known.as_ref().and_then(|u| u.asset.clone()) {
                match zc.lookup_record(&zone, &asset).await {
                    Ok(Some(_)) => {
                        lock.lock()
                            .await
                            .set_upload(&key, "done", None, None)
                            .map_err(failed)?;
                        return Ok(PBackup::Sent);
                    }
                    Ok(None) => {}
                    // Can't tell now: not sent, so never twice; the next sync asks again.
                    Err(e) => return Err(failed(e)),
                }
            }
            if known.is_none() {
                lock.lock()
                    .await
                    .queue_uploads(&[quince_core::photos::Upload {
                        id: key.clone(),
                        batch: "phone-backup".into(),
                        path: path.clone(),
                        name: name.clone(),
                        state: "sending".into(),
                        error: None,
                        asset: None,
                        added_at: jiff::Timestamp::now().as_second(),
                        album: None,
                    }])
                    .map_err(failed)?;
            }
            let file = std::path::Path::new(&path);
            let modified = std::fs::metadata(file)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0));
            let zone_name = jiff::tz::TimeZone::system()
                .iana_name()
                .unwrap_or("UTC")
                .to_string();
            let offset = -i64::from(jiff::Zoned::now().offset().seconds()) / 60;
            // Apple's name for the photo kept the moment it gives one, before the rest is done;
            // the final state written only after that, so it is never overwritten by it.
            let named: std::sync::Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>> =
                std::sync::Arc::default();
            let (keep, key2, held) = (
                std::sync::Arc::clone(&lock),
                key.clone(),
                std::sync::Arc::clone(&named),
            );
            let progress: std::sync::Arc<dyn Fn(Stage) + Send + Sync> =
                std::sync::Arc::new(move |stage| {
                    if let Stage::Named(made) = stage {
                        let (keep, key) = (std::sync::Arc::clone(&keep), key2.clone());
                        let write = runtime().spawn(async move {
                            let _ = keep.lock().await.set_upload(
                                &key,
                                "verifying",
                                None,
                                Some(&made.asset),
                            );
                        });
                        *held
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(write);
                    }
                });
            let sent = up::send(
                &http,
                &upload_root,
                &dsid,
                &zone,
                file,
                &name,
                modified,
                &zone_name,
                offset,
                "phone-backup",
                progress,
            )
            .await;
            let pending = named
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(write) = pending {
                let _ = write.await;
            }
            let sent = sent.map_err(failed)?;
            let l = lock.lock().await;
            Ok(match sent {
                Sending::Sent(s) => {
                    l.set_upload(&key, "done", None, Some(&s.made.asset))
                        .map_err(failed)?;
                    PBackup::Sent
                }
                Sending::Refused(Refusal::Duplicate) => {
                    l.set_upload(&key, "done", None, None).map_err(failed)?;
                    PBackup::Duplicate
                }
                Sending::Refused(r) => {
                    let reason = match r {
                        Refusal::Kind(k) => format!("iCloud Photos doesn't take {k} files."),
                        Refusal::TooSmall => "This file is too small to be a photo.".into(),
                        Refusal::TooLarge => "This file is too large for iCloud Photos.".into(),
                        Refusal::NameTooLong => {
                            "This file's name is too long for iCloud Photos.".into()
                        }
                        Refusal::OverQuota => "Your iCloud storage is full.".into(),
                        Refusal::Duplicate => unreachable!(),
                    };
                    l.set_upload(&key, "refused", Some(&reason), None)
                        .map_err(failed)?;
                    PBackup::Refused { reason }
                }
            })
        })
    }

    /// A photo's picture at `size` fetched to `path` (the app's cache), once: a path that has
    /// it already is left as it is. False when Apple has no picture of that size.
    pub fn photo_file(&self, id: String, size: PSize, path: String) -> Result<bool, EngineError> {
        let target = std::path::PathBuf::from(&path);
        if target.exists() {
            return Ok(true);
        }
        let (lock, _) = self.library()?;
        let (photo, place) = runtime()
            .block_on(async { any_photo(&*lock.lock().await, &id) })
            .map_err(failed)?
            .ok_or_else(|| failed("That photo is no longer in the library."))?;
        let (http, root, params) = self.photos_context()?;
        let pick = |l: &Links| match size {
            PSize::Thumb => l.thumb.clone().or_else(|| l.medium.clone()),
            PSize::Medium => l.medium.clone().or_else(|| l.full.clone()),
            PSize::Full => l.full.clone().or_else(|| l.medium.clone()),
            PSize::Video => l.video.clone(),
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
        let part = target.with_extension("part");
        runtime().block_on(async {
            // A link kept from the pass first; a fresh one from Apple when there is none or it
            // is refused.
            if let Some(url) = fresh(&photo.master).as_ref().and_then(pick) {
                if quince_icloud::photos::fetch_to_file(&http, &url, &part)
                    .await
                    .unwrap_or(false)
                {
                    return std::fs::rename(&part, &target)
                        .map(|()| true)
                        .map_err(failed);
                }
            }
            // Asked of the place Apple keeps it: the library, its Shared Library's zone, or
            // the shared album it is in.
            let links = match &place {
                Place::Library(zone) => {
                    quince_icloud::photos::links(
                        &http,
                        &root,
                        params,
                        zone.as_ref(),
                        &photo.master,
                        &photo.id,
                    )
                    .await
                }
                Place::Album(album) => {
                    quince_icloud::shared_collections::links(
                        &http,
                        &root,
                        params,
                        album,
                        &photo.master,
                        &photo.id,
                    )
                    .await
                }
            }
            .map_err(failed)?
            .ok_or_else(|| failed("Apple no longer has that photo."))?;
            remember(HashMap::from([(photo.master.clone(), links.clone())]));
            let Some(url) = pick(&links) else {
                return Ok(false);
            };
            if !quince_icloud::photos::fetch_to_file(&http, &url, &part)
                .await
                .map_err(failed)?
            {
                return Ok(false);
            }
            std::fs::rename(&part, &target)
                .map(|()| true)
                .map_err(failed)
        })
    }
}
