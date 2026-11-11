//! Notes: a change made here recorded and shown at once (`record_*`), laid back over every pass
//! from Apple until it is sent (`lay`, `replay`), and sent to Apple behind (`send`); and a pass
//! from Apple stored (`refresh`, `apply`). Moved whole from the desktop app so the phone's
//! engine runs the same code.

use crate::library_lock::LibraryLock;
use crate::queue::{json, known_gone, unreadable};
use quince_core::outbox::Pending;
use quince_core::Library;
use quince_icloud::ckdocument::Run;
use quince_icloud::Error;
use serde::{Deserialize, Serialize};

/// The queue Notes' changes wait in.
pub const NOTES: &str = "notes";

/// A note's text saved here: what was typed, and what it was typed over.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteSave {
    pub body: String,
    pub base: Option<String>,
    pub runs: Option<Vec<Run>>,
    pub base_runs: Option<Vec<Run>>,
    /// When it was typed, unix seconds: the note's time edited while it waits. A pass before it
    /// is sent brings Apple's older time, and laid back without it the note dropped down the
    /// list until the send, then jumped back up. Absent in a save queued before it was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
}

/// A note made here, under the name it was given here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteCreate {
    pub folder_guid: Option<String>,
    pub body: String,
    pub at: i64,
}

/// A note moved here into a folder (`None`: Apple's own "Notes").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteMove {
    pub folder_guid: Option<String>,
}

/// A note deleted here: to Recently Deleted, or — from there — for good. `from_folder` is
/// where it was, which says which; `title` what it was called, for a refusal to name it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteDelete {
    pub from_folder: Option<String>,
    pub title: String,
}

/// A note pinned or unpinned here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotePin {
    pub pinned: bool,
}

// --- Recording: each change recorded and laid over the library as one transaction.

/// A note's text, saved here. Several saves before one is sent become one, sent from the
/// text the first was typed over; one already on its way is followed by the next, typed over
/// what it sends.
///
/// A save that changes nothing — the text it was typed over, set the same way however the
/// runs are cut — is no save: it is never queued, and one that undoes the change still waiting
/// takes that change back. Opening or looking at a note must never write it.
pub fn record_note_save(
    library: &mut Library,
    guid: &str,
    body: &str,
    base: Option<String>,
    runs: Option<Vec<Run>>,
    base_runs: Option<Vec<Run>>,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let stored = l.note_body(guid)?;
        let stored_runs: Option<Vec<Run>> = l
            .note_runs(guid)?
            .and_then(|r| serde_json::from_str(&r).ok());
        let waiting: Option<(Pending, NoteSave)> = l
            .outbox_waiting(NOTES, guid, "save")?
            .into_iter()
            .next_back()
            .and_then(|p| serde_json::from_str(&p.payload).ok().map(|s| (p, s)));
        let at = chrono_now();
        let save = |base, base_runs| NoteSave {
            body: body.to_string(),
            base,
            runs: runs.clone(),
            base_runs,
            at: Some(at),
        };
        match waiting {
            Some((p, before)) if p.tries == 0 => {
                if changes_nothing(
                    body,
                    runs.as_deref(),
                    before.base.as_deref(),
                    before.base_runs.as_deref(),
                ) && l.outbox_withdraw(p.id)?
                {
                    // Back to what Apple holds: the change waiting is taken back, not sent.
                } else {
                    let merged = save(before.base, before.base_runs);
                    if !l.outbox_amend(p.id, &json(&merged)?)? {
                        l.outbox_append(NOTES, guid, "save", &json(&merged)?)?;
                    }
                }
            }
            Some((_, before)) => {
                if changes_nothing(
                    body,
                    runs.as_deref(),
                    Some(&before.body),
                    before.runs.as_deref(),
                ) {
                    return Ok(());
                }
                l.outbox_append(
                    NOTES,
                    guid,
                    "save",
                    &json(&save(Some(before.body), before.runs))?,
                )?;
            }
            None => {
                // Nothing waiting: the library holds what Apple holds, the base when the
                // editor did not say.
                let base = base.or(stored);
                let base_runs = base_runs.or(stored_runs);
                if changes_nothing(body, runs.as_deref(), base.as_deref(), base_runs.as_deref()) {
                    return Ok(());
                }
                l.outbox_append(NOTES, guid, "save", &json(&save(base, base_runs))?)?;
            }
        }
        lay_note(l, guid, body, runs.as_deref())?;
        l.touch_note(guid, at)
    })
}

