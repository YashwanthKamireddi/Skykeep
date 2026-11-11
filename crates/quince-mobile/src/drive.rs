//! iCloud Drive on the phone, for Android's Files app (the app's DocumentsProvider): folders read
//! from Apple and kept in the phone's library; a file read in parts as an app reads it, into the
//! desktop folder's own chunk cache (`quince_vfs::Cache`, ranged reads from
//! `quince_icloud::drive_parts`); and a file written by another app held in the desktop folder's
//! own journal (`quince_vfs::writes`) and sent back through its write-back: into that same file
//! when it is still the version opened, beside it as a conflicted copy when it changed elsewhere
//! meanwhile, never over another device's work. What is held outlives the process.

use crate::{failed, runtime, Account, EngineError};
use quince_core::drive::DriveItem;
use quince_vfs::{Fut, Remote};
use std::path::Path;
use std::sync::Arc;

/// An iCloud Drive item, as the Files app lists it.
#[derive(uniffi::Record)]
pub struct DItem {
    /// Apple's id: `FOLDER::…` or `FILE::…`.
    pub id: String,
    pub parent_id: Option<String>,
    /// The name with its extension, as a file manager shows it.
    pub name: String,
    pub folder: bool,
    pub size: Option<i64>,
    /// Unix seconds.
    pub modified_at: Option<i64>,
    /// How many items a folder holds, when Apple says.
    pub children: Option<i64>,
    /// Apple's version of it: what a cached thumbnail or copy is kept under.
    pub etag: Option<String>,
}

fn ditem(i: &DriveItem) -> DItem {
    DItem {
        id: i.drivewsid.clone(),
        parent_id: i.parent_id.clone(),
        name: i.display_name(),
        folder: i.is_container(),
        size: i.size,
        modified_at: i.modified_at,
        children: i.child_count,
        etag: i.etag.clone(),
    }
}

/// The engine's way to Apple for the desktop's file tree (`quince_vfs::Remote`), on the phone:
/// the same calls the desktop's makes (src-tauri/src/mount.rs), with the phone's session and
/// library.
#[derive(Clone)]
struct PhoneRemote(Arc<Session>);

/// What every call to Apple needs, shared by the futures the file tree keeps.
struct Session {
    http: reqwest::Client,
    drivews: String,
    docws: String,
    setup: String,
    params: Vec<(String, String)>,
    library: Arc<quince_sync::library_lock::LibraryLock>,
    /// The download links in use, kept by the account across calls.
    links: Arc<quince_icloud::drive_parts::Links>,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl std::ops::Deref for PhoneRemote {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.0
    }
}

impl Session {
    /// iCloud with no room for `grows` more bytes: said as that, never an opaque refusal.
    async fn room_for(&self, grows: u64) -> Result<(), String> {
        if grows == 0 {
            return Ok(());
        }
        if let Ok(s) =
            quince_icloud::drive::storage(&self.http, &self.setup, self.params.clone()).await
        {
            if s.total_bytes > 0 && s.used_bytes.saturating_add(grows) > s.total_bytes {
                return Err("Your iCloud storage is full.".into());
            }
        }
        Ok(())
    }
}

