//! iCloud Drive in the local library.
//!
//! Apple exposes Drive as a tree addressed by `drivewsid`, an identifier that encodes both
//! the kind and the zone. Folders are fetched one level at a time, so the library holds
//! whatever has been visited rather than a full mirror.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// The root every Drive browse starts from.
pub const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriveItem {
    pub drivewsid: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub extension: Option<String>,
    /// `FOLDER`, `APP_LIBRARY` or `FILE`.
    pub kind: String,
    pub size: Option<i64>,
    pub child_count: Option<i64>,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub etag: Option<String>,
    /// The share this item belongs to, when Apple says it is in one: shared with this account
    /// by someone else, or by this account with others. `None` for an ordinary item, and for
    /// anything stored before shares were read (old Windows placeholder identities included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<DriveShare>,
}

/// Which way a share goes, as Apple's `share` field says it (`TO_ME` / `FROM_ME`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ShareDirection {
    /// Someone else's, shared with this account.
    ToMe,
    /// This account's own, shared with others.
    FromMe,
}

/// What Apple sends about an item's share (icloud.com's Drive code, read 25 Nov 2026):
/// `share`, `shareID {shareName, recordName, zoneID {zoneName, ownerRecordName}}` and
/// `owner {firstName, lastName}`. Kept as Apple sent it, for the shared-folder support still
/// to come; today it only tells what may not be written.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DriveShare {
    /// `None` when Apple sent a `shareID` but not which way it goes.
    pub direction: Option<ShareDirection>,
    pub share_name: Option<String>,
    pub record_name: Option<String>,
    pub zone_name: Option<String>,
    pub owner_record_name: Option<String>,
    /// The owner's name as a person reads it ("Anna Lee"), when Apple gave it.
    pub owner: Option<String>,
}

/// Said when a folder shared with this account could not be listed. The window shows its own
/// translation of it (src/lib/ipc.ts, `messageOf`), so the two are kept word for word alike.
pub const SHARED_CANT_OPEN: &str = "Folders shared with you can\u{2019}t be opened in Skykeep yet.";
/// Said when a file shared with this account is asked for: Apple serves it from the owner's
/// side, a request not yet proven, so none is sent.
pub const SHARED_CANT_DOWNLOAD: &str =
    "Files shared with you can\u{2019}t be downloaded in Skykeep yet.";
/// Said for any change to an item shared with this account: refused before Apple is asked, so
/// nothing of someone else's is renamed, moved or deleted before Skykeep supports shares.
pub const SHARED_READ_ONLY: &str = "Items shared with you can\u{2019}t be changed in Skykeep yet.";

/// Whether an id names something in someone else's share, by the kind Apple puts at its head:
/// `SHARED_FILE` and `SHARED_FOLDER` (a share's root as the person it was shared with sees it),
/// `FILE_IN_SHARED_FOLDER` and `FOLDER_IN_SHARED_FOLDER` (Apple's own item types; icloud.com's
/// `isShareRoot` and `isInSharedFolder`).
#[must_use]
pub fn id_shared_with_me(drivewsid: &str) -> bool {
    matches!(
        drivewsid.split("::").next(),
        Some("SHARED_FILE" | "SHARED_FOLDER" | "FILE_IN_SHARED_FOLDER" | "FOLDER_IN_SHARED_FOLDER")
    )
}

impl DriveItem {
    /// True for anything someone else shared with this account, or inside such a share: read
    /// only here until shares are supported. Apple's `share: "TO_ME"`, or an id of one of the
    /// shared kinds.
    #[must_use]
    pub fn shared_with_me(&self) -> bool {
        // Apple's own word first: what this account shared (`FROM_ME`) is its own, whatever the
        // id's kind (an owner's view of its share is not proven to keep a plain `FOLDER::` id,
        // and the owner must never lose writing to its own folder). The id's kind only when
        // Apple says nothing.
        match self.share.as_ref().and_then(|s| s.direction) {
            Some(ShareDirection::FromMe) => false,
            Some(ShareDirection::ToMe) => true,
            None => id_shared_with_me(&self.drivewsid),
        }
    }

