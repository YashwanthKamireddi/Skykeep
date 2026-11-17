//! Notes and their folders in the local library.

use crate::{Library, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteFolder {
    pub guid: String,
    pub title: String,
    pub sort_order: i64,
    /// The folder this one sits inside, where it sits inside one at all.
    #[serde(default)]
    pub parent_guid: Option<String>,
}

/// Apple's identifier for the folder deleted notes go to.
///
/// The record name, not the title: the title is translated, so matching on "Recently
/// Deleted" would work in English and quietly fail everywhere else.
pub const TRASH_FOLDER: &str = "TrashFolder-CloudKit";

impl NoteFolder {
    /// Whether this is where deleted notes wait before they are really gone.
    #[must_use]
    pub fn is_trash(&self) -> bool {
        self.guid == TRASH_FOLDER
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub guid: String,
    pub folder_guid: Option<String>,
    pub title: String,
    pub snippet: Option<String>,
    pub body: Option<String>,
    /// Locked by its owner, so Apple holds it encrypted and there is nothing to show.
    pub locked: bool,
    pub pinned: bool,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    /// Whether saving an edit would preserve the note.
    ///
    /// Apple's document carries version vectors and merge state around the text. Where that
    /// document can be reproduced byte for byte, an edit is a safe surgical swap; where it
    /// cannot, writing would flatten everything else in it, so the note stays read-only and
    /// the interface says why.
    #[serde(default = "yes")]
    pub editable: bool,
    pub etag: Option<String>,
    /// How the body is set — Apple's attribute runs, as JSON — read from the same record as
    /// the body, so formatting shows without the network. Absent where none was read.
    #[serde(default, skip_serializing)]
    pub runs: Option<String>,
}

fn yes() -> bool {
    true
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    pub fn put_note_folders(&mut self, folders: &[NoteFolder]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO note_folders (guid, title, sort_order, parent_guid, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title, sort_order = excluded.sort_order,
                   parent_guid = excluded.parent_guid,
                   updated_at = excluded.updated_at
                 -- Stored as it was: nothing written, no index or trigger touched (a pass
                 -- storing everything again rewrote every row, each a synced write).
                 WHERE (note_folders.title, note_folders.sort_order, note_folders.parent_guid)
                   IS NOT (excluded.title, excluded.sort_order, excluded.parent_guid)",
            )?;
            let stamp = now();
            for f in folders {
                stmt.execute(params![f.guid, f.title, f.sort_order, f.parent_guid, stamp])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Every stored note's (pinned, locked), by guid — what a pass that carried only part of
    /// a note's state leaves standing.
    pub fn note_flags(&self) -> Result<std::collections::HashMap<String, (bool, bool)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT guid, pinned, locked FROM notes")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, i64>(1)? != 0, row.get::<_, i64>(2)? != 0),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Pin or unpin a stored note. A note this library does not hold is left alone.
    pub fn set_note_pinned(&mut self, guid: &str, pinned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE notes SET pinned = ?2 WHERE guid = ?1",
            params![guid, i64::from(pinned)],
        )?;
        Ok(())
    }

    /// File a stored note in a folder (`None`: no folder) — moved or deleted here, before
    /// Apple has it. A note this library does not hold is left alone.
    pub fn set_note_folder(&mut self, guid: &str, folder_guid: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE notes SET folder_guid = ?2 WHERE guid = ?1",
            params![guid, folder_guid],
        )?;
        Ok(())
    }

    /// A note's text, title and snippet as just saved, touching nothing else — not its
    /// formatting, its folder or its version, which the next pass brings from Apple.
    pub fn set_note_text(
        &mut self,
        guid: &str,
        body: &str,
        title: &str,
        snippet: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE notes SET body = ?2, title = ?3, snippet = ?4 WHERE guid = ?1",
            params![guid, body, title, snippet],
        )?;
        Ok(())
    }

    /// A note edited here, now: its time edited moved on (never back), so it goes to the top
    /// of a list ordered by Date Edited at once, as the phone puts it, rather than at the next
    /// pass.
    pub fn touch_note(&mut self, guid: &str, at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE notes SET modified_at = MAX(COALESCE(modified_at, 0), ?2) WHERE guid = ?1",
            params![guid, at],
        )?;
        Ok(())
    }

    /// A note's formatting as just saved — Apple's runs, as JSON — so a read before the next
    /// pass finds runs that describe the text it was just given.
    pub fn set_note_runs(&mut self, guid: &str, runs: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE notes SET runs = ?2 WHERE guid = ?1",
            params![guid, runs],
        )?;
        Ok(())
    }

    /// The note's attribute runs as stored, JSON, if any were read.
    pub fn note_runs(&self, guid: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT runs FROM notes WHERE guid = ?1",
                params![guid],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Each note's first picture — a photo, a drawing, or a scan or PDF Apple keeps a
    /// rendering of — as `(note, attachment)`, for the small picture the list shows beside a
    /// note, as Notes on the Mac does. Read from what this computer already knows: the note's
    /// stored formatting says where its attachments sit, and an attachment's kind is known
    /// once it has been drawn (`notes.attachment.<id>`, kept by `note_attachment_info`). One
    /// whose kind is not known yet is passed over, never asked of Apple from here. Locked
    /// notes show none: their pictures are not shown either.
    pub fn note_first_pictures(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT n.guid, j.value ->> '$.attachment'
             FROM notes n, json_each(n.runs) j
             JOIN app_state s ON s.key = 'notes.attachment.' || (j.value ->> '$.attachment')
             WHERE n.locked = 0
               AND n.runs LIKE '%\"attachment\"%'
               AND json_valid(n.runs)
               AND json_valid(s.value)
               AND (s.value ->> '$.kind' IN ('image', 'drawing')
                    OR (s.value ->> '$.kind' = 'document' AND s.value ->> '$.hasPreview'))
             ORDER BY n.guid, j.key",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out: Vec<(String, String)> = Vec::new();
        for row in rows {
            let (note, attachment) = row?;
            if out.last().is_none_or(|(n, _)| *n != note) {
                out.push((note, attachment));
            }
        }
        Ok(out)
    }

    pub fn put_notes(&mut self, notes: &[Note]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO notes (guid, folder_guid, title, snippet, body, locked, pinned,
                                    created_at, modified_at, editable, etag, updated_at, runs)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(guid) DO UPDATE SET
                   folder_guid = excluded.folder_guid, title = excluded.title,
                   snippet = excluded.snippet,
                   -- A body that did not arrive — a long note's file whose download failed —
                   -- is not an empty note: what is stored stays, formatting and all, until
                   -- one arrives. A note locked since is the exception: its text goes.
                   body = CASE WHEN excluded.locked THEN NULL ELSE COALESCE(excluded.body, body) END,
                   runs = CASE WHEN excluded.locked THEN NULL
                               WHEN excluded.body IS NULL THEN runs ELSE excluded.runs END,
                   editable = CASE WHEN excluded.body IS NULL AND NOT excluded.locked AND body IS NOT NULL
                                   THEN editable ELSE excluded.editable END,
                   locked = excluded.locked, pinned = excluded.pinned,
                   created_at = excluded.created_at, modified_at = excluded.modified_at,
                   etag = excluded.etag, updated_at = excluded.updated_at
                 -- Stored as it was: nothing written and no words indexed again. Fifty
                 -- thousand notes stored again as they were took 27 s, rewriting the index.
                 WHERE (notes.folder_guid, notes.title, notes.snippet, notes.locked,
                        notes.pinned, notes.created_at, notes.modified_at, notes.etag,
                        notes.body, notes.runs, notes.editable)
                   IS NOT (excluded.folder_guid, excluded.title, excluded.snippet,
                        excluded.locked, excluded.pinned, excluded.created_at,
                        excluded.modified_at, excluded.etag,
                        CASE WHEN excluded.locked THEN NULL ELSE COALESCE(excluded.body, notes.body) END,
                        CASE WHEN excluded.locked THEN NULL
                             WHEN excluded.body IS NULL THEN notes.runs ELSE excluded.runs END,
                        CASE WHEN excluded.body IS NULL AND NOT excluded.locked AND notes.body IS NOT NULL
                             THEN notes.editable ELSE excluded.editable END)",
            )?;
            let stamp = now();
            for n in notes {
                stmt.execute(params![
                    n.guid,
                    n.folder_guid,
                    n.title,
                    n.snippet,
                    n.body,
                    i64::from(n.locked),
                    i64::from(n.pinned),
                    n.created_at,
                    n.modified_at,
                    i64::from(n.editable),
                    n.etag,
                    stamp,
                    n.runs
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Keep only these notes, dropping everything else.
    ///
    /// The same floor a full reminder sync has: a tombstone arrives once, so a sync that
    /// consumed one before it was understood would leave a deleted note on screen forever —
    /// and saving one of those writes it back to iCloud.
    pub fn retain_notes(&mut self, keep: &[String]) -> Result<usize> {
        let tx = self.conn.savepoint()?;
        let removed = {
            let mut known: std::collections::HashSet<&str> =
                keep.iter().map(String::as_str).collect();
            let mut stmt = tx.prepare("SELECT guid FROM notes")?;
            let all: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut delete = tx.prepare("DELETE FROM notes WHERE guid = ?1")?;
            let mut count = 0;
            for guid in &all {
                if !known.remove(guid.as_str()) {
                    delete.execute(params![guid])?;
                    count += 1;
                }
            }
            count
        };
        tx.commit()?;
        Ok(removed)
    }

    pub fn forget_notes(&mut self, note_guids: &[String]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare("DELETE FROM notes WHERE guid = ?1")?;
            for guid in note_guids {
                stmt.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// One note's text, whole.
    ///
    /// Fetched for the note being read rather than for every note in the list, which is what
    /// keeps opening a folder of five thousand cheap.
    pub fn note_body(&self, guid: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT body, title, snippet FROM notes WHERE guid = ?1")?;
        let mut rows = stmt.query(params![guid])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let body: Option<String> = row.get(0)?;
        if body.is_some() {
            return Ok(body);
        }

        // No body stored. That is either a note with nothing in it, or one whose text could
        // not be read — and the two must not be confused, because an editor opened over the
        // second shows an empty box, and the first keystroke saves that emptiness over
        // whatever was really there. A five-line note became a single letter that way.
        //
        // A note with a name or a line of preview has something in it, so no body means it
        // was not read. Answering "not known" keeps the editor shut.
        let title: Option<String> = row.get(1)?;
        let snippet: Option<String> = row.get(2)?;
        let has_content = [title, snippet]
            .into_iter()
            .flatten()
            .any(|s| !s.trim().is_empty());
        Ok(if has_content {
            None
        } else {
            Some(String::new())
        })
    }

    /// How many notes each folder holds, by folder guid, and in all of them together.
    ///
    /// The sidebar puts a count beside every folder, as Notes does. "All" leaves Recently
    /// Deleted out — a note waiting to be purged is not one of your notes — and the trash is
    /// counted on its own line.
    pub fn note_counts(&self) -> Result<(i64, Vec<(String, i64)>)> {
        let mut stmt = self.conn.prepare(
            "SELECT coalesce(folder_guid, ''), count(*) FROM notes GROUP BY folder_guid",
        )?;
        let per: Vec<(String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let all = per
            .iter()
            .filter(|(guid, _)| guid != TRASH_FOLDER)
            .map(|(_, n)| n)
            .sum();
        Ok((all, per))
    }

    /// Forget folders Apple has deleted. A note still pointing at one is left unfiled here
    /// rather than lost; the next pass files it wherever Apple has it.
    pub fn remove_note_folders(&mut self, guids: &[String]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        for guid in guids {
            tx.execute(
                "UPDATE notes SET folder_guid = NULL WHERE folder_guid = ?1",
                params![guid],
            )?;
            tx.execute(
                "UPDATE note_folders SET parent_guid = NULL WHERE parent_guid = ?1",
                params![guid],
            )?;
            tx.execute("DELETE FROM note_folders WHERE guid = ?1", params![guid])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn note_folders(&self) -> Result<Vec<NoteFolder>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, title, sort_order, parent_guid
             FROM note_folders ORDER BY sort_order, title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(NoteFolder {
                guid: row.get(0)?,
                title: row.get(1)?,
                sort_order: row.get(2)?,
                parent_guid: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Notes in a folder, pinned first and then most recently changed — Apple's order.
    ///
    /// Passing `None` is the "All Notes" view, which in Apple's app means every note that
    /// has not been deleted. Including the deleted ones put them in both places at once.
    /// The notes in a folder, without their bodies.
    ///
    /// A list of notes shows a title, a line of preview and a date. Carrying every body along
    /// with that is what the plan calls an unbounded payload: five thousand ordinary notes
    /// come to nearly ten megabytes, and all of it crosses into the window on every refresh to
    /// draw text nobody reads. The body of the one note being read is fetched on its own.
    ///
    /// The preview is a truncation, so it is put in `snippet` — which is only ever displayed —
    /// and never in `body`, which is what an editor would save back. A body that came back
    /// short and got written would take the rest of the note with it.
    /// One note by record name, wherever it is filed — Recently Deleted too — as `notes_in`
    /// gives it: no body. One row by key, where finding it through `notes_in` read every note.
    pub fn note(&self, guid: &str) -> Result<Option<Note>> {
        Ok(self
            .conn
            .query_row(
                "SELECT guid, folder_guid, title,
                        coalesce(snippet, substr(body, 1, 200)), locked, pinned,
                        created_at, modified_at, editable, etag
                 FROM notes WHERE guid = ?1",
                params![guid],
                |row| {
                    Ok(Note {
                        guid: row.get(0)?,
                        folder_guid: row.get(1)?,
                        title: row.get(2)?,
                        snippet: row.get(3)?,
                        body: None,
                        locked: row.get::<_, i64>(4)? != 0,
                        pinned: row.get::<_, i64>(5)? != 0,
                        created_at: row.get(6)?,
                        modified_at: row.get(7)?,
                        editable: row.get::<_, i64>(8)? != 0,
                        etag: row.get(9)?,
                        runs: None,
                    })
                },
            )
            .optional()?)
    }

    pub fn notes_in(&self, folder_guid: Option<&str>) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, folder_guid, title,
                    coalesce(snippet, substr(body, 1, 200)), locked, pinned,
                    created_at, modified_at, editable, etag
             FROM notes
             WHERE (?1 IS NULL AND (folder_guid IS NULL OR folder_guid != ?2))
                OR folder_guid = ?1
             ORDER BY pinned DESC, modified_at DESC, title",
        )?;
        let rows = stmt.query_map(params![folder_guid, TRASH_FOLDER], |row| {
            Ok(Note {
                guid: row.get(0)?,
                folder_guid: row.get(1)?,
                title: row.get(2)?,
                snippet: row.get(3)?,
                // Never a truncation. Absent, so nothing can save a short one back.
                body: None,
                locked: row.get::<_, i64>(4)? != 0,
                pinned: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
                modified_at: row.get(7)?,
                editable: row.get::<_, i64>(8)? != 0,
                etag: row.get(9)?,
                runs: None,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Notes whose title or text holds every word of `query`, in the list's own order.
    ///
    /// Looked up in the search index (migration 45): every word has to begin a word of the
    /// note, in any order, case and accents aside — "cafe" finds "CAFÉ", and a Telugu word
    /// stays one word — as Apple's own search finds notes. Recently Deleted is left out, as it
    /// is on the phone. Bodies are never read here, let alone returned.
    pub fn search_notes(&self, query: &str) -> Result<Vec<Note>> {
        let Some(question) = crate::search::match_all(query) else {
            return Ok(Vec::new());
        };
        let words = crate::search::folded_words(query);
        let mut stmt = self.conn.prepare_cached(
            "SELECT guid, folder_guid, title,
                    coalesce(snippet, substr(body, 1, 200)), locked, pinned,
                    created_at, modified_at, editable, etag, body
             FROM notes
             WHERE guid IN (SELECT d.key FROM search_text
                            JOIN search_docs d ON d.id = search_text.rowid
                            WHERE search_text MATCH ?1 AND d.kind = 'note')
             ORDER BY pinned DESC, modified_at DESC, title",
        )?;
        let rows = stmt.query_map(params![question], |row| {
            let title: String = row.get(2)?;
            let snippet: Option<String> = row.get(3)?;
            let locked = row.get::<_, i64>(4)? != 0;
            // Each result's preview is where its words are, as in Notes: a note found by a
            // word far down its text shows that part of it, not its first line. Never for a
            // locked note, whose text is not shown.
            let snippet = if locked {
                snippet
            } else {
                let body: Option<String> = row.get(10)?;
                crate::search::note_line(&title, body, snippet.clone(), &words)
                    .0
                    .or(snippet)
            };
            Ok(Note {
                guid: row.get(0)?,
                folder_guid: row.get(1)?,
                title,
                snippet,
                body: None,
                locked,
                pinned: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
                modified_at: row.get(7)?,
                editable: row.get::<_, i64>(8)? != 0,
                etag: row.get(9)?,
                runs: None,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn note(guid: &str, folder: &str, title: &str, body: &str) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: Some(folder.into()),
            title: title.into(),
            snippet: None,
            body: Some(body.into()),
            locked: false,
            editable: true,
            pinned: false,
            created_at: Some(1),
            modified_at: Some(1),
            etag: None,
            runs: None,
        }
    }

    /// A pass in which a long note's text failed to download says nothing about its text: the
    /// stored body and formatting stay, and it stays editable. Locked since, its text goes.
    #[test]
    fn a_body_that_did_not_arrive_keeps_the_stored_one() {
        let mut lib = library();
        let mut n = note("N", "F", "Long", "the whole long text");
        n.runs = Some("[]".into());
        lib.put_notes(&[n.clone()]).unwrap();
        let mut empty = n.clone();
        empty.body = None;
        empty.runs = None;
        empty.editable = false;
        lib.put_notes(&[empty.clone()]).unwrap();
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("the whole long text")
        );
        assert_eq!(lib.note_runs("N").unwrap().as_deref(), Some("[]"));
        assert!(lib.note("N").unwrap().unwrap().editable);
        empty.locked = true;
        lib.put_notes(&[empty]).unwrap();
        assert_eq!(
            lib.note_body("N").unwrap(),
            None,
            "a locked note keeps no text here"
        );
    }

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_note_folders(&[
            NoteFolder {
                guid: "F".into(),
                title: "Notes".into(),
                sort_order: 0,
                parent_guid: None,
            },
            NoteFolder {
                guid: TRASH_FOLDER.into(),
                title: "Recently Deleted".into(),
                sort_order: 1,
                parent_guid: None,
            },
        ])
        .unwrap();
        lib.put_notes(&[
            note("A", "F", "Curtains", "Bathroom accessories like mirrors"),
            note("B", "F", "Café list", "Flat white, then a croissant"),
            note("C", "F", "తెలుగు", "Notes written in Telugu"),
            note(
                "D",
                TRASH_FOLDER,
                "Curtains old",
                "deleted draft about mirrors",
            ),
        ])
        .unwrap();
        lib
    }

    fn found(lib: &Library, q: &str) -> Vec<String> {
        lib.search_notes(q)
            .unwrap()
            .into_iter()
            .map(|n| n.guid)
            .collect()
    }

    /// A note found by a word deep in its text shows that part of it under its title.
    #[test]
    fn a_result_previews_where_its_words_are() {
        let mut lib = library();
        lib.put_notes(&[note(
            "E",
            "F",
            "Trip",
            "Trip\nPack the bags\nBook the train\nWater the ferns before leaving",
        )])
        .unwrap();
        let hit = lib.search_notes("ferns").unwrap();
        assert_eq!(
            hit[0].snippet.as_deref(),
            Some("Water the ferns before leaving")
        );
        // Found by its title alone, the preview is its text from the top.
        let hit = lib.search_notes("trip").unwrap();
        assert!(hit[0]
            .snippet
            .as_deref()
            .unwrap()
            .starts_with("Pack the bags"));
    }

    #[test]
    fn a_word_in_the_body_finds_the_note() {
        assert_eq!(found(&library(), "mirrors"), vec!["A"]);
    }

    /// SQLite folds ASCII only; this is the case that made matching live in Rust.
    #[test]
    fn case_is_folded_beyond_ascii() {
        assert_eq!(found(&library(), "CAFÉ"), vec!["B"]);
        assert_eq!(found(&library(), "తెలుగు"), vec!["C"]);
    }

    /// Accents are ignored, as on the phone: "cafe" finds "Café".
    #[test]
    fn accents_are_ignored() {
        assert_eq!(found(&library(), "cafe"), vec!["B"]);
    }

    /// Every word must be there, in any order.
    #[test]
    fn every_word_has_to_appear() {
        assert_eq!(found(&library(), "croissant flat"), vec!["B"]);
        assert!(found(&library(), "croissant mirrors").is_empty());
    }

    /// Recently Deleted is not searched, as on the phone.
    #[test]
    fn deleted_notes_are_not_found() {
        assert!(!found(&library(), "draft").contains(&"D".to_string()));
    }

    #[test]
    fn nothing_typed_finds_nothing_and_nothing_carries_a_body() {
        assert!(found(&library(), "   ").is_empty());
        assert!(library()
            .search_notes("mirrors")
            .unwrap()
            .iter()
            .all(|n| n.body.is_none()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(guid: &str, title: &str) -> NoteFolder {
        NoteFolder {
            guid: guid.into(),
            title: title.into(),
            sort_order: 0,
            parent_guid: None,
        }
    }

    /// Apple Notes nests folders, and the link has to survive a round trip through the
    /// library or somebody's filing comes back as one flat list.
    /// Folders stored again exactly as they are write nothing; a renamed one is written.
    #[test]
    fn storing_folders_again_as_they_are_writes_nothing() {
        let mut lib = Library::in_memory().unwrap();
        let folders = [folder("F1", "Notes"), folder("F2", "Work")];
        lib.put_note_folders(&folders).unwrap();
        let before = lib.conn.total_changes();
        lib.put_note_folders(&folders).unwrap();
        assert_eq!(lib.conn.total_changes(), before, "nothing written again");
        lib.put_note_folders(&[folder("F1", "Notes"), folder("F2", "Work 2026")])
            .unwrap();
        assert!(lib.conn.total_changes() > before, "a rename is written");
    }
    #[test]
    fn a_folder_remembers_the_one_it_sits_inside() {
        let mut lib = Library::in_memory().unwrap();
        let mut inner = folder("f2", "Semester 5");
        inner.parent_guid = Some("f1".into());
        lib.put_note_folders(&[folder("f1", "University"), inner])
            .unwrap();

        let stored = lib.note_folders().unwrap();
        let nested = stored.iter().find(|f| f.guid == "f2").unwrap();
        assert_eq!(nested.parent_guid.as_deref(), Some("f1"));
        assert_eq!(
            stored.iter().find(|f| f.guid == "f1").unwrap().parent_guid,
            None,
            "a top-level folder sits inside nothing"
        );
    }

    /// A folder moved out to the top level must not keep its old parent.
    #[test]
    fn a_folder_moved_to_the_top_stops_naming_a_parent() {
        let mut lib = Library::in_memory().unwrap();
        let mut inner = folder("f2", "Semester 5");
        inner.parent_guid = Some("f1".into());
        lib.put_note_folders(&[folder("f1", "University"), inner.clone()])
            .unwrap();

        inner.parent_guid = None;
        lib.put_note_folders(&[folder("f1", "University"), inner])
            .unwrap();
        assert_eq!(
            lib.note_folders()
                .unwrap()
                .iter()
                .find(|f| f.guid == "f2")
                .unwrap()
                .parent_guid,
            None
        );
    }

    fn note(guid: &str, folder: Option<&str>, pinned: bool, modified: i64) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: folder.map(ToString::to_string),
            title: guid.into(),
            snippet: None,
            body: Some("body".into()),
            locked: false,
            pinned,
            created_at: Some(1),
            modified_at: Some(modified),
            editable: true,
            etag: None,
            runs: None,
        }
    }

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_note_folders(&[folder("f1", "Notes"), folder("f2", "Recently Deleted")])
            .unwrap();
        lib
    }

    /// Apple pins to the top, then orders by most recently changed.
    #[test]
    fn pinned_notes_come_first_then_most_recent() {
        let mut lib = seeded();
        lib.put_notes(&[
            note("old", Some("f1"), false, 100),
            note("newest", Some("f1"), false, 300),
            note("pinned-old", Some("f1"), true, 50),
        ])
        .unwrap();

        let order: Vec<String> = lib
            .notes_in(Some("f1"))
            .unwrap()
            .into_iter()
            .map(|n| n.guid)
            .collect();
        assert_eq!(order, vec!["pinned-old", "newest", "old"]);
    }

    #[test]
    fn every_note_is_returned_when_no_folder_is_given() {
        let mut lib = seeded();
        lib.put_notes(&[
            note("a", Some("f1"), false, 1),
            note("b", Some("f2"), false, 2),
        ])
        .unwrap();
        assert_eq!(lib.notes_in(None).unwrap().len(), 2);
        assert_eq!(lib.notes_in(Some("f1")).unwrap().len(), 1);
    }

    /// A note whose folder disappears must survive, unfiled — losing the note would be worse.
    #[test]
    fn a_note_outlives_its_folder() {
        let mut lib = seeded();
        lib.put_notes(&[note("a", Some("f1"), false, 1)]).unwrap();
        lib.conn()
            .execute("DELETE FROM note_folders WHERE guid = 'f1'", [])
            .unwrap();

        let all = lib.notes_in(None).unwrap();
        assert_eq!(all.len(), 1, "the note must not go with the folder");
        assert!(all[0].folder_guid.is_none(), "it becomes unfiled");
    }

    #[test]
    fn re_syncing_updates_rather_than_duplicates() {
        let mut lib = seeded();
        lib.put_notes(&[note("a", Some("f1"), false, 1)]).unwrap();
        let mut changed = note("a", Some("f1"), false, 2);
        changed.title = "Renamed".into();
        lib.put_notes(&[changed]).unwrap();

        let all = lib.notes_in(None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "Renamed");
    }

    #[test]
    fn a_locked_note_is_recorded_as_locked() {
        let mut lib = seeded();
        let mut locked = note("secret", Some("f1"), false, 1);
        locked.locked = true;
        locked.body = None;
        lib.put_notes(&[locked]).unwrap();
        assert!(lib.notes_in(None).unwrap()[0].locked);
    }
}

/// Reading a lot of notes without carrying every word of them.
#[cfg(test)]
mod body_tests {
    use super::*;

    fn stored() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_note_folders(&[NoteFolder {
            guid: "F1".into(),
            title: "Notes".into(),
            sort_order: 0,
            parent_guid: None,
        }])
        .unwrap();
        lib.put_notes(&[
            Note {
                guid: "N1".into(),
                folder_guid: Some("F1".into()),
                title: "Long one".into(),
                snippet: None,
                body: Some("x".repeat(5_000)),
                locked: false,
                editable: true,
                pinned: false,
                created_at: Some(1),
                modified_at: Some(2),
                etag: None,
                runs: None,
            },
            Note {
                guid: "N2".into(),
                // A note with nothing in it has no name either — a name with no body means
                // the text was not read, which is a different thing and answered differently.
                folder_guid: Some("F1".into()),
                title: String::new(),
                snippet: None,
                body: None,
                locked: false,
                editable: true,
                pinned: false,
                created_at: Some(1),
                modified_at: Some(1),
                etag: None,
                runs: None,
            },
        ])
        .unwrap();
        lib
    }

    /// The danger this guards: a body that came back short and got saved would take the rest
    /// of the note with it. The list carries no body at all, so nothing can.
    #[test]
    fn a_listed_note_carries_no_body_to_save_back() {
        for note in stored().notes_in(None).unwrap() {
            assert_eq!(
                note.body, None,
                "{} must not carry a body into a list",
                note.title
            );
        }
    }

    /// A preview still has to appear, and it is a truncation — so it goes where only display
    /// reads it.
    #[test]
    fn a_note_with_no_snippet_still_previews() {
        let listed = stored().notes_in(None).unwrap();
        let long = listed.iter().find(|n| n.guid == "N1").unwrap();
        assert_eq!(long.snippet.as_deref().map(str::len), Some(200));
    }

    /// The note being read gets its text whole.
    #[test]
    fn the_body_is_fetched_in_full_for_one_note() {
        let lib = stored();
        assert_eq!(lib.note_body("N1").unwrap().map(|b| b.len()), Some(5_000));
    }

    /// A note with nothing in it is not the same as no such note, and the difference decides
    /// whether an editor may open at all.
    #[test]
    fn an_empty_note_and_a_missing_one_are_told_apart() {
        let lib = stored();
        assert_eq!(lib.note_body("N2").unwrap(), Some(String::new()));
        assert_eq!(lib.note_body("nothing-like-this").unwrap(), None);
    }
}

/// Telling an empty note from one whose text could not be read.
#[cfg(test)]
mod unreadable_tests {
    use super::*;

    fn note(guid: &str, title: &str, snippet: Option<&str>, body: Option<&str>) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: None,
            title: title.into(),
            snippet: snippet.map(ToString::to_string),
            body: body.map(ToString::to_string),
            locked: false,
            editable: true,
            pinned: false,
            created_at: Some(1),
            modified_at: Some(1),
            etag: None,
            runs: None,
        }
    }

    /// The difference decides whether an editor opens. Over a note whose text was not read,
    /// an empty box is shown and the first keystroke saves that emptiness over whatever was
    /// really there — which is how a five-line note became a single letter.
    #[test]
    fn a_note_whose_text_was_not_read_answers_that_it_is_not_known() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_notes(&[
            note("EMPTY", "", None, None),
            note("UNREAD", "Shopping list", Some("Milk"), None),
            note(
                "READ",
                "Shopping list",
                Some("Milk"),
                Some("Shopping list\nMilk"),
            ),
        ])
        .unwrap();

        assert_eq!(
            lib.note_body("EMPTY").unwrap(),
            Some(String::new()),
            "a note with nothing in it is empty, and may be typed into"
        );
        assert_eq!(
            lib.note_body("UNREAD").unwrap(),
            None,
            "a note with a name but no text was not read, and must not be typed into"
        );
        assert_eq!(
            lib.note_body("READ").unwrap().as_deref(),
            Some("Shopping list\nMilk")
        );
        assert_eq!(lib.note_body("no-such-note").unwrap(), None);
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;

    fn note(guid: &str, folder: &str) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: Some(folder.into()),
            title: guid.into(),
            snippet: None,
            body: None,
            locked: false,
            pinned: false,
            created_at: None,
            modified_at: None,
            editable: true,
            etag: None,
            runs: None,
        }
    }

    #[test]
    fn every_folder_is_counted_and_all_leaves_out_the_trash() {
        let mut lib = Library::in_memory().unwrap();
        let folder = |guid: &str| NoteFolder {
            guid: guid.into(),
            title: guid.into(),
            sort_order: 0,
            parent_guid: None,
        };
        lib.put_note_folders(&[
            folder("DefaultFolder-CloudKit"),
            folder("Folder/work"),
            folder(TRASH_FOLDER),
        ])
        .unwrap();
        lib.put_notes(&[
            note("a", "DefaultFolder-CloudKit"),
            note("b", "DefaultFolder-CloudKit"),
            note("c", "Folder/work"),
            note("d", TRASH_FOLDER),
        ])
        .unwrap();
        let (all, per) = lib.note_counts().unwrap();
        assert_eq!(all, 3);
        let of = |g: &str| per.iter().find(|(f, _)| f == g).map(|(_, n)| *n);
        assert_eq!(of("DefaultFolder-CloudKit"), Some(2));
        assert_eq!(of("Folder/work"), Some(1));
        assert_eq!(of(TRASH_FOLDER), Some(1));
    }

    /// The list's picture for a note is its first picture whose kind is known here — a file
    /// before it, or one not drawn yet, is passed over — and none for a note without one, a
    /// film, or a locked note.
    #[test]
    fn a_notes_first_known_picture_is_its_picture() {
        let mut lib = Library::in_memory().unwrap();
        let with = |guid: &str, ids: &[&str], locked: bool| {
            let mut runs = vec![r#"{"length":5}"#.to_string()];
            runs.extend(
                ids.iter()
                    .map(|id| format!(r#"{{"length":1,"attachment":"{id}"}}"#)),
            );
            Note {
                guid: guid.into(),
                folder_guid: None,
                title: guid.into(),
                snippet: None,
                body: Some("x".into()),
                locked,
                pinned: false,
                created_at: None,
                modified_at: Some(1),
                editable: true,
                etag: None,
                runs: Some(format!("[{}]", runs.join(","))),
            }
        };
        lib.put_notes(&[
            with("trip", &["FILE", "UNKNOWN", "PIC2", "PIC3"], false),
            with("plain", &[], false),
            with("bank", &["PIC3"], true),
            with("scan", &["DOC"], false),
            with("film", &["FILM"], false),
        ])
        .unwrap();
        for (id, json) in [
            ("FILE", r#"{"kind":"file","hasPreview":false}"#),
            ("PIC2", r#"{"kind":"drawing","hasPreview":true}"#),
            ("PIC3", r#"{"kind":"image","hasPreview":true}"#),
            ("DOC", r#"{"kind":"document","hasPreview":true}"#),
            ("FILM", r#"{"kind":"video","hasPreview":true}"#),
        ] {
            lib.set_state(&format!("notes.attachment.{id}"), json)
                .unwrap();
        }
        assert_eq!(
            lib.note_first_pictures().unwrap(),
            vec![
                ("scan".to_string(), "DOC".to_string()),
                ("trip".to_string(), "PIC2".to_string()),
            ]
        );
    }
}
