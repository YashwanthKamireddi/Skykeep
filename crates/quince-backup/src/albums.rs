//! Albums as folders in the backup: `Albums/<folder>/<album>/<file>`, each file a hard link to
//! the photo's own copy under `Photos/<year>/<month>`, so an album takes no extra space and a
//! photo in five albums is still stored once. icloudpd's most-asked open request (#125, open
//! since 2019).
//!
//! - Only links: an album's folder is made from what is already backed up, never downloaded.
//! - Follows Apple: a photo added to an album gets its link on the next pass; one taken out, or
//!   an album renamed or deleted, has its link taken away. A link goes only while the photo's
//!   own copy is there and the same size, so taking one away never loses a photo.
//! - Names stay put: a link keeps the name it was given; a new one gets a free name, a Live
//!   Photo's still and video the same stem, as under `Photos/`.
//! - Hidden photos never: an album's folder is a plain folder, and would step around the lock.
//! - A drive with no hard links (FAT32, exFAT, some network shares) gets no album folders and
//!   a note why; copying every photo twice could fill it.

use quince_core::backup::{Copied, ALBUMS, ORIGINAL, PHOTOS};
use quince_core::photos::Album;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The top folder albums are laid out in.
pub const TOP: &str = "Albums";

/// What one pass over the albums did: links made (to record), links taken away (to forget),
/// and why it stopped short, if it did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Laid {
    pub made: Vec<Copied>,
    pub gone: Vec<(String, String)>,
    pub note: Option<String>,
}

/// The library's side: the albums, each album's photos in order, and the files backed up.
pub struct Library<'a> {
    pub albums: &'a [Album],
    /// `(album id, photo id)`, in each album's order.
    pub members: &'a [(String, String)],
    /// Every recorded file in the backup: photos' own and albums' links.
    pub files: &'a [Copied],
}

/// Each album's folder, relative to the backup folder: its folders' names, then its own, each
/// made one safe file name; two of one name in one place told apart as `Name 2`, in Apple's
/// order (position, name, id), so the same album keeps the same folder pass after pass.
fn album_dirs(albums: &[Album]) -> HashMap<String, String> {
    let by_id: HashMap<&str, &Album> = albums.iter().map(|a| (a.id.as_str(), a)).collect();
    let mut ordered: Vec<&Album> = albums.iter().collect();
    ordered.sort_by(|a, b| (a.position, &a.name, &a.id).cmp(&(b.position, &b.name, &b.id)));
    // Each album's own name, unique among the others in its place.
    let mut own: HashMap<&str, String> = HashMap::new();
    let mut used: HashMap<Option<&str>, HashSet<String>> = HashMap::new();
    for album in &ordered {
        let place = album.parent.as_deref().filter(|p| by_id.contains_key(p));
        let base = crate::names::file_name(&album.name, "Album");
        let taken = used.entry(place).or_default();
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base} {n}")
                }
            })
            .find(|n| !taken.contains(&n.to_lowercase()))
            .unwrap_or(base);
        taken.insert(name.to_lowercase());
        own.insert(album.id.as_str(), name);
    }
    let mut dirs = HashMap::new();
    for album in albums.iter().filter(|a| !a.folder) {
        let mut parts = vec![own[album.id.as_str()].clone()];
        let mut seen = HashSet::from([album.id.as_str()]);
        let mut at = album.parent.as_deref();
        // A parent chain that loops (a bad record) ends where it would repeat.
        while let Some(parent) = at.and_then(|p| by_id.get(p)) {
            if !seen.insert(parent.id.as_str()) {
                break;
            }
            parts.push(own[parent.id.as_str()].clone());
            at = parent.parent.as_deref();
        }
        parts.push(TOP.to_string());
        parts.reverse();
        dirs.insert(album.id.clone(), parts.join("/"));
    }
    dirs
}

