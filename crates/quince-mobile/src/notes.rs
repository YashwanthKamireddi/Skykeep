//! Notes on the phone, as on the desktop: read from the phone's own library (instant, and there
//! offline), every change recorded and shown at once and sent behind through the same queue,
//! each pass from Apple laid under what is still on its way. All of it is
//! `quince_sync::notes`, the desktop's own code; this file says it in the phone's terms.
//!
//! The phone edits a note's text; its formatting (headings, lists, checklists, links, colours,
//! attachments) is carried from the text it was to the text it is by the engine
//! (`ckdocument::carry_runs`), so editing never loses what the phone doesn't draw.

use crate::{failed, runtime, Account, EngineError};
use quince_icloud::ckdocument::{carry_runs, Run};
use quince_sync::notes as shared;
use tokio::sync::Mutex;

/// One pass at a time over Notes, sending or reading (see reminders.rs).
static PASS: Mutex<()> = Mutex::const_new(());

/// A folder, with how many notes are in it.
#[derive(uniffi::Record)]
pub struct NFolder {
    pub guid: String,
    pub title: String,
    pub parent_guid: Option<String>,
    pub count: u32,
    /// Recently Deleted: notes waiting to be purged, shown on their own.
    pub trash: bool,
}

/// A note in a list.
#[derive(uniffi::Record)]
pub struct NItem {
    pub guid: String,
    pub folder_guid: Option<String>,
    pub title: String,
    pub snippet: Option<String>,
    /// Unix seconds.
    pub modified_at: Option<i64>,
    pub pinned: bool,
    pub locked: bool,
}

/// How a stretch of a note's text is set. `start` and `length` are in UTF-16 units, as Kotlin
/// counts a string. `style`: 0 title, 1 heading, 2 subheading, 4 monospaced, 100 bulleted,
/// 101 dashed, 102 numbered, 103 checklist; none is body.
#[derive(uniffi::Record)]
pub struct NSpan {
    pub start: u32,
    pub length: u32,
    pub style: Option<u32>,
    pub indent: u32,
    /// A checklist item: ticked or not.
    pub checked: Option<bool>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub link: Option<String>,
    /// A table, picture or other attachment this one character stands for, by id.
    pub attachment: Option<String>,
    pub quote: bool,
    /// Notes' text colour: 1 purple, 2 pink, 3 orange, 4 mint, 5 blue.
    pub emphasis: Option<u32>,
}

/// A note opened: its text and how it is set.
#[derive(uniffi::Record)]
pub struct NDoc {
    pub guid: String,
    pub folder_guid: Option<String>,
    pub title: String,
    pub text: String,
    pub spans: Vec<NSpan>,
    pub pinned: bool,
    pub locked: bool,
    /// False for a note this app can show but not yet change safely; shown read only.
    pub editable: bool,
    /// When it last changed (Unix seconds), shown above it.
    pub modified_at: Option<i64>,
}

/// What a sync of Notes came to.
#[derive(uniffi::Record)]
pub struct NSync {
    pub folders: u32,
    pub notes: u32,
    pub waiting: u32,
    pub refused: Vec<String>,
    /// Advanced Data Protection holds Notes until the person approves on one of their
    /// devices: nothing was asked of Apple, and nothing is retried before then.
    pub paused: bool,
}

fn count(n: impl TryInto<u32>) -> u32 {
    n.try_into().unwrap_or(u32::MAX)
}

fn spans(runs: &[Run]) -> Vec<NSpan> {
    let mut at = 0u64;
    runs.iter()
        .map(|r| {
            let span = NSpan {
                start: count(at),
                length: count(r.length),
                style: r.style.map(count),
                indent: count(r.indent),
                checked: r.checked,
                bold: r.bold,
                italic: r.italic,
                underline: r.underline,
                strike: r.strike,
                link: r.link.clone(),
                attachment: r.attachment.clone(),
                quote: r.quote,
                emphasis: r.emphasis.map(count),
            };
            at += r.length;
            span
        })
        .collect()
}

fn item(n: quince_core::notes::Note) -> NItem {
    NItem {
        guid: n.guid,
        folder_guid: n.folder_guid,
        title: n.title,
        snippet: n.snippet,
        modified_at: n.modified_at,
        pinned: n.pinned,
        locked: n.locked,
    }
}

fn stored_runs(l: &quince_core::Library, guid: &str) -> quince_core::Result<Option<Vec<Run>>> {
    Ok(l.note_runs(guid)?
        .and_then(|r| serde_json::from_str(&r).ok()))
}

/// `runs` with the checklist item on the line holding UTF-16 position `at` ticked or not. The
/// whole line: an item can be several runs (a bold word in it), and every one carries the tick.
fn tick(text: &str, runs: &[Run], at: u32, checked: bool) -> Vec<Run> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let at = usize::try_from(at).unwrap_or(usize::MAX).min(units.len());
    let newline = u16::from(b'\n');
    let line_start = units[..at]
        .iter()
        .rposition(|&u| u == newline)
        .map_or(0, |i| i + 1);
    let line_end = units[at..]
        .iter()
        .position(|&u| u == newline)
        .map_or(units.len(), |i| at + i);
    let mut runs = runs.to_vec();
    let mut start = 0usize;
    for r in &mut runs {
        let end = start + usize::try_from(r.length).unwrap_or(0);
        if start <= line_end && end > line_start && r.checked.is_some() {
            r.checked = Some(checked);
        }
        start = end;
    }
    runs
}