    /// True for anything that can be opened to reveal more items.
    #[must_use]
    pub fn is_container(&self) -> bool {
        self.kind == "FOLDER" || self.kind == "APP_LIBRARY"
    }

    /// The name as a person reads it, with the extension Apple keeps separate.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self.extension.as_deref().filter(|e| !e.is_empty()) {
            Some(ext) if !self.is_container() => format!("{}.{}", self.name, ext),
            _ => self.name.clone(),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    /// Replace the contents of one folder.
    ///
    /// Scoped to the folder rather than the whole tree: Drive is fetched a level at a time,
    /// so anything outside this folder is simply not part of this answer.
    pub fn put_drive_children(&mut self, parent_id: &str, items: &[DriveItem]) -> Result<()> {
        // Apple may not repeat a share on everything inside it: what a folder shared with this
        // account holds is taken to be in the same share, so it is never written to either.
        let inherited = self
            .drive_item(parent_id)?
            .and_then(|p| p.share)
            .filter(|s| s.direction == Some(ShareDirection::ToMe));
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO drive_items (drivewsid, parent_id, name, extension, kind, size,
                                          child_count, created_at, modified_at, etag, updated_at,
                                          share)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(drivewsid) DO UPDATE SET
                   parent_id = excluded.parent_id, name = excluded.name,
                   extension = excluded.extension, kind = excluded.kind,
                   size = excluded.size, child_count = excluded.child_count,
                   created_at = excluded.created_at, modified_at = excluded.modified_at,
                   etag = excluded.etag, updated_at = excluded.updated_at,
                   share = excluded.share
                 -- Stored as it was: nothing written, no index or trigger touched (a pass
                 -- storing everything again rewrote every row, each a synced write).
                 WHERE (drive_items.parent_id, drive_items.name, drive_items.extension, drive_items.kind, drive_items.size, drive_items.child_count, drive_items.created_at, drive_items.modified_at, drive_items.etag, drive_items.share)
                   IS NOT (excluded.parent_id, excluded.name, excluded.extension, excluded.kind, excluded.size, excluded.child_count, excluded.created_at, excluded.modified_at, excluded.etag, excluded.share)",
            )?;
            let stamp = now();
            for item in items {
                let share = item
                    .share
                    .as_ref()
                    .or(inherited.as_ref())
                    .and_then(|s| serde_json::to_string(s).ok());
                stmt.execute(params![
                    item.drivewsid,
                    parent_id,
                    item.name,
                    item.extension,
                    item.kind,
                    item.size,
                    item.child_count,
                    item.created_at,
                    item.modified_at,
                    item.etag,
                    stamp,
                    share
                ])?;
            }