/// Now, in unix seconds.
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Whether a note save of `body` set by `runs`, typed over `base` set by `base_runs`, changes
/// nothing: the same text, and — when it carries formatting — the same formatting once both
/// are in one shape ([`quince_icloud::notes::same_formatting`]: Apple's runs come in pieces,
/// an editor's merged, and that difference alone is not an edit).
fn changes_nothing(
    body: &str,
    runs: Option<&[Run]>,
    base: Option<&str>,
    base_runs: Option<&[Run]>,
) -> bool {
    base == Some(body)
        && match runs {
            None => true,
            Some(runs) => {
                base_runs.is_some_and(|b| quince_icloud::notes::same_formatting(body, runs, b))
            }
        }
}

fn lay_note(
    l: &mut Library,
    guid: &str,
    body: &str,
    runs: Option<&[Run]>,
) -> quince_core::Result<()> {
    let (derived, snippet) = quince_icloud::notes::title_and_snippet(body);
    // The title the library holds — Apple's, as the phone set it — while the edit leaves the
    // title line alone; the new first line's once it reaches it (notes.rs
    // `title_line_unchanged`, the rule the send follows too, so here and the phone agree).
    let held = l
        .note(guid)?
        .map(|n| n.title)
        .filter(|t| !t.trim().is_empty());
    let title = match (held, l.note_body(guid)?) {
        (Some(held), Some(before)) if quince_icloud::notes::title_line_unchanged(&before, body) => {
            held
        }
        _ => derived,
    };
    l.set_note_text(
        guid,
        body,
        &title,
        Some(snippet.as_str()).filter(|s| !s.is_empty()),
    )?;
    if let Some(runs) = runs {
        l.set_note_runs(guid, &json(&runs)?)?;
    }
    Ok(())
}

/// A note made here: named now (an uppercase UUID, as Apple names one), shown now, sent
/// behind. Answers its name.
pub fn record_note_create(
    library: &mut Library,
    folder_guid: Option<String>,
    body: &str,
) -> quince_core::Result<String> {
    let guid = quince_icloud::notes::new_note_name();
    let create = NoteCreate {
        folder_guid,
        body: body.to_string(),
        at: jiff::Timestamp::now().as_second(),
    };
    library.atomically(|l| {
        l.outbox_append(NOTES, &guid, "create", &json(&create)?)?;
        lay_note_create(l, &guid, &create)?;
        Ok(guid.clone())
    })
}

/// A folder this library holds, or none: a note filed under one it does not know would break
/// the library's own link from note to folder.
fn known_folder(l: &Library, folder: Option<&str>) -> quince_core::Result<Option<String>> {
    let Some(folder) = folder else {
        return Ok(None);
    };
    Ok(l.note_folders()?
        .into_iter()
        .any(|f| f.guid == folder)
        .then(|| folder.to_string()))
}

fn lay_note_create(l: &mut Library, guid: &str, c: &NoteCreate) -> quince_core::Result<()> {
    if l.note(guid)?.is_some() {
        return Ok(());
    }
    let (title, snippet) = quince_icloud::notes::title_and_snippet(&c.body);
    let folder_guid = known_folder(l, c.folder_guid.as_deref())?;
    l.put_notes(&[quince_core::notes::Note {
        guid: guid.to_string(),
        folder_guid,
        title,
        snippet: Some(snippet).filter(|s| !s.is_empty()),
        body: Some(c.body.clone()),
        locked: false,
        pinned: false,
        created_at: Some(c.at),
        modified_at: Some(c.at),
        editable: true,
        etag: None,
        runs: None,
    }])
}