impl Remote for PhoneRemote {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let folder = folder.to_string();
        let me = self.clone();
        Box::pin(async move {
            let items = quince_icloud::drive::fetch_folder(
                &me.http,
                &me.drivews,
                me.params.clone(),
                &folder,
            )
            .await
            .map_err(text)?;
            let mut l = me.library.lock().await;
            quince_icloud::drive::store_folder(&mut l, &folder, &items).map_err(text)?;
            Ok(items)
        })
    }

    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        let part = self.read_part(item, offset, len);
        Box::pin(async move { Ok(part.await?.bytes) })
    }

    fn read_part(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<quince_vfs::Part> {
        let (me, item) = (self.clone(), item.clone());
        Box::pin(async move {
            let served = me
                .links
                .read(&me.http, &me.docws, me.params.clone(), &item, offset, len)
                .await
                .map_err(text)?;
            Ok(quince_vfs::Part {
                bytes: served.bytes,
                total: served.total,
            })
        })
    }

    fn mkdir(&self, parent: &DriveItem, name: &str) -> Fut<DriveItem> {
        let (parent, name) = (parent.drivewsid.clone(), name.to_string());
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::create_folder(
                &me.http,
                &me.drivews,
                me.params.clone(),
                &parent,
                &name,
            )
            .await
            .map_err(text)
        })
    }

    fn rename(&self, item: &DriveItem, name: &str) -> Fut<()> {
        let (item, name) = (item.clone(), name.to_string());
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::rename(&me.http, &me.drivews, me.params.clone(), &item, &name)
                .await
                .map_err(text)
        })
    }

    fn move_to(&self, item: &DriveItem, folder: &DriveItem) -> Fut<()> {
        let (item, to) = (item.clone(), folder.drivewsid.clone());
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::move_to(&me.http, &me.drivews, me.params.clone(), &item, &to)
                .await
                .map_err(text)
        })
    }

    fn upload(&self, folder: &DriveItem, path: &Path) -> Fut<DriveItem> {
        let (folder, path) = (folder.drivewsid.clone(), path.to_path_buf());
        let me = self.clone();
        Box::pin(async move {
            let len = tokio::fs::metadata(&path).await.map_or(0, |m| m.len());
            me.room_for(len).await?;
            let progress: quince_icloud::drive::Progress = Arc::new(|_, _| {});
            quince_icloud::drive::upload(
                &me.http,
                &me.docws,
                &me.drivews,
                me.params.clone(),
                &folder,
                &path,
                progress,
            )
            .await
            .map_err(text)
        })
    }

    fn replace(&self, item: &DriveItem, path: &Path) -> Fut<Option<DriveItem>> {
        let (item, path) = (item.clone(), path.to_path_buf());
        let me = self.clone();
        Box::pin(async move {
            let len = tokio::fs::metadata(&path).await.map_or(0, |m| m.len());
            let was = item.size.and_then(|s| u64::try_from(s).ok()).unwrap_or(0);
            me.room_for(len.saturating_sub(was)).await?;
            let progress: quince_icloud::drive::Progress = Arc::new(|_, _| {});
            match quince_icloud::drive::replace_contents(
                &me.http,
                &me.docws,
                &me.drivews,
                me.params.clone(),
                &item,
                &path,
                progress,
            )
            .await
            {
                Ok(now) => Ok(Some(now)),
                // Changed on another device first: nothing written; the write-back keeps both.
                Err(quince_icloud::Error::Conflict) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
    }

    fn delete(&self, item: &DriveItem) -> Fut<()> {
        let item = item.clone();
        let me = self.clone();
        Box::pin(async move {
            quince_icloud::drive::delete(&me.http, &me.drivews, me.params.clone(), &item)
                .await
                .map_err(text)
        })
    }

    fn space(&self) -> Fut<(u64, u64)> {
        let me = self.clone();
        Box::pin(async move {
            let s = quince_icloud::drive::storage(&me.http, &me.setup, me.params.clone())
                .await
                .map_err(text)?;
            Ok((s.used_bytes, s.total_bytes))
        })
    }
}

impl Account {
    fn drive(&self) -> Result<PhoneRemote, EngineError> {
        let (library, _) = self.library()?;
        let c = self.client();
        let s = c.session();
        let need = |name: &str| {
            s.service_url(name)
                .ok_or_else(|| failed("Apple hasn't offered iCloud Drive for this account."))
        };
        Ok(PhoneRemote(Arc::new(Session {
            drivews: need("drivews")?,
            docws: need("docws")?,
            setup: c.setup_url().to_string(),
            http: c.http().clone(),
            params: c.service_params(),
            library,
            links: Arc::clone(&self.drive_links),
        })))
    }

    fn known(&self, id: &str) -> Result<DriveItem, EngineError> {
        // The top of iCloud Drive is never one of a folder's items, so never in the library: it
        // is known by its id. Without this, nothing could be made or moved at the top.
        if id == quince_core::drive::ROOT {
            return Ok(DriveItem {
                drivewsid: id.to_string(),
                parent_id: None,
                name: "iCloud Drive".into(),
                extension: None,
                kind: "FOLDER".into(),
                size: None,
                child_count: None,
                created_at: None,
                modified_at: None,
                etag: None,
            });
        }
        let (lock, _) = self.library()?;
        runtime()
            .block_on(async { lock.lock().await.drive_item(id) })
            .map_err(failed)?
            .ok_or_else(|| failed("That item isn't in iCloud Drive any more."))
    }
}

#[uniffi::export]
impl Account {
    /// The top of iCloud Drive's id.
    pub fn drive_root(&self) -> String {
        quince_core::drive::ROOT.to_string()
    }

    /// A folder's items: as the phone last saw them, or (with `fresh`, or when never seen) read
    /// from Apple now and kept.
    pub fn drive_children(&self, folder: String, fresh: bool) -> Result<Vec<DItem>, EngineError> {
        let (lock, _) = self.library()?;
        let kept = runtime()
            .block_on(async { lock.lock().await.drive_children(&folder) })
            .map_err(failed)?;
        if !fresh && !kept.is_empty() {
            return Ok(kept.iter().map(ditem).collect());
        }
        let remote = self.drive()?;
        let items = runtime().block_on(remote.list(&folder)).map_err(failed)?;
        Ok(items.iter().map(ditem).collect())
    }

    /// One item, as the phone last saw it.
    pub fn drive_item(&self, id: String) -> Result<Option<DItem>, EngineError> {
        let (lock, _) = self.library()?;
        Ok(runtime()
            .block_on(async { lock.lock().await.drive_item(&id) })
            .map_err(failed)?
            .as_ref()
            .map(ditem))
    }

    /// Items whose name has `query`, from what the phone has seen.
    pub fn drive_search(&self, query: String) -> Result<Vec<DItem>, EngineError> {
        let (lock, _) = self.library()?;
        Ok(runtime()
            .block_on(async { lock.lock().await.search_drive(&query, 200) })
            .map_err(failed)?
            .iter()
            .map(ditem)
            .collect())
    }

    /// A file fetched whole to `path` (the app's cache), for the Files app to open.
    pub fn drive_download(&self, id: String, path: String) -> Result<(), EngineError> {
        let item = self.known(&id)?;
        let remote = self.drive()?;
        let partial = format!("{path}.part");
        runtime()
            .block_on(quince_icloud::drive::download_to(
                &remote.http,
                &remote.docws,
                remote.params.clone(),
                &item,
                Path::new(&partial),
            ))
            .map_err(failed)?;
        // Whole or not at all: an app never opens half a file.
        std::fs::rename(&partial, &path).map_err(failed)
    }

    /// A new file sent to iCloud: `staged` holds its bytes, named as the file is to be called,
    /// for `folder` (a new document, an upload, Send to iPhone). One of that name there already
    /// is kept, and this goes beside it as a conflicted copy. A file an app opened and wrote is
    /// held and sent against the version it was opened at instead ([`Account::drive_write_open`]):
    /// a base looked up only now could be another device's newer version, and be written over.
    pub fn drive_write_back(&self, folder: String, staged: String) -> Result<DItem, EngineError> {
        let remote = self.drive()?;
        let folder_item = self.known(&folder)?;
        let sent = runtime()
            .block_on(quince_vfs::writes::send(
                &remote,
                &folder_item,
                Path::new(&staged),
                None,
                None,
            ))
            .map_err(failed)?;
        // Kept, so the Files app lists it at once.
        let _ = runtime().block_on(remote.list(&folder));
        Ok(ditem(&sent.item))
    }

    /// A new folder in `parent`.
    pub fn drive_mkdir(&self, parent: String, name: String) -> Result<DItem, EngineError> {
        let remote = self.drive()?;
        let parent_item = self.known(&parent)?;
        let made = runtime()
            .block_on(remote.mkdir(&parent_item, &name))
            .map_err(failed)?;
        let _ = runtime().block_on(remote.list(&parent));
        Ok(ditem(&made))
    }

    /// Renamed in its folder.
    pub fn drive_rename(&self, id: String, name: String) -> Result<(), EngineError> {
        let remote = self.drive()?;
        let item = self.known(&id)?;
        runtime()
            .block_on(remote.rename(&item, &name))
            .map_err(failed)?;
        if let Some(parent) = item.parent_id {
            let _ = runtime().block_on(remote.list(&parent));
        }
        Ok(())
    }

    /// Moved into another folder.
    pub fn drive_move(&self, id: String, folder: String) -> Result<(), EngineError> {
        let remote = self.drive()?;
        let item = self.known(&id)?;
        let to = self.known(&folder)?;
        runtime()
            .block_on(remote.move_to(&item, &to))
            .map_err(failed)?;
        for f in [item.parent_id.clone().unwrap_or_default(), folder] {
            if !f.is_empty() {
                let _ = runtime().block_on(remote.list(&f));
            }
        }
        Ok(())
    }

    /// Deleted: into iCloud Drive's Recently Deleted, where it can be put back for 30 days.
    pub fn drive_delete(&self, id: String) -> Result<(), EngineError> {
        let remote = self.drive()?;
        let item = self.known(&id)?;
        runtime().block_on(remote.delete(&item)).map_err(failed)?;
        if let Some(parent) = item.parent_id {
            let _ = runtime().block_on(remote.list(&parent));
        }
        Ok(())
    }

    /// iCloud storage, `[used, total]` in bytes.
    pub fn drive_space(&self) -> Result<Vec<u64>, EngineError> {
        let remote = self.drive()?;
        let (used, total) = runtime().block_on(remote.space()).map_err(failed)?;
        Ok(vec![used, total])
    }
}

/// A file's key in the write journal: its document id, hashed the same on every run (FNV-1a),
/// since the journal is keyed by number and outlives the process.
fn key_of(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Whether a copy on the phone holds `item`'s bytes: its length, and written no earlier than
/// Apple last changed the file. Checked against the very version that becomes a write's base,
/// in the same step, so the base can never be newer than the bytes it is sent over.
fn is_copy_of(copy: &Path, item: &DriveItem) -> bool {
    let Ok(m) = std::fs::metadata(copy) else {
        return false;
    };
    let written = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0);
    item.size
        .and_then(|s| u64::try_from(s).ok())
        .is_some_and(|s| s == m.len())
        && written >= item.modified_at.unwrap_or(0)
}

/// A Drive file opened for reading by another app, read in parts as it asks for them: each
/// 4 MB chunk fetched from Apple once, into the parts kept on the phone (`quince_vfs::Cache`,
/// the desktop folder's own), and served from there. Fixed to the version opened, so one
/// reading never mixes two versions of the file.
#[derive(uniffi::Object)]
pub struct DriveReader {
    item: DriveItem,
    cache: Arc<quince_vfs::Cache>,
    remote: PhoneRemote,
}

#[uniffi::export]
impl DriveReader {
    /// The file's length as served: a Pages, Numbers or Keynote package's is asked of Apple.
    pub fn size(&self) -> Result<u64, EngineError> {
        runtime()
            .block_on(self.cache.sized(&self.remote, &self.item))
            .map_err(failed)
    }

    /// `len` bytes from `offset`, fewer only at the end of the file.
    pub fn read(&self, offset: u64, len: u32) -> Result<Vec<u8>, EngineError> {
        runtime()
            .block_on(
                self.cache
                    .read(&self.remote, &self.item, offset, u64::from(len)),
            )
            .map_err(failed)
    }
}

/// Apple's picture of a Drive file, for the Files app's thumbnails.
#[derive(uniffi::Record)]
pub struct DThumb {
    pub id: String,
    pub image: Vec<u8>,
}

/// How sending a held file went.
#[derive(uniffi::Enum)]
pub enum WriteSent {
    /// Nothing was waiting for that file.
    NotHeld,
    /// An app still has it open.
    StillOpen,
    /// At Apple: `settled` when nothing wrote to it meanwhile; otherwise it goes again.
    Sent {
        item: DItem,
        conflicted: bool,
        settled: bool,
    },
}

impl Account {
    /// The parts read for this account, opened where the app said ([`Account::drive_parts_at`]).
    fn parts(&self) -> Result<Arc<quince_vfs::Cache>, EngineError> {
        let (_, account) = self.library()?;
        let (dir, budget) = self
            .drive_parts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or_else(|| failed("Drive's parts have nowhere to go yet."))?;
        let dir = dir.join(&account);
        let mut held = self
            .drive_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Android may empty the app's cache while it runs: a cache whose folder went is opened
        // again, never trusted to still hold the parts it remembers.
        if let Some((who, cache)) = held.as_ref() {
            if *who == account && dir.exists() {
                return Ok(Arc::clone(cache));
            }
        }
        let cache = quince_vfs::Cache::new(&dir)
            .map_err(failed)?
            .with_budget(budget);
        // What earlier runs left beyond the budget is let go of now.
        cache.evict();
        let cache = Arc::new(cache);
        *held = Some((account, Arc::clone(&cache)));
        Ok(cache)
    }

    /// The write journal for this account: in its library's folder (the app's private files,
    /// never the cache Android may empty), so Remove from Phone takes it with the library and
    /// signing out keeps it for the same account's return.
    fn writes(&self) -> Result<Arc<quince_vfs::writes::Writes>, EngineError> {
        let (_, account) = self.library()?;
        let mut held = self
            .drive_writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((who, writes)) = held.as_ref() {
            if *who == account {
                return Ok(Arc::clone(writes));
            }
        }
        let dir = self
            .dir
            .join("libraries")
            .join(&account)
            .join("drive-writes");
        let writes = quince_vfs::writes::Writes::new(&dir).map_err(failed)?;
        writes.resume();
        let writes = Arc::new(writes);
        *held = Some((account, Arc::clone(&writes)));
        Ok(writes)
    }

    /// The session's Drive state let go of: the parts read (deleted, as the cache they are) and
    /// the journal closed (its files stay with the library they belong to).
    pub(crate) fn forget_drive_parts(&self) {
        self.drive_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.drive_writes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let at = self
            .drive_parts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some((dir, _)) = at {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[uniffi::export]
impl Account {
    /// Where Drive's parts read for the Files app are kept (the app's cache) and the most disk
    /// they may take; past it, those read longest ago are let go of.
    pub fn drive_parts_at(&self, dir: String, budget: u64) {
        *self
            .drive_parts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((dir.into(), budget));
        self.drive_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }

    /// A file to read in parts, at the version the phone has listed.
    pub fn drive_reader(&self, id: String) -> Result<Arc<DriveReader>, EngineError> {
        let item = self.known(&id)?;
        if item.is_container() {
            return Err(failed("That's a folder."));
        }
        Ok(Arc::new(DriveReader {
            item,
            cache: self.parts()?,
            remote: self.drive()?,
        }))
    }

    /// Apple's pictures of these files, for those Apple draws (images, films, Office and iWork
    /// documents among them): one request for them all, then each picture. Files Apple has no
    /// picture of are left out.
    pub fn drive_thumbnails(&self, ids: Vec<String>) -> Result<Vec<DThumb>, EngineError> {
        let files: Vec<DriveItem> = ids.iter().filter_map(|id| self.known(id).ok()).collect();
        let remote = self.drive()?;
        let got = runtime()
            .block_on(quince_icloud::drive::thumbnails(
                &remote.http,
                &remote.docws,
                remote.params.clone(),
                &files,
            ))
            .map_err(failed)?;
        Ok(got
            .into_iter()
            .map(|(id, image)| DThumb { id, image })
            .collect())
    }

    /// A file opened by another app to write: held in the journal from now, at the version
    /// opened, and answered with where its bytes are to be written. `copy` is the phone's copy
    /// of it, used when it is that version; otherwise Apple's is fetched (none when `truncate`:
    /// the app writes it whole). Opened again while still held, it is the same bytes and the
    /// same base: the version first opened, whatever the library lists by then.
    pub fn drive_write_open(
        &self,
        id: String,
        copy: Option<String>,
        truncate: bool,
    ) -> Result<String, EngineError> {
        let writes = self.writes()?;
        let key = key_of(&id);
        if let Some(held) = writes.get(key) {
            let staged = writes.staging(key, &held.name);
            if truncate {
                std::fs::write(&staged, b"").map_err(failed)?;
            }
            writes.hold(key, |p| {
                let p = p.unwrap_or(held);
                quince_vfs::writes::Pending {
                    open: p.open + 1,
                    generation: p.generation + 1,
                    dirty: true,
                    modified: std::time::SystemTime::now(),
                    ..p
                }
            });
            return Ok(staged.to_string_lossy().into_owned());
        }
        let item = self.known(&id)?;
        let folder = item
            .parent_id
            .clone()
            .ok_or_else(|| failed("That file has no folder in iCloud Drive."))?;
        let name = item.display_name();
        let staged = writes.staging(key, &name);
        if let Some(dir) = staged.parent() {
            std::fs::create_dir_all(dir).map_err(failed)?;
        }
        // The bytes first, then the journal: a journal entry always names bytes on disk.
        match copy.as_deref().map(Path::new) {
            _ if truncate => std::fs::write(&staged, b"").map_err(failed)?,
            Some(copy) if is_copy_of(copy, &item) => {
                std::fs::copy(copy, &staged).map_err(failed)?;
            }
            _ => {
                let remote = self.drive()?;
                let partial = staged.with_extension("download");
                runtime()
                    .block_on(quince_icloud::drive::download_to(
                        &remote.http,
                        &remote.docws,
                        remote.params.clone(),
                        &item,
                        &partial,
                    ))
                    .map_err(failed)?;
                std::fs::rename(&partial, &staged).map_err(failed)?;
            }
        }
        writes.hold(key, |_| quince_vfs::writes::Pending {
            folder,
            name,
            replaces: Some(item),
            open: 1,
            dirty: true,
            generation: 1,
            modified: std::time::SystemTime::now(),
            sender: false,
            quiet_since: None,
            sending: false,
            failure: None,
        });
        Ok(staged.to_string_lossy().into_owned())
    }

    /// The app writing a held file closed it: once every app has, it can go.
    pub fn drive_write_closed(&self, id: String) -> Result<(), EngineError> {
        let writes = self.writes()?;
        let key = key_of(&id);
        if let Some(held) = writes.get(key) {
            writes.hold(key, |p| {
                let p = p.unwrap_or(held);
                quince_vfs::writes::Pending {
                    open: p.open.saturating_sub(1),
                    modified: std::time::SystemTime::now(),
                    ..p
                }
            });
        }
        Ok(())
    }

    /// Where a held file's bytes are, while it waits to go: the newest there is of it, read
    /// from there and never fetched over.
    pub fn drive_write_path(&self, id: String) -> Result<Option<String>, EngineError> {
        let writes = self.writes()?;
        let key = key_of(&id);
        Ok(writes
            .get(key)
            .map(|p| writes.staging(key, &p.name).to_string_lossy().into_owned()))
    }

    /// Every file waiting to go, by its document id.
    pub fn drive_writes_waiting(&self) -> Result<Vec<String>, EngineError> {
        let writes = self.writes()?;
        Ok(writes
            .keys()
            .into_iter()
            .filter_map(|k| writes.get(k)?.replaces.map(|r| r.drivewsid))
            .collect())
    }

    /// A held file sent, against the version it was opened at (`quince_vfs::writes::send_held`).
    /// Once it has gone and nothing wrote to it meanwhile, its bytes become the phone's copy at
    /// `keep_to`.
    pub fn drive_write_send(
        &self,
        id: String,
        keep_to: Option<String>,
    ) -> Result<WriteSent, EngineError> {
        let writes = self.writes()?;
        let key = key_of(&id);
        let Some(held) = writes.get(key) else {
            return Ok(WriteSent::NotHeld);
        };
        let remote = self.drive()?;
        let folder = self.known(&held.folder)?;
        let sent = runtime()
            .block_on(quince_vfs::writes::send_held(
                &remote,
                &writes,
                key,
                &folder,
                keep_to.as_deref().map(Path::new),
            ))
            .map_err(failed)?;
        Ok(match sent {
            quince_vfs::writes::Held::Nothing => WriteSent::NotHeld,
            quince_vfs::writes::Held::Open => WriteSent::StillOpen,
            quince_vfs::writes::Held::Sent { sent, settled } => {
                // Kept, so the Files app lists it as Apple has it now.
                let _ = runtime().block_on(remote.list(&held.folder));
                WriteSent::Sent {
                    item: ditem(&sent.item),
                    conflicted: sent.conflicted,
                    settled,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The journal's key for a document is the same on every run (it outlives the process),
    /// and differs between documents.
    #[test]
    fn a_documents_key_is_stable() {
        assert_eq!(key_of(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(key_of("a"), 0xaf63_dc4c_8601_ec8c);
        assert_ne!(key_of("FILE::x::A"), key_of("FILE::x::B"));
    }

    /// A copy is the base's bytes only at its length and no older than Apple's change.
    #[test]
    fn a_copy_is_the_versions_only_when_it_matches() {
        let dir = std::env::temp_dir().join(format!("skykeep-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("a.txt");
        std::fs::write(&copy, b"12345").unwrap();
        let mut item = DriveItem {
            drivewsid: "FILE::x".into(),
            parent_id: None,
            name: "a".into(),
            extension: Some("txt".into()),
            kind: "FILE".into(),
            size: Some(5),
            child_count: None,
            created_at: None,
            modified_at: Some(1),
            etag: None,
        };
        assert!(is_copy_of(&copy, &item));
        item.size = Some(6);
        assert!(!is_copy_of(&copy, &item), "another length");
        item.size = Some(5);
        item.modified_at = Some(i64::MAX / 2);
        assert!(
            !is_copy_of(&copy, &item),
            "changed at Apple after the copy was written"
        );
        assert!(!is_copy_of(&dir.join("none"), &item));
        let _ = std::fs::remove_dir_all(dir);
    }
}
