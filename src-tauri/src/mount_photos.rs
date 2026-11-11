//! iCloud Photos as a folder (quince-vfs), laid out as people look for
//! photos: by when they were taken, in their albums, the favourites, the videos.
//!
//!   Library/2026/10 October/IMG_1234.HEIC
//!   Albums/Trips/Goa/IMG_0042.JPG
//!   Favorites/…   Videos/…
//!
//! Each file is the original, under its own name and dated when it was taken; a Live Photo is
//! its still and, beside it under the same name, its moving part as shot (IMG_1234.MOV) — as
//! Image Capture brings them off an iPhone. Its bytes are
//! read from the app's own photo cache (fetched once if they are not there), never copied a
//! second time; thumbnails are the app's own, so a file manager browsing downloads nothing.
//! Hidden and Recently Deleted are never shown.
//!
//! Photos are added and deleted here, never rewritten in place (quince-vfs `add_only`): a
//! file put in the library, or in an album, goes up to iCloud Photos (and into the album); a
//! file deleted goes to Recently Deleted, or out of the album it was deleted from.

use crate::photos::{cache_path, cached, Size};
use crate::Data;
use quince_core::drive::DriveItem;
use quince_core::photos::{Photo, View};
use quince_vfs::{Fut, Remote, Store, ROOT};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const LIBRARY: &str = "PH::library";
const ALBUMS: &str = "PH::albums";
const FAVORITES: &str = "PH::favorites";
const VIDEOS: &str = "PH::videos";
const FILE: &str = "PHOTO::";
/// A Live Photo's moving part, beside its still.
const LIVE: &str = "LIVE::";

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn folder(id: String, parent: &str, name: String) -> DriveItem {
    DriveItem {
        drivewsid: id,
        parent_id: Some(parent.to_string()),
        name,
        extension: None,
        kind: "FOLDER".into(),
        size: None,
        child_count: None,
        created_at: None,
        modified_at: None,
        etag: None,
    }
}

/// A photo as a file: its own name (or one made from its id and kind), its size, the time it
/// was taken.
fn file(p: &Photo, parent: &str) -> DriveItem {
    let name = p.filename.clone().unwrap_or_else(|| {
        let ext = match p.kind {
            quince_core::photos::Kind::Video => "MOV",
            _ => "JPG",
        };
        format!("{}.{ext}", p.id)
    });
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), Some(e.to_string())),
        _ => (name, None),
    };
    DriveItem {
        drivewsid: format!("{FILE}{}", p.id),
        parent_id: Some(parent.to_string()),
        name: stem,
        extension: ext,
        kind: "FILE".into(),
        size: p.size,
        child_count: None,
        created_at: p.added_at,
        modified_at: Some(p.taken_at),
        etag: p.change_tag.clone(),
    }
}

/// A photo's files in a folder: the photo, and for a Live Photo its `.MOV` too.
fn files(p: &Photo, parent: &str) -> Vec<DriveItem> {
    let still = file(p, parent);
    let live = (p.kind == quince_core::photos::Kind::Live)
        .then_some(p.live_size)
        .flatten()
        .map(|size| DriveItem {
            drivewsid: format!("{LIVE}{}", p.id),
            extension: Some("MOV".into()),
            size: Some(size),
            ..still.clone()
        });
    std::iter::once(still).chain(live).collect()
}

/// The photo a file in the folder is of, and whether it is a Live Photo's moving part.
fn photo_of(item: &DriveItem) -> Option<(&str, bool)> {
    item.drivewsid
        .strip_prefix(FILE)
        .map(|id| (id, false))
        .or_else(|| item.drivewsid.strip_prefix(LIVE).map(|id| (id, true)))
}

/// The photo a file in File Explorer's iCloud Photos folder stands for (a Live Photo's `.MOV`:
/// its still), read from the file's own placeholder: for its thumbnail (thumbs.rs).
#[cfg(windows)]
pub fn photo_at(path: &std::path::Path) -> Option<String> {
    let item = quince_vfs::cloud::identity_of(path)?;
    photo_of(&item).map(|(id, _)| id.to_string())
}

