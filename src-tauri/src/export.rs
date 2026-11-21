//! Export Everything, the desktop's commands: the work itself is quince_backup::export, shared
//! with the Mac app. This file keeps what belongs to the window: the event it hears, one export
//! at a time, the folder grant, and Show in Folder.

use crate::{Data, UiResult};
pub(crate) use quince_backup::export::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, State};

/// Told to the window as an export moves along: an [`ExportProgress`].
pub const EVENT: &str = "quince://export";

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
/// The last finished export, the one folder Show in Folder may show.
static LAST: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Clears the running flag however the export ends.
struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Export everything into a new folder inside `dest`, telling [`EVENT`] as it goes. Photos only
/// when `photos` is true: they can be very large.
#[tauri::command]
pub async fn library_export_start(
    dest: String,
    photos: Option<bool>,
    app: tauri::AppHandle,
    data: State<'_, Data>,
) -> UiResult<ExportSummary> {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Err("An export is already under way.".into());
    }
    let _running = Running;
    crate::grants::take_write(&dest)?;
    CANCEL.store(false, Ordering::Release);
    let data_dir = crate::state_dir(&app);
    // Checked and read in one hold of the library, then let go of before a file is written.
    let (snap, dest) = {
        let lib = data.0.lock().await;
        let synced: Vec<PathBuf> = ["drive_folder_path", "photos_folder_path"]
            .iter()
            .filter_map(|k| lib.state(k).ok().flatten())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        let dest = check_dest(Path::new(&dest), &data_dir, &synced)?;
        (gather(&lib, photos.unwrap_or(false))?, dest)
    };
    let sources = Sources::at(&data_dir);
    let handle = app.clone();
    let now = jiff::Zoned::now();
    let done = tokio::task::spawn_blocking(move || {
        run(&snap, &sources, &dest, &now, &CANCEL, &mut |p| {
            let _ = handle.emit(EVENT, p);
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    match done {
        Ok(summary) => {
            if let Ok(mut last) = LAST.lock() {
                *last = Some(PathBuf::from(&summary.path));
            }
            Ok(summary)
        }
        Err(Stop::Cancelled) => Err(CANCELLED.into()),
        Err(Stop::Failed(why)) => Err(why),
    }
}

/// Stop the export under way; what it wrote so far is removed.
#[tauri::command]
pub fn library_export_cancel() {
    if RUNNING.load(Ordering::Acquire) {
        CANCEL.store(true, Ordering::Release);
    }
}

/// Show the last finished export in the file manager — that folder only, never a path the page
/// names.
#[tauri::command]
pub fn library_export_reveal(app: tauri::AppHandle) -> UiResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let last = LAST.lock().ok().and_then(|l| l.clone());
    let path = last
        .filter(|p| p.is_dir())
        .ok_or_else(|| "That export isn’t there any more.".to_string())?;
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())
}