            // Anything still filed under this folder that Apple no longer lists was moved or
            // deleted elsewhere.
            let keep: Vec<&str> = items.iter().map(|i| i.drivewsid.as_str()).collect();
            if keep.is_empty() {
                tx.execute(
                    "DELETE FROM drive_items WHERE parent_id = ?1",
                    params![parent_id],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM drive_items WHERE parent_id = ?1
                       AND drivewsid NOT IN (SELECT value FROM json_each(?2))",
                    params![parent_id, crate::id_list(&keep)],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Forget a folder Apple no longer has — it, and everything stored beneath it, at any
    /// depth. Only for a folder Apple has said is gone: nothing here asks Apple anything.
    ///
    /// A folder shared with this account is never forgotten this way, and `false` is answered:
    /// Skykeep does not yet send the share Apple needs to list one, so Apple's "not found" for
    /// it says nothing about whether it is there. Forgotten, it came back with the next listing
    /// of the folder above it, and was said to be deleted every time it was opened.
    pub fn forget_drive_folder(&mut self, drivewsid: &str) -> Result<bool> {
        if self.drive_shared_with_me(drivewsid)? {
            return Ok(false);
        }
        self.conn.execute(
            "WITH RECURSIVE gone(id) AS (
               SELECT ?1
               UNION SELECT d.drivewsid FROM drive_items d JOIN gone ON d.parent_id = gone.id
             )
             DELETE FROM drive_items WHERE drivewsid IN (SELECT id FROM gone)",
            params![drivewsid],
        )?;
        Ok(true)
    }

    /// Whether `drivewsid` is someone else's, shared with this account, or inside such a share:
    /// by its id, or as it is stored here (see [`DriveItem::shared_with_me`]).
    pub fn drive_shared_with_me(&self, drivewsid: &str) -> Result<bool> {
        if id_shared_with_me(drivewsid) {
            return Ok(true);
        }
        Ok(self
            .drive_item(drivewsid)?
            .is_some_and(|item| item.shared_with_me()))
    }

    /// What is inside a folder, folders first and then by name — how a file browser reads.
    ///
    /// Named the way Finder orders them: numbers as numbers and accents ignored, so "Scan 10"
    /// follows "Scan 9" and "Été" sits among the E's. SQL's `lower` can do neither.
    /// Files and folders whose name holds every word of `query`, across every folder this
    /// computer has listed: case and accents aside, as Finder finds them. At most `limit`.
    pub fn search_drive(&self, query: &str, limit: usize) -> Result<Vec<DriveItem>> {
        let words = crate::fold::words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM drive_items
             ORDER BY modified_at IS NULL, modified_at DESC"
        ))?;
        let rows = stmt.query_map([], item_from_row)?;
        let mut found = Vec::new();
        for row in rows {
            let item = row?;
            let name = format!("{}.{}", item.name, item.extension.as_deref().unwrap_or(""));
            if crate::fold::holds_all(&crate::fold::fold(&name), &words) {
                found.push(item);
                if found.len() >= limit {
                    break;
                }
            }
        }
        Ok(found)
    }

    pub fn drive_children(&self, parent_id: &str) -> Result<Vec<DriveItem>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM drive_items WHERE parent_id = ?1"
        ))?;
        let rows = stmt.query_map(params![parent_id], item_from_row)?;
        let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        items.sort_by(|a, b| {
            (!a.is_container())
                .cmp(&!b.is_container())
                .then_with(|| crate::fold::natural(&a.name, &b.name))
        });
        Ok(items)
    }

    /// What is inside a folder as `drive_children` has it, or `None` when this library was never
    /// told: nothing is stored under it and Apple's own count for it is not zero. Drive is
    /// fetched a folder at a time, so a folder never opened here has no items stored, and that
    /// is not the same as empty. Read as empty, File Explorer was told a folder never listed
    /// held nothing, and opening it failed ("The cloud operation is invalid") until it had been
    /// opened in Skykeep's own Drive first (#44).
    pub fn drive_children_known(&self, parent_id: &str) -> Result<Option<Vec<DriveItem>>> {
        let items = self.drive_children(parent_id)?;
        if !items.is_empty() {
            return Ok(Some(items));
        }
        let empty = self
            .drive_item(parent_id)?
            .is_some_and(|folder| folder.child_count == Some(0));
        Ok(empty.then_some(items))
    }

    /// One item by its identifier, for building a breadcrumb trail.
    pub fn drive_item(&self, drivewsid: &str) -> Result<Option<DriveItem>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM drive_items WHERE drivewsid = ?1"
        ))?;
        let mut rows = stmt.query_map(params![drivewsid], item_from_row)?;
        Ok(rows.next().transpose()?)
    }
}

/// The columns [`item_from_row`] reads, in its order.
pub(crate) const ITEM_COLUMNS: &str = "drivewsid, parent_id, name, extension, kind, size, \
    child_count, created_at, modified_at, etag, share";

