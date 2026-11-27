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
/// Freeing up space stopped before checking anything: why, in words.
pub const FREE_UP_FAILED: &str = "quince://free-up-failed";

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

    fn send_changes(&self) {
        crate::outbox::kick();
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
    let _ = APP.set(app.clone());
    let host = Desktop(app);
    // A folder chosen just now needs the person's choice of it (the dialog's grant); the one
    // already saved does not. Asked every time, switching the Hidden album or iCloud Drive on
    // a backup already running answered "Choose the file again": the grant was spent setting
    // the folder, minutes or a launch ago.
    let saved = quince_backup::backup::folder(&host).await;
    if needs_grant(folder.as_deref(), saved.as_deref()) {
        if let Some(f) = &folder {
            crate::grants::take_write(f)?;
        }
    }
    quince_backup::backup::set(&host, folder, hidden, drive).await
}

/// Free up iCloud space: the photos and videos taken more than `months` ago that may leave
/// iCloud because the backup has them, and what stays and why (quince_core::free_up).
#[tauri::command]
pub async fn free_up_plan(
    months: u32,
    app: AppHandle,
) -> Result<quince_core::free_up::Plan, String> {
    let before = jiff::Zoned::now()
        .checked_sub(jiff::Span::new().months(i64::from(months.clamp(1, 120))))
        .map_err(|e| e.to_string())?
        .timestamp()
        .as_second();
    app.state::<Data>()
        .inner()
        .0
        .read(move |library| library.free_up_plan(before))
        .await
        .map_err(|e| e.to_string())
}

/// Free up iCloud space: `ids` (from [`free_up_plan`], less any the person unticked) leave
/// iCloud, each only once checked against Apple's own copy (quince_backup::free_up). Runs in
/// the background; how it goes comes with the backup's own status.
#[tauri::command]
pub async fn free_up_start(ids: Vec<String>, app: AppHandle) -> Result<(), String> {
    let _ = APP.set(app.clone());
    let host = Desktop(app);
    let teller = host.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = quince_backup::free_up::run(&host, ids).await {
            teller.0.emit(FREE_UP_FAILED, e).ok();
        }
    });
    Ok(())
}

/// Stops freeing up space after the photos being checked now.
#[tauri::command]
pub fn free_up_stop() {
    quince_backup::free_up::stop();
}

/// Shared Albums in the backup, or left out.
#[tauri::command]
pub async fn backup_set_shared(app: AppHandle, on: bool) -> Result<Status, String> {
    let _ = APP.set(app.clone());
    quince_backup::backup::set_shared(&Desktop(app), on).await
}

/// Whether setting `folder` needs the person's fresh choice of it: only a folder other than the
/// one already saved. Off (`None`) needs none.
fn needs_grant(folder: Option<&str>, saved: Option<&std::path::Path>) -> bool {
    match folder {
        None => false,
        Some(f) => saved.is_none_or(|s| s != std::path::Path::new(f)),
    }
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

#[cfg(test)]
mod tests {
    use super::needs_grant;
    use std::path::Path;

    /// The folder already saved is set again by every switch on the card (Hidden, iCloud
    /// Drive): no fresh choice needed. A new folder needs one; turning backup off needs none.
    #[test]
    fn only_a_new_folder_needs_the_persons_choice() {
        let saved = Some(Path::new("/media/backup"));
        assert!(
            !needs_grant(Some("/media/backup"), saved),
            "a switch on the card"
        );
        assert!(needs_grant(Some("/media/other"), saved), "a new folder");
        assert!(needs_grant(Some("/media/backup"), None), "the first folder");
        assert!(!needs_grant(None, saved), "off");
    }
}