/// (year, month 1–12) a photo was taken in, in this computer's time zone.
fn taken(p: &Photo) -> (i16, i8) {
    let zone = jiff::tz::TimeZone::system();
    jiff::Timestamp::from_second(p.taken_at)
        .map(|t| t.to_zoned(zone).date())
        .map_or((1970, 1), |d| (d.year(), d.month()))
}

/// What a folder holds, read from the library (the app keeps it in step with Apple).
///
/// Read beside the library's lock, never behind it: a file manager browsing the folder waited
/// out every sync pass the window was storing.
fn children(app: &AppHandle, id: &str) -> Option<Vec<DriveItem>> {
    app.state::<Data>()
        .0
        .blocking_read(|library| Ok(children_in(library, id)))
        .ok()
        .flatten()
}

fn children_in(library: &quince_core::Library, id: &str) -> Option<Vec<DriveItem>> {
    // Oldest first, so that of two photos with the same name (an iPhone's IMG_ numbers come
    // round again every ten thousand) the older keeps the plain name and the newer is "(2)".
    // Newest first, each new favourite or video with a name already there renamed the older
    // one, and backups and open paths saw a file move.
    let photos = |view: View| oldest_first(library.photos(view).unwrap_or_default());
    Some(match id {
        ROOT => vec![
            folder(LIBRARY.into(), ROOT, "Library".into()),
            folder(ALBUMS.into(), ROOT, "Albums".into()),
            folder(FAVORITES.into(), ROOT, "Favorites".into()),
            folder(VIDEOS.into(), ROOT, "Videos".into()),
        ],
        LIBRARY => {
            let mut years: Vec<i16> = photos(View::Library).iter().map(|p| taken(p).0).collect();
            years.sort_unstable();
            years.dedup();
            years
                .into_iter()
                .rev()
                .map(|y| folder(format!("PH::y:{y}"), LIBRARY, y.to_string()))
                .collect()
        }
        FAVORITES => photos(View::Favorites)
            .iter()
            .flat_map(|p| files(p, FAVORITES))
            .collect(),
        VIDEOS => photos(View::Videos)
            .iter()
            .flat_map(|p| files(p, VIDEOS))
            .collect(),
        ALBUMS => albums_in(library, None, ALBUMS),
        _ => {
            if let Some(y) = id.strip_prefix("PH::y:") {
                let y: i16 = y.parse().ok()?;
                let mut months: Vec<i8> = photos(View::Library)
                    .iter()
                    .filter(|p| taken(p).0 == y)
                    .map(|p| taken(p).1)
                    .collect();
                months.sort_unstable();
                months.dedup();
                months
                    .into_iter()
                    .map(|m| {
                        let name =
                            format!("{m:02} {}", MONTHS[usize::try_from(m - 1).unwrap_or(0)]);
                        folder(format!("PH::m:{y}-{m}"), id, name)
                    })
                    .collect()
            } else if let Some(ym) = id.strip_prefix("PH::m:") {
                let (y, m) = ym.split_once('-')?;
                let (y, m): (i16, i8) = (y.parse().ok()?, m.parse().ok()?);
                photos(View::Library)
                    .iter()
                    .filter(|p| taken(p) == (y, m))
                    .flat_map(|p| files(p, id))
                    .collect()
            } else {
                let album = id.strip_prefix("PH::album:")?;
                let is_folder = library
                    .albums()
                    .ok()?
                    .iter()
                    .any(|a| a.id == album && a.folder);
                if is_folder {
                    albums_in(library, Some(album), id)
                } else {
                    oldest_first(library.album_photos(album).ok()?)
                        .iter()
                        .filter(|p| !p.hidden && !p.deleted)
                        .flat_map(|p| files(p, id))
                        .collect()
                }
            }
        }
    })
}

/// Photos in the order their names are given out: taken first, named first.
fn oldest_first(mut photos: Vec<Photo>) -> Vec<Photo> {
    photos.sort_by(|a, b| a.taken_at.cmp(&b.taken_at).then_with(|| a.id.cmp(&b.id)));
    photos
}

/// The albums (and album folders) directly inside `parent`, in Photos' own order.
fn albums_in(library: &quince_core::Library, parent: Option<&str>, at: &str) -> Vec<DriveItem> {
    let mut albums: Vec<_> = library
        .albums()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.parent.as_deref() == parent)
        .collect();
    albums.sort_by_key(|a| a.position);
    albums
        .into_iter()
        .map(|a| folder(format!("PH::album:{}", a.id), at, a.name))
        .collect()
}

