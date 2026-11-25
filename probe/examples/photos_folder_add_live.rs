//! Live: the Photos folder's add-only mode (quince-vfs `add_only`) against real iCloud Photos,
//! through the same uploader and delete the app uses. Mounts a small Photos folder — Library
//! (takes new photos) and Favorites (a view: takes nothing) — then, as any program would:
//!
//! - copies the test JPEG into Library: it goes up and is confirmed in the library;
//! - tries what a Photos folder must refuse: a folder made, a photo written over, renamed, a
//!   file put in Favorites — each "read-only file system";
//! - deletes it: into Recently Deleted, confirmed from Apple.
//!
//! The file must be named quince-upload-test.jpg (photos_expunge deletes it for good after).
use anyhow::{anyhow, Result};
use quince_core::drive::DriveItem;
use quince_icloud::photos::{Flag, CONTAINER};
use quince_vfs::{fuse, Cache, Fut, Remote, Store, ROOT};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const LIBRARY: &str = "PH::library";
const FAVORITES: &str = "PH::favorites";

struct Ctx {
    http: reqwest::Client,
    ck: String,
    up: String,
    dsid: String,
    params: Vec<(String, String)>,
}

#[derive(Clone)]
struct Photos {
    ctx: Arc<Ctx>,
    /// What this run added, as the library would list it.
    added: Arc<Mutex<HashMap<String, DriveItem>>>,
}

fn folder(id: &str, name: &str) -> DriveItem {
    DriveItem {
        drivewsid: id.into(),
        parent_id: Some(ROOT.into()),
        name: name.into(),
        extension: None,
        kind: "FOLDER".into(),
        size: None,
        child_count: None,
        created_at: None,
        modified_at: None,
        etag: None,
        share: None,
    }
}

impl Photos {
    fn children(&self, id: &str) -> Vec<DriveItem> {
        match id {
            ROOT => vec![folder(LIBRARY, "Library"), folder(FAVORITES, "Favorites")],
            LIBRARY => self.added.lock().unwrap().values().cloned().collect(),
            _ => vec![],
        }
    }
}

impl Store for Photos {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        Some(Photos::children(self, folder))
    }
}

impl Remote for Photos {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let items = self.children(folder);
        Box::pin(async move { Ok(items) })
    }
    fn read(&self, _: &DriveItem, _: u64, _: u64) -> Fut<Vec<u8>> {
        Box::pin(async { Err("not read here".to_string()) })
    }
    fn accepts(&self, folder: &DriveItem) -> bool {
        folder.drivewsid == LIBRARY
    }
    fn upload(&self, folder: &DriveItem, path: &std::path::Path) -> Fut<DriveItem> {
        let (me, folder, path) = (self.clone(), folder.drivewsid.clone(), path.to_path_buf());
        Box::pin(async move {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let c = &me.ctx;
            let library =
                quince_icloud::photos::library_zone(&quince_icloud::cloudkit::ZoneClient::new(
                    &c.http,
                    &c.ck,
                    quince_icloud::photos::CONTAINER,
                    c.params.clone(),
                ))
                .await;
            let outcome = quince_icloud::photos_upload::upload(
                &c.http,
                &c.http,
                &c.up,
                &c.dsid,
                &library,
                &path,
                &name,
                1_792_521_600_000,
                "Asia/Kolkata",
                -330,
                &uuid::Uuid::new_v4().to_string(),
                Arc::new(|_| {}),
            )
            .await
            .map_err(|e| e.to_string())?;
            let quince_icloud::photos_upload::Outcome::Uploaded(u) = outcome else {
                return Err(format!("refused: {outcome:?}"));
            };
            println!("uploaded through the folder: asset {}", u.asset);
            let (stem, ext) = name.rsplit_once('.').unwrap();
            let item = DriveItem {
                drivewsid: format!("PHOTO::{}", u.asset),
                parent_id: Some(folder),
                name: stem.into(),
                extension: Some(ext.into()),
                kind: "FILE".into(),
                size: std::fs::metadata(&path).ok().map(|m| m.len() as i64),
                child_count: None,
                created_at: None,
                modified_at: None,
                etag: None,
                share: None,
            };
            me.added.lock().unwrap().insert(u.asset, item.clone());
            Ok(item)
        })
    }
    fn delete(&self, item: &DriveItem) -> Fut<()> {
        let (me, item) = (self.clone(), item.clone());
        Box::pin(async move {
            let asset = item.drivewsid.trim_start_matches("PHOTO::").to_string();
            let c = &me.ctx;
            let zc = quince_icloud::cloudkit::ZoneClient::new(
                &c.http,
                &c.ck,
                CONTAINER,
                c.params.clone(),
            );
            let zone = quince_icloud::photos::library_zone(&zc).await;
            let rec = zc
                .lookup_record(&zone, &asset)
                .await
                .map_err(|e| e.to_string())?
                .ok_or("gone")?;
            quince_icloud::photos::set_flag(
                &c.http,
                &c.ck,
                c.params.clone(),
                None,
                &asset,
                rec["recordChangeTag"].as_str(),
                Flag::Deleted,
                true,
            )
            .await
            .map_err(|e| e.to_string())?;
            me.added.lock().unwrap().remove(&asset);
            Ok(())
        })
    }
}