impl Account {
    fn with_notes<T>(
        &self,
        f: impl FnOnce(&mut quince_core::Library) -> quince_core::Result<T>,
    ) -> Result<T, EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async {
            let mut l = lock.lock().await;
            f(&mut l).map_err(failed)
        })
    }
}

fn gone() -> quince_core::Error {
    quince_core::Error::Io(std::io::Error::other(
        "This note isn't here any more: it was deleted, here or on another device.",
    ))
}

#[uniffi::export]
impl Account {
    /// The folders, in Apple's order, each with how many notes it holds.
    pub fn note_folders(&self) -> Result<Vec<NFolder>, EngineError> {
        self.with_notes(|l| {
            let (_, counts) = l.note_counts()?;
            Ok(l.note_folders()?
                .into_iter()
                .map(|f| NFolder {
                    count: count(
                        counts
                            .iter()
                            .find(|(g, _)| *g == f.guid)
                            .map_or(0, |(_, n)| *n),
                    ),
                    trash: f.is_trash(),
                    guid: f.guid,
                    title: f.title,
                    parent_guid: f.parent_guid,
                })
                .collect())
        })
    }

    /// The notes in a folder, or in all but Recently Deleted (`None`): pinned first, then the
    /// most recently changed, as Notes lists them.
    pub fn notes(&self, folder_guid: Option<String>) -> Result<Vec<NItem>, EngineError> {
        self.with_notes(|l| {
            Ok(l.notes_in(folder_guid.as_deref())?
                .into_iter()
                .map(item)
                .collect())
        })
    }

    /// Notes whose title or text has every word of `query`.
    pub fn search_notes(&self, query: String) -> Result<Vec<NItem>, EngineError> {
        self.with_notes(|l| Ok(l.search_notes(&query)?.into_iter().map(item).collect()))
    }

    /// A note opened, with its text and formatting.
    pub fn note(&self, guid: String) -> Result<NDoc, EngineError> {
        self.with_notes(|l| {
            let n = l.note(&guid)?.ok_or_else(gone)?;
            let text = l.note_body(&guid)?.unwrap_or_default();
            let runs = stored_runs(l, &guid)?.unwrap_or_default();
            Ok(NDoc {
                spans: spans(&runs),
                guid: n.guid,
                folder_guid: n.folder_guid,
                title: n.title,
                text,
                pinned: n.pinned,
                locked: n.locked,
                editable: n.editable && !n.locked,
                modified_at: n.modified_at,
            })
        })
    }

    /// The note's text as edited here: shown at once, sent behind. Its formatting is carried
    /// over from what the text was, so every heading, checklist and link stays where it was.
    pub fn note_save(&self, guid: String, text: String) -> Result<(), EngineError> {
        self.with_notes(|l| {
            let before = l.note_body(&guid)?.ok_or_else(gone)?;
            let base_runs = stored_runs(l, &guid)?;
            let runs = base_runs.as_ref().map(|r| carry_runs(&before, r, &text));
            shared::record_note_save(l, &guid, &text, Some(before), runs, base_runs)
        })
    }

    /// A checklist item ticked or unticked: the paragraph holding UTF-16 position `at`.
    pub fn note_check(&self, guid: String, at: u32, checked: bool) -> Result<(), EngineError> {
        self.with_notes(|l| {
            let text = l.note_body(&guid)?.ok_or_else(gone)?;
            let Some(base_runs) = stored_runs(l, &guid)? else {
                return Ok(());
            };
            let runs = tick(&text, &base_runs, at, checked);
            if runs == base_runs {
                return Ok(());
            }
            shared::record_note_save(
                l,
                &guid,
                &text,
                Some(text.clone()),
                Some(runs),
                Some(base_runs),
            )
        })
    }

    /// A new note in `folder_guid` (Apple's own Notes folder when none): shown at once.
    pub fn note_create(
        &self,
        folder_guid: Option<String>,
        text: String,
    ) -> Result<String, EngineError> {
        self.with_notes(|l| shared::record_note_create(l, folder_guid, &text))
    }

    /// Moved into another folder.
    pub fn note_move(&self, guid: String, folder_guid: Option<String>) -> Result<(), EngineError> {
        self.with_notes(|l| shared::record_note_move(l, &guid, folder_guid))
    }

    /// Pinned or unpinned.
    pub fn note_pin(&self, guid: String, pinned: bool) -> Result<(), EngineError> {
        self.with_notes(|l| shared::record_note_pin(l, &guid, pinned))
    }

    /// Deleted: to Recently Deleted, or from there for good, as Notes does it.
    pub fn note_delete(&self, guid: String) -> Result<(), EngineError> {
        self.with_notes(|l| {
            let n = l.note(&guid)?.ok_or_else(gone)?;
            shared::record_note_delete(l, &n)
        })
    }

