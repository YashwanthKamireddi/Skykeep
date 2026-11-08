//! Notes commands for locked notes and attachments.
//!
//! Locked notes open read-only: the password crosses from the window once, opens the note,
//! and is kept in this process's memory for the session only — never in the library, the
//! settings or the keyring — so the same note opens again without asking until the app quits,
//! the account signs out, or the window says to lock again ([`note_lock_forget`]). The opened
//! text is handed to the window and not stored either: a locked note's words never reach disk
//! here.

use crate::{Auth, Data, UiResult};
use quince_icloud::notes::attach::Attached;
use quince_icloud::notes::locked::Unlocked;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

/// A password held in memory, wiped when let go of.
struct Secret(Vec<u8>);

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// The password that opened each locked note this session, by note.
static PASSWORDS: Mutex<Option<HashMap<String, Secret>>> = Mutex::new(None);

fn remembered(guid: &str) -> Option<String> {
    PASSWORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()?
        .get(guid)
        .and_then(|s| String::from_utf8(s.0.clone()).ok())
}

fn remember(guid: &str, password: &str) {
    PASSWORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(guid.to_string(), Secret(password.as_bytes().to_vec()));
}

/// Every remembered password let go of: on sign-out, or when the window locks notes again.
pub fn forget_passwords() {
    *PASSWORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Open a locked note: its text and formatting. With no password, the one that opened it
/// earlier this session is used; a wrong one is forgotten, never kept.
/// Whether attaching files to notes is switched on (`NOTE_ATTACH_WRITES`): the window shows
/// its paperclip and takes drops only then, so turning the backend flag on is the whole change.
#[tauri::command]
pub fn note_attach_enabled() -> bool {
    quince_icloud::notes::attach::NOTE_ATTACH_WRITES
}

#[tauri::command]
pub async fn note_unlock(
    guid: String,
    password: Option<String>,
    auth: State<'_, Auth>,
) -> UiResult<Unlocked> {
    let password = password
        .filter(|p| !p.is_empty())
        .or_else(|| remembered(&guid))
        .ok_or("Enter this note’s password to open it.")?;
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    match quince_icloud::notes::locked::unlock_cloudkit(&http, &root, params, &guid, &password)
        .await
    {
        Ok(opened) => {
            remember(&guid, &password);
            Ok(opened)
        }
        Err(e) => {
            if let Some(map) = PASSWORDS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut()
            {
                map.remove(&guid);
            }
            Err(e.to_string())
        }
    }
}

/// The hint left with a locked note's password, if any.
#[tauri::command]
pub async fn note_lock_hint(guid: String, auth: State<'_, Auth>) -> UiResult<Option<String>> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    quince_icloud::notes::locked::hint_cloudkit(&http, &root, params, &guid)
        .await
        .map_err(|e| e.to_string())
}

/// Lock every opened note again: the passwords this session held are wiped.
#[tauri::command]
pub fn note_lock_forget() {
    forget_passwords();
}

/// Add the file at `path` to a note at `at_offset` (UTF-16 units of `base`, the text the
/// editor shows, or of Apple's text when no base is given). Answers what was added, then
/// brings the library up to date.
/// A small picture of a file about to be added to a note — shown at once while the file goes
/// up to Apple, so adding never looks stuck. Only for a file the person just picked or dropped
/// (the same grant the add itself checks), read on this computer, never sent anywhere. Empty
/// for a file that is not a picture.
#[tauri::command]
pub async fn note_attach_preview(path: String) -> UiResult<tauri::ipc::Response> {
    crate::grants::take_read(&path)?;
    crate::cpu::run(move || {
        let Ok(bytes) = std::fs::read(&path) else {
            return Vec::new();
        };
        let bytes = if quince_media::is_heif(&bytes) {
            match quince_media::heic_to_jpeg(&bytes) {
                Ok(jpeg) => jpeg,
                Err(_) => return Vec::new(),
            }
        } else {
            bytes
        };
        match quince_media::fit(&bytes, 480) {
            Ok(Some(small)) => small,
            Ok(None) => bytes,
            Err(_) => Vec::new(),
        }
    })
    .await
    .map(tauri::ipc::Response::new)
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn note_attach_file(
    guid: String,
    path: String,
    at_offset: u64,
    base: Option<String>,
    app: tauri::AppHandle,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<Attached> {
    crate::grants::take_read(&path)?;
    // A computer's own bookkeeping (desktop.ini, .DS_Store…), as Send to iPhone leaves it out.
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if crate::send::not_sent(&name) {
        return Err(format!("{name} is a system file, so it wasn’t added."));
    }
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let attached = quince_icloud::notes::attach::attach_file_cloudkit(
        &http,
        &root,
        params.clone(),
        &guid,
        std::path::Path::new(&path),
        base.as_deref(),
        at_offset,
    )
    .await
    .map_err(|e| e.to_string())?;
    // Apple has it, and said what the note now is: kept here as that at once. Waiting for a
    // whole notes pass first held every file added — each one — on "Adding" for its length.
    // The pass still runs, behind, for the note's new tag and anything else that moved; one
    // failing must not read as the attach failing.
    {
        let mut library = data.0.lock().await;
        if let Err(e) = lay_attached(&mut library, &guid, &attached) {
            quiet!("keeping a note's new attachment: {e}");
        }
    }
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;
        let data = app.state::<Data>();
        if let Err(e) = crate::resync_notes(&http, &root, params, &data).await {
            quiet!("notes refresh after an attachment: {e}");
        }
    });
    Ok(attached)
}

/// A note with a file just added, kept as Apple now holds it: its text and formatting as
/// written, its time edited moved on — and any edit still on its way to Apple laid back over
/// it, as a pass lays them, so nothing typed is shown lost meanwhile.
fn lay_attached(
    library: &mut quince_core::Library,
    guid: &str,
    attached: &Attached,
) -> quince_core::Result<()> {
    let (title, snippet) = quince_icloud::notes::title_and_snippet(&attached.body);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    library.atomically(|l| {
        l.set_note_text(
            guid,
            &attached.body,
            &title,
            Some(snippet.as_str()).filter(|s| !s.is_empty()),
        )?;
        if !attached.runs.is_empty() {
            let runs = serde_json::to_string(&attached.runs)
                .map_err(|e| quince_core::Error::Io(std::io::Error::other(e.to_string())))?;
            l.set_note_runs(guid, &runs)?;
        }
        l.touch_note(guid, now)?;
        crate::outbox::replay(l, crate::outbox::NOTES)
    })
}

#[cfg(test)]
mod tests {
    use super::lay_attached;
    use quince_core::Library;
    use quince_icloud::ckdocument::Run;
    use quince_icloud::notes::attach::Attached;

    fn note(lib: &mut Library, guid: &str, body: &str) {
        lib.put_notes(&[quince_core::notes::Note {
            guid: guid.into(),
            folder_guid: None,
            title: body.into(),
            snippet: None,
            body: Some(body.into()),
            locked: false,
            pinned: false,
            created_at: None,
            modified_at: Some(1),
            editable: true,
            etag: None,
            runs: None,
        }])
        .unwrap();
    }

    fn attached(body: &str, runs: Vec<Run>) -> Attached {
        Attached {
            attachment: "A".into(),
            media: "M".into(),
            uti: "public.jpeg".into(),
            offset: 5,
            body: body.into(),
            runs,
        }
    }

    /// A file added is kept as Apple answered at once — text, formatting with the file's run,
    /// a new time edited — with no pass waited for.
    #[test]
    fn an_added_file_is_kept_without_a_pass() {
        let mut lib = Library::in_memory().unwrap();
        note(&mut lib, "N", "Trip\nend");
        let runs = vec![
            Run {
                length: 5,
                style: Some(0),
                ..Run::default()
            },
            Run {
                length: 1,
                attachment: Some("A".into()),
                ..Run::default()
            },
            Run {
                length: 4,
                ..Run::default()
            },
        ];
        lay_attached(
            &mut lib,
            "N",
            &attached("Trip\n\u{FFFC}\nend", runs.clone()),
        )
        .unwrap();
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("Trip\n\u{FFFC}\nend")
        );
        let kept: Vec<Run> = serde_json::from_str(&lib.note_runs("N").unwrap().unwrap()).unwrap();
        assert_eq!(kept, runs);
        let n = lib.note("N").unwrap().unwrap();
        assert_eq!(n.title, "Trip");
        assert!(n.modified_at.unwrap() > 1);
    }

    /// Typing still on its way to Apple stays on screen over the note as Apple answered it.
    #[test]
    fn typing_not_yet_sent_stays_over_an_added_file() {
        let mut lib = Library::in_memory().unwrap();
        note(&mut lib, "N", "Trip\nend");
        crate::outbox::record_note_save(
            &mut lib,
            "N",
            "Trip\nend and more",
            Some("Trip\nend".into()),
            None,
            None,
        )
        .unwrap();
        lay_attached(&mut lib, "N", &attached("Trip\n\u{FFFC}\nend", Vec::new())).unwrap();
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("Trip\nend and more")
        );
    }
}