/// EROFS, "read-only file system" (30 on Linux).
fn erofs<T>(what: &str, r: std::io::Result<T>) {
    let ok = matches!(&r, Err(e) if e.raw_os_error() == Some(30));
    println!("{} {what}: {:?}", if ok { "PASS" } else { "FAIL" }, r.err());
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (jpeg, at) = (
        std::path::PathBuf::from(args.next().ok_or_else(|| anyhow!("jpeg"))?),
        std::path::PathBuf::from(args.next().ok_or_else(|| anyhow!("mount point"))?),
    );
    let rt = tokio::runtime::Runtime::new()?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let ctx = rt.block_on(async {
        let mut client = quince_auth::Client::new(&dir)?;
        if !client.resume().await? {
            return Err(anyhow!("no session"));
        }
        let s = client.session();
        let p = client.service_params();
        Ok::<_, anyhow::Error>(Ctx {
            http: client.http().clone(),
            ck: s
                .service_url("ckdatabasews")
                .ok_or_else(|| anyhow!("no ck"))?,
            up: s
                .service_url("photosupload")
                .ok_or_else(|| anyhow!("no upload"))?,
            dsid: p
                .iter()
                .find(|(k, _)| k == "dsid")
                .map(|(_, v)| v.clone())
                .unwrap_or_default(),
            params: p,
        })
    })?;
    let photos = Photos {
        ctx: Arc::new(ctx),
        added: Arc::default(),
    };
    let scratch = std::env::temp_dir().join(format!("quince-photos-add-{}", std::process::id()));
    std::fs::create_dir_all(&at)?;
    let fs = fuse::DriveFs::new(
        Arc::new(photos.clone()),
        Arc::new(photos.clone()),
        Arc::new(Cache::new(&scratch.join("cache"))?),
        rt.handle().clone(),
    )
    .add_only()
    .with_writes(&scratch.join("writes"))?;
    let writes = fs.writes().ok_or_else(|| anyhow!("no writes"))?;
    let session = fuse::mount(fs, &at)?;
    let lib = at.join("Library");
    let target = lib.join("quince-upload-test.jpg");

    std::fs::copy(&jpeg, &target)?;
    println!("copied in");
    let started = std::time::Instant::now();
    while writes.waiting().files > 0 && started.elapsed().as_secs() < 120 {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let asset = photos.added.lock().unwrap().keys().next().cloned();
    println!(
        "{} sent within {:.1}s",
        if asset.is_some() { "PASS" } else { "FAIL" },
        started.elapsed().as_secs_f64()
    );
    let asset = asset.ok_or_else(|| anyhow!("never sent"))?;
    let found = rt.block_on(async {
        let c = &photos.ctx;
        let zc =
            quince_icloud::cloudkit::ZoneClient::new(&c.http, &c.ck, CONTAINER, c.params.clone());
        let zone = quince_icloud::photos::library_zone(&zc).await;
        zc.lookup_record(&zone, &asset).await
    })?;
    println!(
        "{} in iCloud Photos",
        if found.is_some() { "PASS" } else { "FAIL" }
    );
    println!(
        "{} listed in Library as {:?}",
        if target.exists() { "PASS" } else { "FAIL" },
        std::fs::read_dir(&lib)?
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );

    erofs("no folders made", std::fs::create_dir(lib.join("New")));
    erofs(
        "a photo never written over",
        std::fs::OpenOptions::new().write(true).open(&target),
    );
    erofs(
        "nor renamed",
        std::fs::rename(&target, lib.join("other.jpg")),
    );
    erofs(
        "nothing put in a view",
        std::fs::write(at.join("Favorites").join("x.jpg"), b"x"),
    );

    std::fs::remove_file(&target)?;
    let after = rt.block_on(async {
        let c = &photos.ctx;
        let zc =
            quince_icloud::cloudkit::ZoneClient::new(&c.http, &c.ck, CONTAINER, c.params.clone());
        let zone = quince_icloud::photos::library_zone(&zc).await;
        zc.lookup_record(&zone, &asset).await
    })?;
    let deleted = after
        .as_ref()
        .and_then(|r| r.pointer("/fields/isDeleted/value"))
        .and_then(serde_json::Value::as_i64)
        == Some(1);
    println!(
        "{} deleted through the folder: in Recently Deleted",
        if deleted { "PASS" } else { "FAIL" }
    );
    drop(session);
    let _ = std::fs::remove_dir_all(&scratch);
    println!("ASSET {asset}");
    Ok(())
}