/// One `drive_items` row selected as [`ITEM_COLUMNS`].
pub(crate) fn item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DriveItem> {
    let share: Option<String> = row.get(10)?;
    Ok(DriveItem {
        drivewsid: row.get(0)?,
        parent_id: row.get(1)?,
        name: row.get(2)?,
        extension: row.get(3)?,
        kind: row.get(4)?,
        size: row.get(5)?,
        child_count: row.get(6)?,
        created_at: row.get(7)?,
        modified_at: row.get(8)?,
        etag: row.get(9)?,
        share: share.and_then(|s| serde_json::from_str(&s).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, name: &str, kind: &str) -> DriveItem {
        DriveItem {
            drivewsid: id.into(),
            parent_id: None,
            name: name.into(),
            extension: if kind == "FILE" {
                Some("pdf".into())
            } else {
                None
            },
            kind: kind.into(),
            size: if kind == "FILE" { Some(1024) } else { None },
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: None,
            share: None,
        }
    }

    /// A folder listed again exactly as it was writes nothing (no row, no search index entry);
    /// any one field Apple changed is still written.
    #[test]
    fn listing_a_folder_again_as_it_is_writes_nothing() {
        let mut lib = Library::in_memory().unwrap();
        let mut a = item("D::a", "a", "FILE");
        a.size = Some(10);
        a.etag = Some("e1".into());
        let b = item("F::b", "b", "FOLDER");
        lib.put_drive_children(ROOT, &[a.clone(), b.clone()])
            .unwrap();
        let before = lib.conn.total_changes();
        lib.put_drive_children(ROOT, &[a.clone(), b.clone()])
            .unwrap();
        assert_eq!(lib.conn.total_changes(), before, "nothing written again");
        let mut changed = a.clone();
        changed.etag = Some("e2".into());
        lib.put_drive_children(ROOT, &[changed, b]).unwrap();
        assert!(
            lib.conn.total_changes() > before,
            "a new version is written"
        );
    }

    #[test]
    fn a_folder_never_listed_is_unknown_not_empty() {
        let mut lib = Library::in_memory().unwrap();
        let mut vault = item("F::vault", "Vault", "FOLDER");
        vault.child_count = Some(12);
        let mut bare = item("F::bare", "Bare", "FOLDER");
        bare.child_count = Some(0);
        let unsaid = item("F::unsaid", "Unsaid", "FOLDER");
        lib.put_drive_children(ROOT, &[vault, bare, unsaid])
            .unwrap();
        // Apple says twelve, none stored: never listed here.
        assert_eq!(lib.drive_children_known("F::vault").unwrap(), None);
        // Apple says none: empty, known.
        assert_eq!(lib.drive_children_known("F::bare").unwrap(), Some(vec![]));
        // No count given: not known either.
        assert_eq!(lib.drive_children_known("F::unsaid").unwrap(), None);
        // Listed: what it holds.
        lib.put_drive_children("F::vault", &[item("D::n", "note", "FILE")])
            .unwrap();
        let known = lib.drive_children_known("F::vault").unwrap().unwrap();
        assert_eq!(known.len(), 1);
        // The root, never listed: unknown.
        let fresh = Library::in_memory().unwrap();
        assert_eq!(fresh.drive_children_known(ROOT).unwrap(), None);
    }

    #[test]
    fn a_forgotten_folder_takes_everything_beneath_it_and_nothing_else() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(
            ROOT,
            &[item("F::a", "a", "FOLDER"), item("F::b", "b", "FOLDER")],
        )
        .unwrap();
        lib.put_drive_children(
            "F::a",
            &[item("F::a1", "a1", "FOLDER"), item("D::x", "x", "FILE")],
        )
        .unwrap();
        lib.put_drive_children("F::a1", &[item("D::deep", "deep", "FILE")])
            .unwrap();
        lib.put_drive_children("F::b", &[item("D::y", "y", "FILE")])
            .unwrap();
        lib.forget_drive_folder("F::a").unwrap();
        let names = |p: &str| {
            lib.drive_children(p)
                .unwrap()
                .into_iter()
                .map(|i| i.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(ROOT), ["b"]);
        assert!(names("F::a").is_empty() && names("F::a1").is_empty());
        assert!(lib.drive_item("D::deep").unwrap().is_none());
        assert_eq!(names("F::b"), ["y"]);
    }

    fn shared_to_me() -> DriveShare {
        DriveShare {
            direction: Some(ShareDirection::ToMe),
            record_name: Some("rec".into()),
            owner_record_name: Some("_owner".into()),
            owner: Some("Anna Lee".into()),
            ..DriveShare::default()
        }
    }

    /// What this account shared is its own to write, whatever kind of id Apple gives it: only
    /// Apple's `TO_ME`, or a shared kind of id with no direction said, is someone else's.
    #[test]
    fn what_i_shared_is_never_read_only() {
        let mut mine = item("SHARED_FOLDER::com.apple.CloudDocs::mine", "Mine", "FOLDER");
        mine.share = Some(DriveShare {
            direction: Some(ShareDirection::FromMe),
            ..DriveShare::default()
        });
        assert!(!mine.shared_with_me(), "FROM_ME wins over the id's kind");
        let theirs = item("SHARED_FOLDER::com.apple.CloudDocs::bo", "Band", "FOLDER");
        assert!(theirs.shared_with_me(), "a shared kind, no direction said");
        let mut to_me = item("FOLDER::com.apple.CloudDocs::anna", "Trip", "FOLDER");
        to_me.share = Some(shared_to_me());
        assert!(to_me.shared_with_me());
        assert!(!item("FOLDER::com.apple.CloudDocs::plain", "Plain", "FOLDER").shared_with_me());
    }

    /// A folder someone shared with this account that Apple wouldn't list is not one deleted
    /// elsewhere: it and what is stored in it stay, and the caller is told nothing was
    /// forgotten. Marked by Apple's `share` field or by a shared kind of id alike.
    #[test]
    fn a_folder_shared_with_me_is_never_forgotten() {
        let mut lib = Library::in_memory().unwrap();
        let mut anna = item("F::anna", "Trip", "FOLDER");
        anna.share = Some(shared_to_me());
        let bo = item("SHARED_FOLDER::com.apple.CloudDocs::bo", "Band", "FOLDER");
        let mut mine = item("F::mine", "Mine", "FOLDER");
        mine.share = Some(DriveShare {
            direction: Some(ShareDirection::FromMe),
            ..DriveShare::default()
        });
        lib.put_drive_children(ROOT, &[anna, bo.clone(), mine])
            .unwrap();
        lib.put_drive_children("F::anna", &[item("D::a1", "a1", "FILE")])
            .unwrap();

        assert!(!lib.forget_drive_folder("F::anna").unwrap());
        assert!(!lib.forget_drive_folder(&bo.drivewsid).unwrap());
        assert!(lib.drive_item("F::anna").unwrap().is_some());
        assert!(lib.drive_item(&bo.drivewsid).unwrap().is_some());
        assert_eq!(lib.drive_children("F::anna").unwrap().len(), 1);
        // This account's own shared folder is its own, and goes as any other.
        assert!(lib.forget_drive_folder("F::mine").unwrap());
        assert!(lib.drive_item("F::mine").unwrap().is_none());
    }

    /// The share is kept with the item, and what a shared folder holds is in its share too,
    /// whether or not Apple repeats it on each.
    #[test]
    fn a_share_is_stored_and_passed_to_what_is_inside() {
        let mut lib = Library::in_memory().unwrap();
        let mut anna = item("F::anna", "Trip", "FOLDER");
        anna.share = Some(shared_to_me());
        lib.put_drive_children(ROOT, &[anna.clone(), item("F::plain", "p", "FOLDER")])
            .unwrap();
        lib.put_drive_children("F::anna", &[item("D::in", "in", "FILE")])
            .unwrap();
        lib.put_drive_children("F::plain", &[item("D::out", "out", "FILE")])
            .unwrap();

        let stored = lib.drive_item("F::anna").unwrap().unwrap();
        assert_eq!(stored.share, anna.share);
        assert!(lib.drive_item("D::in").unwrap().unwrap().shared_with_me());
        assert!(lib.drive_shared_with_me("D::in").unwrap());
        assert!(!lib.drive_shared_with_me("D::out").unwrap());
        assert!(!lib.drive_shared_with_me("F::plain").unwrap());
        assert!(lib
            .drive_shared_with_me("FILE_IN_SHARED_FOLDER::z::never-listed")
            .unwrap());
        // Listed again unchanged, nothing is written again.
        let before = lib.conn.total_changes();
        lib.put_drive_children(ROOT, &[anna, item("F::plain", "p", "FOLDER")])
            .unwrap();
        assert_eq!(lib.conn.total_changes(), before);
    }

    #[test]
    fn folders_come_before_files_then_alphabetical() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(
            ROOT,
            &[
                item("f3", "notes", "FILE"),
                item("f1", "Zebra", "FOLDER"),
                item("f2", "apples", "FOLDER"),
                item("f4", "Archive", "FILE"),
            ],
        )
        .unwrap();

        let names: Vec<String> = lib
            .drive_children(ROOT)
            .unwrap()
            .iter()
            .map(|i| i.name.clone())
            .collect();
        assert_eq!(names, vec!["apples", "Zebra", "Archive", "notes"]);
    }

    /// Finder's order: "Scan 10" after "Scan 9", and "Été" among the E's rather than after Z.
    #[test]
    fn names_sort_the_way_finder_sorts_them() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(
            ROOT,
            &[
                item("a", "Scan 10", "FILE"),
                item("b", "Zoo", "FILE"),
                item("c", "Scan 9", "FILE"),
                item("d", "Été", "FILE"),
            ],
        )
        .unwrap();
        let names: Vec<String> = lib
            .drive_children(ROOT)
            .unwrap()
            .iter()
            .map(|i| i.name.clone())
            .collect();
        assert_eq!(names, vec!["Été", "Scan 9", "Scan 10", "Zoo"]);
    }

    #[test]
    fn an_extension_shows_on_files_but_not_folders() {
        assert_eq!(item("a", "report", "FILE").display_name(), "report.pdf");
        assert_eq!(item("b", "Downloads", "FOLDER").display_name(), "Downloads");
    }

    #[test]
    fn app_libraries_are_containers_too() {
        assert!(item("a", "Shortcuts", "APP_LIBRARY").is_container());
        assert!(item("b", "Downloads", "FOLDER").is_container());
        assert!(!item("c", "report", "FILE").is_container());
    }

    /// Refreshing one folder must not disturb another; Drive is fetched a level at a time.
    #[test]
    fn refreshing_a_folder_leaves_other_folders_alone() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(ROOT, &[item("a", "Downloads", "FOLDER")])
            .unwrap();
        lib.put_drive_children("a", &[item("a1", "inside", "FILE")])
            .unwrap();

        // Re-fetch the root; the child folder's contents must survive.
        lib.put_drive_children(ROOT, &[item("a", "Downloads", "FOLDER")])
            .unwrap();
        assert_eq!(lib.drive_children("a").unwrap().len(), 1);
    }

    #[test]
    fn an_item_removed_from_a_folder_disappears() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(ROOT, &[item("a", "One", "FILE"), item("b", "Two", "FILE")])
            .unwrap();
        lib.put_drive_children(ROOT, &[item("a", "One", "FILE")])
            .unwrap();
        assert_eq!(lib.drive_children(ROOT).unwrap().len(), 1);
    }

    #[test]
    fn an_item_can_be_looked_up_for_a_breadcrumb() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(ROOT, &[item("a", "Downloads", "FOLDER")])
            .unwrap();
        let found = lib.drive_item("a").unwrap().unwrap();
        assert_eq!(found.name, "Downloads");
        assert_eq!(found.parent_id.as_deref(), Some(ROOT));
        assert!(lib.drive_item("nope").unwrap().is_none());
    }
}

#[cfg(test)]
mod many_tests {
    use super::*;

    /// A folder past SQLite's 32,766 parameters (a camera's dump): stored, and pruned, in one
    /// statement. One parameter per kept item failed its store on every pass.
    #[test]
    fn a_folder_of_forty_thousand_files_is_stored_and_pruned() {
        let mut lib = Library::in_memory().unwrap();
        let item = |i: usize| DriveItem {
            drivewsid: format!("FILE::{i}"),
            parent_id: Some("root".into()),
            name: format!("IMG_{i}.jpg"),
            extension: Some("jpg".into()),
            kind: "FILE".into(),
            size: Some(1),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: None,
            share: None,
        };
        let mut items: Vec<DriveItem> = (0..40_000).map(item).collect();
        lib.put_drive_children("root", &items).unwrap();
        items.truncate(39_999);
        lib.put_drive_children("root", &items).unwrap();
        let kept = lib.drive_children("root").unwrap();
        assert_eq!(kept.len(), 39_999);
        assert!(kept.iter().all(|i| i.drivewsid != "FILE::39999"));
    }
}
