//! Backup's record: every file copied into the backup folder, where, and how large (migration
//! 53, `backup_files`). One row per file: a photo's original (`part` "original"), its Live Photo
//! video ("live"); later phases add rows (an edited version, iCloud Drive's files), never tables.
//!
//! The folder itself is the truth: src-tauri/src/backup.rs checks each row against the file it
//! names and lets go of any whose file is missing or another size, so the next pass copies that
//! photo again.
//!
//! What is backed up: every photo and video in the library, the personal library and the
//! Shared Library; Recently Deleted never (it is on its way out of iCloud); Hidden only when
//! asked for; every photo of a burst, not only the one Photos shows.

use crate::photos::{row_to_photo, Photo, COLUMNS};
use crate::{Library, Result};
use rusqlite::params;

/// Where a backed-up file came from.
pub const PHOTOS: &str = "photos";
/// The parts of a photo that are copied.
pub const ORIGINAL: &str = "original";
pub const LIVE: &str = "live";
/// An album's folder in the backup (`Albums/<album>/<file>`): a hard link to the photo's own
/// file under `Photos/`, so no photo is stored twice. Its item is `<album id>/<photo id>`.
pub const ALBUMS: &str = "albums";
/// A shared album's photo, copied into `Shared Albums/<album>/`: its item is
/// `<album zone>/<photo id>` (ids are an album's own, so two albums may share one).
pub const SHARED: &str = "shared";

/// A shared album's photo waiting to be copied: the photo, where it lives, and its album.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedPending {
    pub photo: Photo,
    pub place: crate::shared::SharedPlace,
    pub title: Option<String>,
}

/// One file in the backup folder: its path relative to the folder, and its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    pub source: String,
    pub item: String,
    pub part: String,
    pub path: String,
    pub size: i64,
    pub version: Option<String>,
}

/// How far a backup is: photos copied of all there are to copy, and their bytes (a Live Photo's
/// video counted with its still).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct Progress {
    pub copied: i64,
    pub total: i64,
    pub copied_bytes: i64,
    pub total_bytes: i64,
}

fn wanted(hidden: bool) -> &'static str {
    if hidden {
        "deleted = 0"
    } else {
        "deleted = 0 AND hidden = 0"
    }
}