/// The file name part of a recorded path.
fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The directory part of a recorded path.
fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Lays `Albums/` out under `folder` as the library has it now. Blocking (file system work):
/// run it off the async runtime.
pub fn lay_out(folder: &Path, library: &Library<'_>) -> Laid {
    // No albums at all while links are recorded: the library is being read again from the
    // start (a full sync replaces the album list), not every album deleted. Nothing is taken
    // away on a read like that (another iCloud tool's sync emptied people's backups this way,
    // steilerDev/icloud-photos-sync#871); the next pass, with the albums back, carries on.
    if library.albums.is_empty() && library.files.iter().any(|c| c.source == ALBUMS) {
        return Laid::default();
    }
    let dirs = album_dirs(library.albums);
    // Photos' own files, by photo and part, and all of a photo's together (the original first).
    let own: HashMap<(&str, &str), &Copied> = library
        .files
        .iter()
        .filter(|c| c.source == PHOTOS)
        .map(|c| ((c.item.as_str(), c.part.as_str()), c))
        .collect();
    let mut by_photo: HashMap<&str, Vec<&Copied>> = HashMap::new();
    for c in library.files.iter().filter(|c| c.source == PHOTOS) {
        by_photo.entry(c.item.as_str()).or_default().push(c);
    }
    for parts in by_photo.values_mut() {
        parts.sort_by_key(|c| (c.part != ORIGINAL, c.part.clone()));
    }
    let recorded: HashMap<(&str, &str), &Copied> = library
        .files
        .iter()
        .filter(|c| c.source == ALBUMS)
        .map(|c| ((c.item.as_str(), c.part.as_str()), c))
        .collect();

    // What should be there: each album's backed-up photos, every part of each.
    let mut wanted: Vec<(String, &str, String, Vec<&Copied>)> = Vec::new();
    for (album, photo) in library.members {
        let Some(dir) = dirs.get(album) else { continue };
        // Not backed up yet: its link comes once it is.
        let Some(parts) = by_photo.get(photo.as_str()).cloned() else {
            continue;
        };
        wanted.push((
            format!("{album}/{photo}"),
            photo.as_str(),
            dir.clone(),
            parts,
        ));
    }

    let mut laid = Laid::default();
    let mut keep: HashSet<(String, String)> = HashSet::new();
    // Names in each album's folder already given: recorded links that stay where they are.
    let mut claimed: HashMap<String, HashSet<String>> = HashMap::new();
    for (item, _, dir, parts) in &wanted {
        for own in parts {
            if let Some(link) = recorded.get(&(item.as_str(), own.part.as_str())) {
                if dir_of(&link.path) == dir {
                    keep.insert((item.clone(), own.part.clone()));
                    claimed
                        .entry(dir.clone())
                        .or_default()
                        .insert(name_of(&link.path).to_lowercase());
                }
            }
        }
    }

    // Links no longer wanted (out of the album, album renamed or gone): taken away while the
    // photo's own copy is whole, so nothing but the link goes.
    for ((item, part), link) in &recorded {
        if keep.contains(&(item.to_string(), part.to_string())) {
            continue;
        }
        let photo = item.split_once('/').map_or("", |(_, p)| p);
        let whole = own.get(&(photo, *part)).is_some_and(|c| {
            std::fs::metadata(folder.join(&c.path)).is_ok_and(|m| m.len() as i64 == c.size)
        });
        // The photo's own copy not whole: the link may be the only copy left, so it stays, and
        // stays recorded, until the photo is copied again.
        if !whole {
            continue;
        }
        if link.path.starts_with(&format!("{TOP}/")) {
            let path = folder.join(&link.path);
            let removed = match std::fs::symlink_metadata(&path) {
                Ok(m) if m.is_file() => std::fs::remove_file(&path).is_ok(),
                // Something else under its name now (a folder the person made): left alone.
                Ok(_) => true,
                Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            };
            // Couldn't be taken away (in use, read-only): kept recorded, tried again next pass.
            if !removed {
                continue;
            }
            // The album's folders, once empty (a non-empty one is left as it is).
            let mut dir = path.parent();
            while let Some(d) = dir {
                if d == folder.join(TOP) || std::fs::remove_dir(d).is_err() {
                    break;
                }
                dir = d.parent();
            }
        }
        laid.gone.push((ALBUMS.to_string(), item.to_string()));
    }

    // New links: a free stem in the album's folder for all of a photo's parts together.
    for (item, _, dir, parts) in &wanted {
        let new: Vec<&&Copied> = parts
            .iter()
            .filter(|c| !keep.contains(&(item.clone(), c.part.clone())))
            .collect();
        if new.is_empty() {
            continue;
        }
        let taken = claimed.entry(dir.clone()).or_default();
        let on_disk: HashSet<String> = std::fs::read_dir(folder.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_lowercase())
            .collect();
        let first = name_of(&parts[0].path);
        let (stem, _) = crate::names::name_parts(first);
        let names_for = |n: usize| -> Vec<String> {
            new.iter()
                .map(|c| {
                    let (_, ext) = crate::names::name_parts(name_of(&c.path));
                    if n == 1 {
                        format!("{stem}{ext}")
                    } else {
                        format!("{stem} {n}{ext}")
                    }
                })
                .collect()
        };
        let Some(names) = (1..10_000).map(names_for).find(|names| {
            names.iter().all(|n| {
                let key = n.to_lowercase();
                !taken.contains(&key) && !on_disk.contains(&key)
            })
        }) else {
            continue;
        };
        if let Err(e) = std::fs::create_dir_all(folder.join(dir)) {
            laid.note = Some(format!("Album folders couldn't be made: {e}"));
            return laid;
        }
        for (own, name) in new.iter().zip(&names) {
            let rel = format!("{dir}/{name}");
            let path = folder.join(&rel);
            // Every piece of the name was made safe; still, nothing is written outside the
            // album's own folder.
            if !crate::saving::directly_in(&folder.join(dir), &path) {
                continue;
            }
            match std::fs::hard_link(folder.join(&own.path), &path) {
                Ok(()) => {
                    taken.insert(name.to_lowercase());
                    laid.made.push(Copied {
                        source: ALBUMS.into(),
                        item: item.clone(),
                        part: own.part.clone(),
                        path: rel,
                        size: own.size,
                        version: None,
                    });
                }
                // The photo's own copy went missing since: its check copies it again, and its
                // link comes after.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // A photo in more albums than its file can have links (1,023 on NTFS): this
                // album goes without it; every other link is made.
                Err(e) if too_many_links(&e) => {}
                Err(e) => {
                    laid.note = Some(no_links(&e));
                    return laid;
                }
            }
        }
    }
    laid
}