    /// A new folder at the top of Notes, made at Apple and read back, as the desktop makes one.
    pub fn note_folder_create(&self, title: String) -> Result<(), EngineError> {
        let (http, root, params) = self.notes_context()?;
        runtime()
            .block_on(quince_icloud::notes::create_folder_cloudkit(
                &http, &root, params, &title, None,
            ))
            .map_err(failed)?;
        self.notes_reread()
    }

    /// A folder renamed at Apple and read back.
    pub fn note_folder_rename(&self, guid: String, title: String) -> Result<(), EngineError> {
        let (http, root, params) = self.notes_context()?;
        runtime()
            .block_on(quince_icloud::notes::rename_folder_cloudkit(
                &http, &root, params, &guid, &title,
            ))
            .map_err(failed)?;
        self.notes_reread()
    }

    /// A folder deleted at Apple: its notes, and those of every folder inside it, to Recently
    /// Deleted (as Notes does it), then read back.
    pub fn note_folder_delete(&self, guid: String) -> Result<(), EngineError> {
        let (http, root, params) = self.notes_context()?;
        let (folders, notes) = self.with_notes(|l| Ok((l.note_folders()?, l.notes_in(None)?)))?;
        runtime()
            .block_on(quince_icloud::notes::delete_folder_cloudkit(
                &http, &root, params, &folders, &notes, &guid,
            ))
            .map_err(failed)?;
        self.notes_reread()
    }

    /// The changes made here sent to Apple, oldest first, then Apple's Notes read and laid
    /// under whatever is still on its way, as the desktop does it.
    pub fn notes_sync(&self) -> Result<NSync, EngineError> {
        // The session and whose library it is, read in one look (see reminders_sync).
        let apple = self.apple("notes")?;
        let lock = self.library_of(&apple)?;
        runtime().block_on(async {
            let waits = apple.waiting.contains(&shared::NOTES);
            // Held by Advanced Data Protection: nothing is sent until the approval (see
            // reminders_sync).
            let report = match apple.cloudkit.as_deref() {
                Some(root) if !waits => {
                    quince_sync::queue::drain_as(&lock, shared::NOTES, &PASS, &apple.account, |p| {
                        shared::send(&lock, &apple.http, root, apple.params.clone(), p)
                    })
                    .await
                }
                _ => quince_sync::queue::Report {
                    outcome: quince_sync::queue::Outcome::Later,
                    sent: false,
                    refused: Vec::new(),
                },
            };
            let pass = self
                .pass(quince_sync::pass::NOTES, &apple, &lock, &PASS)
                .await;
            let paused =
                crate::pass::settled(&pass, "Apple hasn't offered Notes for this account.")?;
            let l = lock.lock().await;
            let (all, _) = l.note_counts().map_err(failed)?;
            let folders = l
                .note_folders()
                .map_err(failed)?
                .into_iter()
                .filter(|f| !f.is_trash())
                .count();
            let waiting = l.outbox_pending(shared::NOTES).map_err(failed)?.len();
            Ok(NSync {
                folders: count(folders),
                notes: count(all),
                waiting: count(waiting),
                refused: report.refused.into_iter().map(|(_, why)| why).collect(),
                paused,
            })
        })
    }
}

/// What a call to Apple's Notes needs: the client, CloudKit's address, the session's parameters.
type Apple = (reqwest::Client, String, Vec<(String, String)>);

impl Account {
    fn notes_context(&self) -> Result<Apple, EngineError> {
        let c = self.client();
        let root = c
            .session()
            .service_url("ckdatabasews")
            .ok_or_else(|| failed("Apple hasn't offered Notes for this account."))?;
        Ok((c.http().clone(), root, c.service_params()))
    }

    /// Apple's Notes read again after a folder change that reached Apple. Failing, it is not the
    /// change failing (it is made): the next sync brings the screen up to date.
    fn notes_reread(&self) -> Result<(), EngineError> {
        let apple = self.apple("notes")?;
        let lock = self.library_of(&apple)?;
        let _ = runtime().block_on(self.pass(quince_sync::pass::NOTES, &apple, &lock, &PASS));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checklist item made of three runs (a bold word in the middle) is ticked whole, and
    /// the item on the next line is left as it was.
    #[test]
    fn a_tick_covers_the_whole_line_and_only_it() {
        let item = |length, bold| Run {
            length,
            style: Some(103),
            checked: Some(false),
            bold,
            ..Run::default()
        };
        // "Buy milk now\nCall Ada\n": runs "Buy ", "milk", " now\n", "Call Ada\n".
        let runs = vec![
            item(4, false),
            item(4, true),
            item(5, false),
            item(9, false),
        ];
        let text = "Buy milk now\nCall Ada\n";
        let ticked = tick(text, &runs, 5, true);
        assert_eq!(
            ticked.iter().map(|r| r.checked).collect::<Vec<_>>(),
            vec![Some(true), Some(true), Some(true), Some(false)]
        );
    }
}