impl Library {
    /// Up to `limit` photos whose original is not copied yet, newest first: what was just taken
    /// is safe soonest. `skip`: photos left alone this run.
    pub fn backup_pending(
        &self,
        hidden: bool,
        limit: usize,
        skip: &[String],
    ) -> Result<Vec<Photo>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM photos
             WHERE {} AND NOT EXISTS (SELECT 1 FROM backup_files b
                                      WHERE b.source = '{PHOTOS}' AND b.item = photos.id
                                        AND b.part = '{ORIGINAL}')
             ORDER BY taken_at DESC, id
             LIMIT ?1",
            wanted(hidden)
        ))?;
        let rows = stmt.query_map(params![(limit + skip.len()) as i64], row_to_photo)?;
        let mut out = Vec::new();
        for row in rows {
            let photo = row?;
            if !skip.contains(&photo.id) {
                out.push(photo);
            }
            if out.len() == limit {
                break;
            }
        }
        Ok(out)
    }

    /// Files copied, recorded together: a photo's original and its Live video are one step.
    pub fn backup_copied(&mut self, files: &[Copied]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO backup_files (source, item, part, path, size, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (source, item, part) DO UPDATE SET path = excluded.path,
                     size = excluded.size, version = excluded.version",
            )?;
            for f in files {
                stmt.execute(params![f.source, f.item, f.part, f.path, f.size, f.version])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every file recorded as copied, to be checked against the folder.
    pub fn backup_files(&self) -> Result<Vec<Copied>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT source, item, part, path, size, version FROM backup_files")?;
        let rows = stmt.query_map([], |r| {
            Ok(Copied {
                source: r.get(0)?,
                item: r.get(1)?,
                part: r.get(2)?,
                path: r.get(3)?,
                size: r.get(4)?,
                version: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Items no longer whole in the folder (a file gone or changed): every part of each let go
    /// of, so the item is copied again whole.
    pub fn backup_forget(&mut self, items: &[(String, String)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt =
                tx.prepare_cached("DELETE FROM backup_files WHERE source = ?1 AND item = ?2")?;
            for (source, item) in items {
                stmt.execute(params![source, item])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// A new backup folder: nothing is copied there yet.
    pub fn backup_clear(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM backup_files", [])?;
        Ok(())
    }

    /// Every path used in the folder, lower-cased: a new file never takes another's name, on
    /// disks where case doesn't tell names apart either.
    pub fn backup_paths(&self) -> Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare_cached("SELECT path FROM backup_files")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = std::collections::HashSet::new();
        for path in rows {
            out.insert(path?.to_lowercase());
        }
        Ok(out)
    }

    /// Every album's photos, in each album's own order: none deleted, and none hidden (an
    /// album's folder is a plain folder, so the Hidden album's lock would be stepped around).
    pub fn backup_album_members(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.album, i.asset FROM photo_album_items i JOIN photos p ON p.id = i.asset
             WHERE p.deleted = 0 AND p.hidden = 0
             ORDER BY i.album, i.position, p.taken_at DESC, p.id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Up to `limit` shared albums' photos not copied yet, newest first. `skip`: items
    /// (`<zone>/<id>`) left alone this run.
    pub fn backup_shared_pending(
        &self,
        limit: usize,
        skip: &[String],
    ) -> Result<Vec<SharedPending>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT p.photo, a.zone, a.database, a.owner, a.title FROM shared_album_photos p
               JOIN shared_albums a ON a.zone = p.zone
              WHERE NOT EXISTS (SELECT 1 FROM backup_files b
                                 WHERE b.source = '{SHARED}' AND b.item = p.zone || '/' || p.id
                                   AND b.part = '{ORIGINAL}')
              ORDER BY p.taken_at DESC, p.zone, p.id
              LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![(limit + skip.len()) as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                crate::shared::SharedPlace {
                    zone: r.get(1)?,
                    database: r.get(2)?,
                    owner: r.get(3)?,
                },
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (json, place, title) = row?;
            let photo: Photo = serde_json::from_str(&json).map_err(|e| {
                crate::Error::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?;
            if skip.contains(&format!("{}/{}", place.zone, photo.id)) {
                continue;
            }
            out.push(SharedPending {
                photo,
                place,
                title,
            });
            if out.len() == limit {
                break;
            }
        }
        Ok(out)
    }

    /// Shared albums' photos copied, of all there are.
    pub fn backup_shared_progress(&self) -> Result<(i64, i64)> {
        Ok(self.conn.query_row(
            &format!(
                "SELECT COUNT(b.item), COUNT(*) FROM shared_album_photos p
                   LEFT JOIN backup_files b ON b.source = '{SHARED}'
                         AND b.item = p.zone || '/' || p.id AND b.part = '{ORIGINAL}'"
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// Photos copied of all there are to copy.
    pub fn backup_progress(&self, hidden: bool) -> Result<Progress> {
        let (copied, total, total_bytes) = self.conn.query_row(
            &format!(
                "SELECT COUNT(b.item), COUNT(*),
                        COALESCE(SUM(COALESCE(photos.size, 0) + COALESCE(photos.live_size, 0)), 0)
                 FROM photos
                 LEFT JOIN backup_files b ON b.source = '{PHOTOS}' AND b.item = photos.id
                                         AND b.part = '{ORIGINAL}'
                 WHERE {}",
                wanted(hidden)
            ),
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let copied_bytes = self.conn.query_row(
            &format!(
                "SELECT COALESCE(SUM(b.size), 0) FROM backup_files b
                 JOIN photos ON photos.id = b.item
                 WHERE b.source = '{PHOTOS}' AND {}",
                wanted(hidden)
            ),
            [],
            |r| r.get(0),
        )?;
        Ok(Progress {
            copied,
            total,
            copied_bytes,
            total_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::tests::photo;

    fn file(item: &str, part: &str) -> Copied {
        Copied {
            source: PHOTOS.into(),
            item: item.into(),
            part: part.into(),
            path: format!("Photos/2026/11/{item}-{part}"),
            size: 10,
            version: None,
        }
    }

    #[test]
    fn what_is_left_to_copy_newest_first_never_deleted_hidden_when_asked() {
        let mut lib = Library::in_memory().unwrap();
        let mut hidden = photo("hidden", 50);
        hidden.hidden = true;
        let mut gone = photo("gone", 60);
        gone.deleted = true;
        lib.put_photos(&[photo("old", 10), photo("new", 90), hidden, gone])
            .unwrap();
        let ids = |lib: &Library, hidden, skip: &[String]| {
            lib.backup_pending(hidden, 10, skip)
                .unwrap()
                .into_iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&lib, false, &[]), ["new", "old"]);
        assert_eq!(ids(&lib, true, &[]), ["new", "hidden", "old"]);
        assert_eq!(
            ids(&lib, false, &["new".into()]),
            ["old"],
            "left alone this run"
        );
        assert_eq!(
            lib.backup_progress(false).unwrap(),
            Progress {
                copied: 0,
                total: 2,
                copied_bytes: 0,
                total_bytes: 2_000_000
            }
        );

        // An original and its Live video, one step; a Live video alone never counts as copied.
        lib.backup_copied(&[file("new", ORIGINAL), file("new", LIVE)])
            .unwrap();
        lib.backup_copied(&[file("old", LIVE)]).unwrap();
        assert_eq!(ids(&lib, false, &[]), ["old"]);
        assert_eq!(
            lib.backup_progress(false).unwrap(),
            Progress {
                copied: 1,
                total: 2,
                // Every file in the folder counts: `new`'s still and video, and `old`'s video.
                copied_bytes: 30,
                total_bytes: 2_000_000
            }
        );
        assert!(lib
            .backup_paths()
            .unwrap()
            .contains("photos/2026/11/new-live"));

        // A file of it gone from the folder: the whole photo copied again.
        lib.backup_forget(&[(PHOTOS.into(), "new".into())]).unwrap();
        assert_eq!(ids(&lib, false, &[]), ["new", "old"]);
        assert!(lib.backup_files().unwrap().iter().all(|f| f.item != "new"));
        lib.backup_clear().unwrap();
        assert!(lib.backup_files().unwrap().is_empty());
    }

    /// Shared albums' photos waiting, newest first, until each is copied; one photo id in two
    /// albums is two items.
    #[test]
    fn shared_albums_photos_wait_until_copied() {
        use crate::shared::SharedAlbum;
        let mut lib = Library::in_memory().unwrap();
        let album = |zone: &str| SharedAlbum {
            zone: zone.into(),
            database: "shared".into(),
            owner: Some("_owner".into()),
            title: Some(format!("Trip {zone}")),
            expires_at: None,
            count: 0,
            cover: None,
        };
        lib.put_shared_album(&album("z1"), &[photo("a", 10), photo("b", 90)], None)
            .unwrap();
        lib.put_shared_album(&album("z2"), &[photo("a", 50)], None)
            .unwrap();
        let items = |lib: &Library, skip: &[String]| {
            lib.backup_shared_pending(10, skip)
                .unwrap()
                .into_iter()
                .map(|p| format!("{}/{}", p.place.zone, p.photo.id))
                .collect::<Vec<_>>()
        };
        assert_eq!(items(&lib, &[]), ["z1/b", "z2/a", "z1/a"]);
        assert_eq!(items(&lib, &["z2/a".into()]), ["z1/b", "z1/a"]);
        assert_eq!(lib.backup_shared_progress().unwrap(), (0, 3));
        lib.backup_copied(&[Copied {
            source: SHARED.into(),
            item: "z2/a".into(),
            part: ORIGINAL.into(),
            path: "Shared Albums/Trip z2/a.JPG".into(),
            size: 5,
            version: None,
        }])
        .unwrap();
        assert_eq!(items(&lib, &[]), ["z1/b", "z1/a"]);
        assert_eq!(lib.backup_shared_progress().unwrap(), (1, 3));
        let first = &lib.backup_shared_pending(1, &[]).unwrap()[0];
        assert_eq!(first.title.as_deref(), Some("Trip z1"));
        assert_eq!(first.place.owner.as_deref(), Some("_owner"));
    }

    /// Albums' photos for their folders, in each album's order: never a hidden or deleted one.
    #[test]
    fn album_members_are_in_order_never_hidden_or_deleted() {
        let mut lib = Library::in_memory().unwrap();
        let mut hidden = photo("hidden", 50);
        hidden.hidden = true;
        let mut gone = photo("gone", 60);
        gone.deleted = true;
        lib.put_photos(&[photo("a", 10), photo("b", 20), hidden, gone])
            .unwrap();
        lib.put_album_items(&[
            ("x".into(), "b".into(), 1),
            ("x".into(), "a".into(), 2),
            ("x".into(), "hidden".into(), 3),
            ("x".into(), "gone".into(), 4),
        ])
        .unwrap();
        assert_eq!(
            lib.backup_album_members().unwrap(),
            [("x".to_string(), "b".to_string()), ("x".into(), "a".into())]
        );
    }
}
