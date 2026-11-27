//! Free up iCloud space: which photos and videos may leave iCloud because the backup has them,
//! and what stays and why. The library's side only: the content check against Apple's own
//! checksum, the edit check and the removal itself are quince-backup's (free_up.rs), done on
//! Apple's records as they are at that moment. docs/free-up-space.md has the research behind
//! every rule here (icloudpd's and icloud-photos-sync's lost-photo bugs, designed out).
//!
//! A photo may leave only when all of these hold, judged by its id, never a file name:
//! - it is older than the age chosen, by the day it was taken, and that day is known;
//! - it is not a favourite, not hidden, and not in a Shared Library (it belongs to everyone in
//!   it, and leaving would take it from them too);
//! - every part of it is in the backup: its original, and a Live Photo's video too.
//!
//! What Skykeep removed is remembered with the albums it was in (migration 59): the backup's
//! album folders keep linking it after Apple lets it go, as they would not for a photo deleted
//! on a device.

use crate::backup::{Copied, LIVE, ORIGINAL, PHOTOS};
use crate::photos::Kind;
use crate::{Library, Result};
use rusqlite::params;
use serde::Serialize;

/// One photo or video that may leave iCloud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeItem {
    pub id: String,
    pub taken_at: i64,
    /// Its original's bytes and a Live Photo's video's, as Apple gives them.
    pub bytes: i64,
    pub video: bool,
}

/// Of the photos old enough, those that stay in iCloud, by why.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stays {
    pub favourites: i64,
    pub hidden: i64,
    pub shared_library: i64,
    pub not_backed_up: i64,
    pub undated: i64,
}

/// What freeing up space would do: the photos that may leave, newest first, and what stays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub items: Vec<FreeItem>,
    pub stays: Stays,
}