pub struct PhotosStore(pub AppHandle);

impl Store for PhotosStore {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        children(&self.0, folder)
    }
}

pub struct PhotosRemote(pub AppHandle);

/// Whether a folder is a place photos can be added: the library at any depth (a photo then
/// shows under the month it was taken), or an album (it goes into it too). Favorites, Videos,
/// the albums list and album folders are views, not places.
fn accepts(app: &AppHandle, folder: &str) -> bool {
    if folder == LIBRARY || folder.starts_with("PH::y:") || folder.starts_with("PH::m:") {
        return true;
    }
    folder.strip_prefix("PH::album:").is_some_and(|album| {
        app.state::<Data>()
            .0
            .blocking_read(|library| {
                Ok(library
                    .albums()
                    .unwrap_or_default()
                    .iter()
                    .any(|a| a.id == album && !a.folder))
            })
            .unwrap_or(false)
    })
}

impl Remote for PhotosRemote {
    fn accepts(&self, folder: &DriveItem) -> bool {
        // Asked from the folder's own threads, never the async ones: the lock can block.
        accepts(&self.0, &folder.drivewsid)
    }

    fn upload(&self, folder: &DriveItem, path: &std::path::Path) -> Fut<DriveItem> {
        let (app, folder, path) = (self.0.clone(), folder.drivewsid.clone(), path.to_path_buf());
        Box::pin(async move {
            let album = folder.strip_prefix("PH::album:").map(str::to_string);
            let asset = crate::photos::add_from_folder(&app, &path, album).await?;
            let made = {
                let data = app.state::<Data>();
                let library = data.0.lock().await;
                library.photo(&asset).map_err(|e| e.to_string())?
            };
            Ok(match made {
                Some(p) => file(&p, &folder),
                // Not read back yet: named as it was put there, until the next look.
                None => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let (stem, ext) = match name.rsplit_once('.') {
                        Some((s, e)) if !s.is_empty() => (s.to_string(), Some(e.to_string())),
                        _ => (name, None),
                    };
                    DriveItem {
                        drivewsid: format!("{FILE}{asset}"),
                        parent_id: Some(folder),
                        name: stem,
                        extension: ext,
                        kind: "FILE".into(),
                        size: std::fs::metadata(&path)
                            .ok()
                            .and_then(|m| i64::try_from(m.len()).ok()),
                        child_count: None,
                        created_at: None,
                        modified_at: None,
                        etag: None,
                    }
                }
            })
        })
    }

    fn delete(&self, item: &DriveItem) -> Fut<()> {
        let (app, item) = (self.0.clone(), item.clone());
        Box::pin(async move {
            let (id, _) = photo_of(&item).ok_or("not a photo")?;
            let ids = vec![id.to_string()];
            // In an album, Delete takes it out of the album, as Photos does; anywhere else it
            // goes to Recently Deleted, there for 30 days. A Live Photo's still and .MOV go as one.
            match item
                .parent_id
                .as_deref()
                .and_then(|f| f.strip_prefix("PH::album:"))
            {
                Some(album) => crate::photos::photos_album_remove(
                    album.to_string(),
                    ids,
                    app.state(),
                    app.state(),
                )
                .await
                .map_err(|e| e.to_string()),
                None => crate::photos::photos_set_flag(
                    ids,
                    quince_icloud::photos::Flag::Deleted,
                    true,
                    app.state(),
                )
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            }
        })
    }

    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        // Nothing to ask Apple: the library is kept in step by the app's own sync.
        let (app, folder) = (self.0.clone(), folder.to_string());
        Box::pin(async move {
            tauri::async_runtime::spawn_blocking(move || {
                children(&app, &folder).unwrap_or_default()
            })
            .await
            .map_err(|e| e.to_string())
        })
    }

    fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
        // Every photo is read from the app's cache (file): never through the folder's own.
        Box::pin(async { Err("read through the photo cache".to_string()) })
    }

    fn file(&self, item: &DriveItem) -> Option<Fut<PathBuf>> {
        let (id, live) = photo_of(item)?;
        let (id, size) = (
            id.to_string(),
            if live { Size::Live } else { Size::Original },
        );
        let app = self.0.clone();
        Some(Box::pin(async move { cached(&app, &id, size).await }))
    }

    fn is_local(&self, item: &DriveItem) -> bool {
        photo_of(item).is_some_and(|(id, live)| {
            cache_path(&self.0, id, if live { Size::Live } else { Size::Original }).exists()
        })
    }

    fn thumbnails(&self, folder: &str) -> Fut<HashMap<String, Vec<u8>>> {
        let (app, folder) = (self.0.clone(), folder.to_string());
        Box::pin(async move {
            let (a, f) = (app.clone(), folder.clone());
            let files =
                tauri::async_runtime::spawn_blocking(move || children(&a, &f).unwrap_or_default())
                    .await
                    .map_err(|e| e.to_string())?;
            let mut out = HashMap::new();
            for item in files.iter().filter(|i| i.kind == "FILE") {
                // A Live Photo's .MOV is pictured by its still, as Photos shows it.
                let Some((id, _)) = photo_of(item) else {
                    continue;
                };
                if let Ok(path) = cached(&app, id, Size::Thumb).await {
                    if let Ok(bytes) = tokio::fs::read(&path).await {
                        out.insert(item.drivewsid.clone(), bytes);
                    }
                }
            }
            Ok(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two photos with the same name keep their names as new ones arrive: the older the plain
    /// one, the newer "(2)" — a newer favourite of the same name used to take the plain name
    /// and rename the older file under its owner.
    #[test]
    fn same_named_photos_keep_their_names_as_new_ones_arrive() {
        let shot = |id: &str, at: i64| -> Photo {
            serde_json::from_value(serde_json::json!({
                "id": id, "master": format!("M{id}"), "takenAt": at, "kind": "image",
                "width": 4032, "height": 3024, "filename": "IMG_0001.JPG", "size": 1000,
                "favorite": true, "hidden": false, "deleted": false, "subtype": 0
            }))
            .unwrap()
        };
        let mut lib = quince_core::Library::in_memory().unwrap();
        lib.put_photos(&[shot("old", 1_600_000_000)]).unwrap();
        let names = |lib: &quince_core::Library| -> Vec<(String, String)> {
            quince_vfs::named(&children_in(lib, FAVORITES).unwrap())
                .into_iter()
                .map(|(n, i)| (n, i.drivewsid))
                .collect()
        };
        let before = names(&lib);
        lib.put_photos(&[shot("new", 1_700_000_000)]).unwrap();
        let after = names(&lib);
        let of = |all: &[(String, String)], id: &str| {
            all.iter()
                .find(|(_, d)| d.ends_with(id))
                .map(|(n, _)| n.clone())
        };
        assert_eq!(of(&before, "old"), Some("IMG_0001.JPG".into()));
        assert_eq!(of(&after, "old"), Some("IMG_0001.JPG".into()), "{after:?}");
        assert_eq!(of(&after, "new"), Some("IMG_0001 (2).JPG".into()));
    }

    /// A Live Photo is two files under one name — the still and its .MOV, at the moving part's
    /// own size — and each knows which picture it is.
    #[test]
    fn a_live_photo_is_its_still_and_its_mov() {
        let mut p: Photo = serde_json::from_value(serde_json::json!({
            "id": "A1", "master": "M1", "takenAt": 1_789_640_832, "kind": "live",
            "width": 4032, "height": 3024, "filename": "IMG_0001.HEIC", "size": 2_400_000,
            "favorite": false, "hidden": false, "deleted": false, "subtype": 0,
            "liveSize": 3_500_000
        }))
        .unwrap();
        let both = files(&p, "PH::m:2026-9");
        let named: Vec<_> = both
            .iter()
            .map(|f| (f.display_name(), f.size, photo_of(f)))
            .collect();
        assert_eq!(
            named,
            vec![
                (
                    "IMG_0001.HEIC".to_string(),
                    Some(2_400_000),
                    Some(("A1", false))
                ),
                (
                    "IMG_0001.MOV".to_string(),
                    Some(3_500_000),
                    Some(("A1", true))
                ),
            ]
        );
        p.kind = quince_core::photos::Kind::Image;
        assert_eq!(
            files(&p, "PH::m:2026-9").len(),
            1,
            "a still photo is one file"
        );
    }
}
