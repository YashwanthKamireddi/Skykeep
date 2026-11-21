//! Backup, the desktop's side: the work itself is quince_backup::backup, shared with the Mac
//! app. Here the app gives it the library, the session and the window (the [`Host`]), and the
//! window's commands call it.

use crate::Data;
use quince_backup::backup::{Host, Service, Status};
use quince_sync::library_lock::LibraryLock;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

/// The window hears how a backup is going here.
pub const PROGRESS: &str = "quince://backup";

/// The app, kept at launch for Photos' change notice (refresh.rs has none of its own).
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

#[derive(Clone)]
struct Desktop(AppHandle);

impl Host for Desktop {
    fn library(&self) -> &LibraryLock {
        &self.0.state::<Data>().inner().0
    }

    fn cloudkit(&self) -> futures_util::future::BoxFuture<'_, Result<Service, String>> {
        Box::pin(async move { crate::cloudkit_context(&self.0.state::<crate::Auth>()).await })
    }

    fn drive(&self) -> futures_util::future::BoxFuture<'_, Result<(Service, String), String>> {
        Box::pin(async move {
            let drive = crate::drive_context(&self.0.state::<crate::Auth>()).await?;
            let docws = {
                let auth = self.0.state::<crate::Auth>();
                let client = auth.client.lock().await;
                client
                    .session()
                    .service_url("docws")
                    .ok_or("Apple hasn't offered iCloud Drive downloads for this account.")?
            };
            Ok((drive, docws))
        })
    }

    fn hidden_open(&self) -> bool {
        crate::photos::HIDDEN_OPEN.load(Ordering::SeqCst)
    }

    fn tell(&self, status: Status) {
        self.0.emit(PROGRESS, status).ok();
    }

    fn spawn(&self, work: futures_util::future::BoxFuture<'static, ()>) {
        tauri::async_runtime::spawn(work);
    }
}

/// The backup folder, shown in the file manager.
#[tauri::command]
pub async fn backup_reveal(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let folder = quince_backup::backup::folder(&Desktop(app.clone()))
        .await
        .filter(|f| f.is_dir())
        .ok_or(
            "The backup folder isn\u{2019}t there. Connect its drive, or choose another folder.",
        )?;
    app.opener()
        .reveal_item_in_dir(&folder)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn backup_status(app: AppHandle) -> Result<Status, String> {
    Ok(quince_backup::backup::status(&Desktop(app)).await)
}

/// Backup on into `folder` (chosen in the folder dialog, so granted), or off (`None`); with the
/// Hidden album or without; with iCloud Drive or without.
#[tauri::command]
pub async fn backup_set(
    app: AppHandle,
    folder: Option<String>,
    hidden: bool,
    drive: bool,
) -> Result<Status, String> {
    if let Some(f) = &folder {
        crate::grants::take_write(f)?;
    }
    let _ = APP.set(app.clone());
    quince_backup::backup::set(&Desktop(app), folder, hidden, drive).await
}

/// At launch: the app kept for Photos' notices, which come from refresh.rs with no app of their
/// own, so a launch in the tray (no window to sign in) still backs up.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

/// Photos changed at Apple: what is new goes into the backup, when backup is on.
pub fn photos_changed() {
    if let Some(app) = APP.get() {
        kick(app);
    }
}

/// iCloud Drive changed at Apple (its change feed, drive_watch.rs): gone through again.
pub fn drive_changed() {
    quince_backup::backup::drive_due();
    if let Some(app) = APP.get() {
        kick(app);
    }
}

/// Copy what isn't copied yet, if backup is on: at launch, when Photos changed, when turned on.
pub fn kick(app: &AppHandle) {
    let _ = APP.set(app.clone());
    quince_backup::backup::kick(&Desktop(app.clone()));
}