impl Library {
    /// The photos taken before `before` (unix seconds) that may leave iCloud, and those that
    /// stay, and why. A photo counts once, under the first reason that keeps it.
    pub fn free_up_plan(&self, before: i64) -> Result<Plan> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT p.id, p.taken_at, COALESCE(p.size, 0) + COALESCE(p.live_size, 0), p.kind,
                    p.favorite, p.hidden, p.zone IS NOT NULL,
                    EXISTS (SELECT 1 FROM backup_files b WHERE b.source = '{PHOTOS}'
                              AND b.item = p.id AND b.part = '{ORIGINAL}'),
                    EXISTS (SELECT 1 FROM backup_files b WHERE b.source = '{PHOTOS}'
                              AND b.item = p.id AND b.part = '{LIVE}')
               FROM photos p
              WHERE p.deleted = 0 AND p.taken_at < ?1
              ORDER BY p.taken_at DESC, p.id"
        ))?;
        let rows = stmt.query_map(params![before], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, bool>(5)?,
                r.get::<_, bool>(6)?,
                r.get::<_, bool>(7)?,
                r.get::<_, bool>(8)?,
            ))
        })?;
        let mut plan = Plan::default();
        for row in rows {
            let (id, taken_at, bytes, kind, favourite, hidden, shared, original, live) = row?;
            let kind = Kind::parse(&kind);
            let s = &mut plan.stays;
            if taken_at <= 0 {
                s.undated += 1;
            } else if favourite {
                s.favourites += 1;
            } else if hidden {
                s.hidden += 1;
            } else if shared {
                s.shared_library += 1;
            } else if !original || (kind == Kind::Live && !live) {
                s.not_backed_up += 1;
            } else {
                plan.items.push(FreeItem {
                    id,
                    taken_at,
                    bytes,
                    video: kind == Kind::Video,
                });
            }
        }
        Ok(plan)
    }

    /// The backup's files of each photo (`photos` rows), to be checked before it leaves.
    pub fn free_up_files(&self, ids: &[String]) -> Result<Vec<Copied>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT source, item, part, path, size, version FROM backup_files
              WHERE source = '{PHOTOS}' AND item IN (SELECT value FROM json_each(?1))"
        ))?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let rows = stmt.query_map(params![crate::id_list(&ids)], |r| {
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

    /// Photos Skykeep is removing from iCloud, remembered with the albums each is in now: the
    /// backup's album folders keep them once Apple lets them go.
    pub fn free_up_remember(&mut self, ids: &[String], at: i64) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut put = tx.prepare(
                "INSERT INTO backup_freed (asset, albums, freed_at)
                 VALUES (?1, (SELECT COALESCE(json_group_array(album), '[]')
                                FROM photo_album_items WHERE asset = ?1), ?2)
                 ON CONFLICT (asset) DO UPDATE SET albums = excluded.albums,
                     freed_at = excluded.freed_at",
            )?;
            for id in ids {
                put.execute(params![id, at])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Photos Skykeep freed and the albums they were in: `(album, asset)`, for the backup's
    /// album folders.
    pub fn freed_album_members(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT a.value, f.asset FROM backup_freed f, json_each(f.albums) a
              ORDER BY a.value, f.freed_at, f.asset",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photos::tests::photo;

    fn copied(item: &str, part: &str) -> Copied {
        Copied {
            source: PHOTOS.into(),
            item: item.into(),
            part: part.into(),
            path: format!("Photos/2020/01/{item}-{part}"),
            size: 10,
            version: None,
        }
    }

    /// Only photos old enough, backed up whole, and none of: favourite, hidden, Shared
    /// Library, no date, Live without its video. Each that stays is counted once, by why.
    #[test]
    fn only_whole_backed_up_ordinary_photos_may_leave() {
        let mut lib = Library::in_memory().unwrap();
        let mut fav = photo("fav", 10);
        fav.favorite = true;
        let mut hidden = photo("hidden", 11);
        hidden.hidden = true;
        let undated = photo("undated", 0);
        let mut live_whole = photo("live-whole", 12);
        live_whole.kind = Kind::Live;
        let mut live_half = photo("live-half", 13);
        live_half.kind = Kind::Live;
        let mut gone = photo("gone", 14);
        gone.deleted = true;
        let mut video = photo("video", 15);
        video.kind = Kind::Video;
        lib.put_photos(&[
            photo("old", 16),
            photo("new", 999),
            photo("unbacked", 17),
            fav,
            hidden,
            undated,
            live_whole,
            live_half,
            gone,
            video,
        ])
        .unwrap();
        lib.backup_copied(&[
            copied("old", ORIGINAL),
            copied("new", ORIGINAL),
            copied("fav", ORIGINAL),
            copied("hidden", ORIGINAL),
            copied("undated", ORIGINAL),
            copied("live-whole", ORIGINAL),
            copied("live-whole", LIVE),
            copied("live-half", ORIGINAL),
            copied("video", ORIGINAL),
        ])
        .unwrap();
        let plan = lib.free_up_plan(100).unwrap();
        let ids: Vec<&str> = plan.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["old", "video", "live-whole"], "newest first");
        assert!(plan.items[1].video);
        assert_eq!(
            plan.stays,
            Stays {
                favourites: 1,
                hidden: 1,
                shared_library: 0,
                not_backed_up: 2,
                undated: 1,
            }
        );
    }

    /// A freed photo is remembered with its albums, so the album folders keep it after Apple
    /// lets it go and its album rows with it.
    #[test]
    fn a_freed_photo_keeps_its_albums() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_photos(&[photo("p", 10)]).unwrap();
        lib.put_album_items(&[("a1".into(), "p".into(), 1), ("a2".into(), "p".into(), 2)])
            .unwrap();
        lib.free_up_remember(&["p".into()], 500).unwrap();
        lib.forget_album_items(&[("a1".into(), "p".into()), ("a2".into(), "p".into())])
            .unwrap();
        assert_eq!(
            lib.freed_album_members().unwrap(),
            [
                ("a1".to_string(), "p".to_string()),
                ("a2".into(), "p".into())
            ]
        );
        assert_eq!(lib.free_up_files(&["p".into()]).unwrap(), vec![]);
    }
}