/// A note moved to a folder. Only where it ends up needs to reach Apple: an earlier move not
/// yet sent is replaced.
pub fn record_note_move(
    library: &mut Library,
    guid: &str,
    folder_guid: Option<String>,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let m = NoteMove { folder_guid };
        l.outbox_push(NOTES, guid, "move", &json(&m)?)?;
        lay_note_move(l, guid, &m)
    })
}

fn lay_note_move(l: &mut Library, guid: &str, m: &NoteMove) -> quince_core::Result<()> {
    // No folder is Apple's own "Notes", where Apple files it.
    let to = m
        .folder_guid
        .as_deref()
        .unwrap_or(quince_icloud::notes::DEFAULT_FOLDER);
    let folder = known_folder(l, Some(to))?;
    l.set_note_folder(guid, folder.as_deref())
}

/// A note deleted: to Recently Deleted, or from there for good.
pub fn record_note_delete(
    library: &mut Library,
    note: &quince_core::notes::Note,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let d = NoteDelete {
            from_folder: note.folder_guid.clone(),
            title: note.title.clone(),
        };
        l.outbox_append(NOTES, &note.guid, "delete", &json(&d)?)?;
        lay_note_delete(l, &note.guid, &d)
    })
}

fn lay_note_delete(l: &mut Library, guid: &str, d: &NoteDelete) -> quince_core::Result<()> {
    use quince_core::notes::TRASH_FOLDER;
    if d.from_folder.as_deref() == Some(TRASH_FOLDER) {
        return l.forget_notes(&[guid.to_string()]);
    }
    let trash = known_folder(l, Some(TRASH_FOLDER))?;
    if trash.is_none() {
        // No Recently Deleted to show it in: out of sight, as it would be on the phone.
        return l.forget_notes(&[guid.to_string()]);
    }
    l.set_note_folder(guid, trash.as_deref())
}

/// A note pinned or unpinned: the last word is the one sent.
pub fn record_note_pin(library: &mut Library, guid: &str, pinned: bool) -> quince_core::Result<()> {
    library.atomically(|l| {
        l.outbox_push(NOTES, guid, "pin", &json(&NotePin { pinned })?)?;
        l.set_note_pinned(guid, pinned)
    })
}