/// The file already has as many links as its disk allows (EMLINK; ERROR_TOO_MANY_LINKS).
fn too_many_links(e: &std::io::Error) -> bool {
    #[cfg(windows)]
    const TOO_MANY: i32 = 1142;
    #[cfg(not(windows))]
    const TOO_MANY: i32 = 31;
    e.raw_os_error() == Some(TOO_MANY)
}

/// Why album folders stopped, in words a person can act on.
fn no_links(e: &std::io::Error) -> String {
    use std::io::ErrorKind::{QuotaExceeded, StorageFull};
    match e.kind() {
        StorageFull | QuotaExceeded => "The backup drive is full, so album folders weren't finished.".into(),
        _ => "This backup drive can't keep albums as folders (it has no hard links, as FAT32 and exFAT don't), so photos are kept by date only.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::backup::LIVE;

    fn album(id: &str, name: &str, position: i64, parent: Option<&str>, folder: bool) -> Album {
        Album {
            id: id.into(),
            name: name.into(),
            position,
            change_tag: None,
            folder,
            parent: parent.map(Into::into),
        }
    }

    fn own(dir: &Path, photo: &str, part: &str, rel: &str, bytes: &[u8]) -> Copied {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        Copied {
            source: PHOTOS.into(),
            item: photo.into(),
            part: part.into(),
            path: rel.into(),
            size: bytes.len() as i64,
            version: None,
        }
    }

    fn base(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("skykeep-albums-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(unix)]
    fn same_file(a: &Path, b: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        let (a, b) = (std::fs::metadata(a).unwrap(), std::fs::metadata(b).unwrap());
        (a.dev(), a.ino()) == (b.dev(), b.ino())
    }

    /// An album is a folder of links to the photos' own files: nothing stored twice, a Live
    /// Photo's still and video under one stem, albums inside folders nested, two albums of one
    /// name told apart, hidden-safe names, and a second pass changes nothing.
    #[test]
    fn albums_are_folders_of_links_to_the_photos_own_files() {
        let dir = base("lay");
        let mut files = vec![
            own(
                &dir,
                "p1",
                ORIGINAL,
                "Photos/2026/01/IMG_1.HEIC",
                b"still-one",
            ),
            own(&dir, "p1", LIVE, "Photos/2026/01/IMG_1.MOV", b"video-one!"),
            own(&dir, "p2", ORIGINAL, "Photos/2025/07/IMG_1.JPG", b"another"),
            own(&dir, "p3", ORIGINAL, "Photos/2025/07/IMG_3.JPG", b"third"),
            own(&dir, "p4", ORIGINAL, "Photos/2024/02/IMG_3.JPG", b"fourth!"),
        ];
        let albums = [
            album("trips", "Trips", 0, None, true),
            album("goa", "Goa", 1, Some("trips"), false),
            album("fav", "Family", 2, None, false),
            album("fav2", "family", 3, None, false),
            album("odd", "../../escape", 4, None, false),
        ];
        let members: Vec<(String, String)> = [
            ("goa", "p1"),
            ("goa", "p2"),
            ("fav", "p3"),
            ("fav", "p4"),
            ("fav2", "p3"),
            ("odd", "p3"),
            ("fav", "not-backed-up"),
        ]
        .iter()
        .map(|(a, p)| (a.to_string(), p.to_string()))
        .collect();
        let laid = lay_out(
            &dir,
            &Library {
                albums: &albums,
                members: &members,
                files: &files,
            },
        );
        assert_eq!(laid.note, None);
        let paths: HashSet<&str> = laid.made.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(
            paths,
            HashSet::from([
                "Albums/Trips/Goa/IMG_1.HEIC",
                "Albums/Trips/Goa/IMG_1.MOV",
                // Another photo whose name differs only in its extension: kept as it is, as
                // under Photos/.
                "Albums/Trips/Goa/IMG_1.JPG",
                "Albums/Family/IMG_3.JPG",
                // Another photo of the same name in the same album: its own name.
                "Albums/Family/IMG_3 2.JPG",
                "Albums/family 2/IMG_3.JPG",
                "Albums/escape/IMG_3.JPG",
            ])
        );
        for link in &laid.made {
            let target = files
                .iter()
                .find(|c| link.item.ends_with(&format!("/{}", c.item)) && c.part == link.part)
                .unwrap();
            assert_eq!(
                std::fs::read(dir.join(&link.path)).unwrap(),
                std::fs::read(dir.join(&target.path)).unwrap()
            );
            #[cfg(unix)]
            assert!(
                same_file(&dir.join(&link.path), &dir.join(&target.path)),
                "a link, not a copy"
            );
        }

        // Recorded, the next pass has nothing to do.
        files.extend(laid.made);
        let again = lay_out(
            &dir,
            &Library {
                albums: &albums,
                members: &members,
                files: &files,
            },
        );
        assert_eq!(again, Laid::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A photo taken out of an album, or an album renamed, loses only its link: the photo's own
    /// file stays, an emptied album folder goes, and a renamed album's photos are linked under
    /// its new name. A link whose photo's own copy is gone is never taken away (it may be the
    /// only copy left).
    #[test]
    fn a_link_goes_when_its_photo_leaves_the_album_never_the_photo() {
        let dir = base("leave");
        let mut files = vec![
            own(&dir, "p1", ORIGINAL, "Photos/2026/01/A.JPG", b"aaaa"),
            own(&dir, "p2", ORIGINAL, "Photos/2026/01/B.JPG", b"bbbbb"),
        ];
        let mut albums = vec![album("x", "Party", 0, None, false)];
        let both: Vec<(String, String)> =
            vec![("x".into(), "p1".into()), ("x".into(), "p2".into())];
        let laid = lay_out(
            &dir,
            &Library {
                albums: &albums,
                members: &both,
                files: &files,
            },
        );
        assert_eq!(laid.made.len(), 2);
        files.extend(laid.made);

        // p2 taken out of the album.
        let one: Vec<(String, String)> = vec![("x".into(), "p1".into())];
        let laid = lay_out(
            &dir,
            &Library {
                albums: &albums,
                members: &one,
                files: &files,
            },
        );
        assert_eq!(laid.gone, vec![(ALBUMS.to_string(), "x/p2".to_string())]);
        assert!(!dir.join("Albums/Party/B.JPG").exists());
        assert!(
            dir.join("Photos/2026/01/B.JPG").is_file(),
            "the photo itself stays"
        );
        files.retain(|c| !(c.source == ALBUMS && c.item == "x/p2"));

        // Renamed: the old folder emptied and gone, the photo linked under the new name.
        albums[0].name = "Party 2026".into();
        let laid = lay_out(
            &dir,
            &Library {
                albums: &albums,
                members: &one,
                files: &files,
            },
        );
        assert_eq!(laid.gone, vec![(ALBUMS.to_string(), "x/p1".to_string())]);
        assert_eq!(laid.made.len(), 1);
        assert_eq!(laid.made[0].path, "Albums/Party 2026/A.JPG");
        assert!(!dir.join("Albums/Party").exists());
        files.retain(|c| c.source != ALBUMS);
        files.extend(laid.made);

        // The album list read empty (a full sync under way): nothing taken away.
        let laid = lay_out(
            &dir,
            &Library {
                albums: &[],
                members: &[],
                files: &files,
            },
        );
        assert_eq!(laid, Laid::default());
        assert!(dir.join("Albums/Party 2026/A.JPG").is_file());

        // The photo's own copy gone from the drive: its link is the only copy, so it stays.
        std::fs::remove_file(dir.join("Photos/2026/01/A.JPG")).unwrap();
        let laid = lay_out(
            &dir,
            &Library {
                albums: &albums,
                members: &[],
                files: &files,
            },
        );
        assert_eq!(laid, Laid::default(), "kept, and kept recorded");
        assert!(
            dir.join("Albums/Party 2026/A.JPG").is_file(),
            "never the last copy"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
