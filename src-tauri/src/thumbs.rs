//! File Explorer's thumbnails for the iCloud Photos folder, answered: Explorer's thumbnail
//! provider (crates/quince-thumbs, a DLL inside Explorer) sends a file's path over a local pipe,
//! and gets back that photo's small picture, ready to draw. The picture is the one the Photos
//! app's grid keeps, or fetched alone (a few kilobytes): a thumbnail never downloads the photo.
//!
//! The pipe takes only this session's own clients (`reject_remote_clients`; other people on the
//! computer can read it but never write a request), answers for Skykeep's own placeholders
//! only, a few at a time so a folder of thousands never floods Apple, and always within the
//! provider's patience, so Explorer is never kept waiting on us.

use crate::photos::Size;
use quince_thumbs::protocol;
use std::time::Duration;
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// Pictures prepared at once: enough to fill a screen of thumbnails quickly, few enough that a
/// folder of thousands scrolled past asks Apple for no more than this at a time.
static AT_ONCE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// Within the provider's own patience (8 s), so a slow answer is ours to drop, not Explorer's.
const ANSWER_WITHIN: Duration = Duration::from_secs(6);

/// Start answering, once per run. Another Skykeep already answering (a second copy started):
/// that one does, and this returns.
pub fn serve(app: &AppHandle) {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Some(session) = quince_thumbs::this_session() else {
        return;
    };
    let name = protocol::pipe_name(session);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        loop {
            let Ok(pipe) = ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create(&name)
            else {
                return;
            };
            first = false;
            if pipe.connect().await.is_ok() {
                tauri::async_runtime::spawn(answer(app.clone(), pipe));
            }
        }
    });
}

async fn answer(app: AppHandle, mut pipe: NamedPipeServer) {
    let picture = tokio::time::timeout(ANSWER_WITHIN, picture_for(&app, &mut pipe))
        .await
        .ok()
        .flatten();
    let reply = protocol::reply(picture.as_ref().map(|(w, h, px)| (*w, *h, px.as_slice())));
    if pipe.write_all(&reply).await.is_err() {
        return;
    }
    // Held open until the provider has read it all and closed its end: closing first could
    // throw away what it hasn't read yet.
    let mut rest = [0u8; 1];
    let _ = tokio::time::timeout(Duration::from_secs(2), pipe.read(&mut rest)).await;
}

async fn picture_for(app: &AppHandle, pipe: &mut NamedPipeServer) -> Option<(u32, u32, Vec<u8>)> {
    use std::os::windows::ffi::OsStringExt;
    let mut head = [0u8; 8];
    pipe.read_exact(&mut head).await.ok()?;
    let (size, units) = protocol::request_head(head)?;
    let mut body = vec![0u8; units as usize * 2];
    pipe.read_exact(&mut body).await.ok()?;
    let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&protocol::path_of(&body)));
    let _turn = AT_ONCE.acquire().await.ok()?;
    let id = tokio::task::spawn_blocking(move || crate::mount_photos::photo_at(&path))
        .await
        .ok()??;
    let bytes = crate::photos::picture(app, &id, Size::Thumb).await.ok()?;
    tokio::task::spawn_blocking(move || quince_media::thumbnail_bgra(&bytes, size).ok())
        .await
        .ok()?
}