/// One waiting Notes change laid over the library.
pub fn lay(l: &mut Library, p: &Pending) -> quince_core::Result<()> {
    match p.kind.as_str() {
        "save" => {
            if let Ok(s) = serde_json::from_str::<NoteSave>(&p.payload) {
                if l.note(&p.record)?.is_some() {
                    lay_note(l, &p.record, &s.body, s.runs.as_deref())?;
                    if let Some(at) = s.at {
                        l.touch_note(&p.record, at)?;
                    }
                }
            }
        }
        "create" => {
            if let Ok(c) = serde_json::from_str::<NoteCreate>(&p.payload) {
                lay_note_create(l, &p.record, &c)?;
            }
        }
        "move" => {
            if let Ok(m) = serde_json::from_str::<NoteMove>(&p.payload) {
                lay_note_move(l, &p.record, &m)?;
            }
        }
        "delete" => {
            if let Ok(d) = serde_json::from_str::<NoteDelete>(&p.payload) {
                lay_note_delete(l, &p.record, &d)?;
            }
        }
        "pin" => {
            if let Ok(x) = serde_json::from_str::<NotePin>(&p.payload) {
                l.set_note_pinned(&p.record, x.pinned)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The note changes still waiting, laid over what a pass stored, in the order made.
pub fn replay(library: &mut Library) -> quince_core::Result<()> {
    for p in library.outbox_pending(NOTES)? {
        lay(library, &p)?;
    }
    Ok(())
}

// --- From Apple.

/// A pass from Apple stored: folders and notes, then every edit still on its way laid back over
/// them, so what was typed here stays as typed. `full` marks a pass from the start. Answers the
/// folders and notes it held.
pub fn apply(
    library: &mut Library,
    fetched: quince_icloud::notes::NotesFetch,
    full: bool,
) -> Result<(usize, usize), Error> {
    // One transaction, Apple's pass and the edits laid back over it, as the desktop stores it:
    // killed between the two (Android stops apps), unsent edits showed reverted until the next.
    library.atomically(|library| {
        let (folders, notes, _) = quince_icloud::notes::apply_cloudkit(library, fetched)?;
        replay(library)?;
        if full {
            library.mark_full_sync(NOTES)?;
        }
        Ok((folders, notes))
    })
}

// --- Sending: the queue's half.

pub async fn send(
    library: &LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
) -> Result<(), Error> {
    use quince_icloud::notes as n;
    match p.kind.as_str() {
        "save" => send_save(library, http, root, params, p).await,
        "create" => {
            let c: NoteCreate = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            // The shape of a document Apple wrote, noticed during a sync: a note built without
            // it reads back here perfectly and shows on a phone as an empty note.
            let template = library.lock().await.state(n::TEMPLATE_KEY)?;
            n::create_cloudkit_named(
                http,
                root,
                params,
                &p.record,
                c.folder_guid.as_deref(),
                &c.body,
                template.as_deref(),
            )
            .await
        }
        "move" => {
            let m: NoteMove = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            let Some(note) = library.lock().await.note(&p.record)? else {
                known_gone(library, NOTES).await?;
                return Err(Error::Refused(
                    "This note was deleted on another device, so it wasn’t moved.".into(),
                ));
            };
            n::move_cloudkit(http, root, params, &note, m.folder_guid.as_deref()).await
        }
        "delete" => {
            let d: NoteDelete = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            let stored = library.lock().await.note(&p.record)?;
            // Sent as the note was when it was deleted: from Recently Deleted is for good.
            let note = quince_core::notes::Note {
                guid: p.record.clone(),
                folder_guid: d.from_folder.clone(),
                title: d.title.clone(),
                snippet: None,
                body: None,
                locked: false,
                pinned: false,
                created_at: None,
                modified_at: None,
                editable: true,
                etag: stored.and_then(|s| s.etag),
                runs: None,
            };
            n::delete_cloudkit(http, root, params, &note).await
        }
        "pin" => {
            let x: NotePin = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            // Gone since: nothing left to pin.
            let Some(note) = library.lock().await.note(&p.record)? else {
                return known_gone(library, NOTES).await;
            };
            n::pin_cloudkit(http, root, params, &note, x.pinned).await
        }
        other => Err(Error::Refused(format!(
            "Skykeep does not know how to send “{other}”."
        ))),
    }
}

async fn send_save(
    library: &LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
) -> Result<(), Error> {
    let s: NoteSave = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
    let Some(note) = library.lock().await.note(&p.record)? else {
        known_gone(library, NOTES).await?;
        return Err(Error::Refused(
            "This note was deleted on another device, so the edit wasn’t saved.".into(),
        ));
    };
    let saved = quince_icloud::notes::edit_rich_cloudkit_from(
        http,
        root,
        params,
        &note,
        s.base.as_deref(),
        &s.body,
        s.runs.as_deref(),
        s.base_runs.as_deref(),
    )
    .await?;
    // Apple's change feed does not always carry our own write back on the very next pass:
    // what Apple now holds — ours, or ours with another device's edit merged in — is what the
    // note says, unless more typing waits to go after it.
    let mut l = library.lock().await;
    let more = l
        .outbox_waiting(NOTES, &p.record, "save")?
        .iter()
        .any(|w| w.id != p.id);
    if !more {
        let runs = s.runs.as_ref().map(|runs| {
            if saved == s.body {
                runs.clone()
            } else {
                quince_icloud::ckdocument::carry_runs(&s.body, runs, &saved)
            }
        });
        lay_note(&mut l, &p.record, &saved, runs.as_deref())?;
    }
    Ok(())
}
