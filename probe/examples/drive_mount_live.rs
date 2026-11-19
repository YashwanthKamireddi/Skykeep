//! Live, read-only: his iCloud Drive mounted in a scratch folder (the first argument) with the
//! app's own engine (quince-vfs), for a minute or the seconds given second — listed through
//! Apple's folder API, read through its signed links in byte ranges. Nothing is written to
//! iCloud. The folder is let go of at the end.
use anyhow::{anyhow, Result};
use quince_core::drive::DriveItem;
use quince_vfs::{fuse, Cache, Fut, Remote, Store};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct Apple {
    http: reqwest::Client,
    drivews: String,
    docws: String,
    params: Vec<(String, String)>,
    library: Arc<Mutex<HashMap<String, Vec<DriveItem>>>>,
    links: Arc<Mutex<HashMap<String, String>>>,
}

impl Remote for Apple {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        let (http, root, p, folder, library) = (
            self.http.clone(),
            self.drivews.clone(),
            self.params.clone(),
            folder.to_string(),
            Arc::clone(&self.library),
        );
        Box::pin(async move {
            let items = quince_icloud::drive::fetch_folder(&http, &root, p, &folder)
                .await
                .map_err(|e| e.to_string())?;
            library.lock().unwrap().insert(folder, items.clone());
            Ok(items)
        })
    }
    fn mkdir(&self, parent: &DriveItem, name: &str) -> Fut<DriveItem> {
        let (http, root, p, parent, name) = (
            self.http.clone(),
            self.drivews.clone(),
            self.params.clone(),
            parent.drivewsid.clone(),
            name.to_string(),
        );
        Box::pin(async move {
            quince_icloud::drive::create_folder(&http, &root, p, &parent, &name)
                .await
                .map_err(|e| e.to_string())
        })
    }
    fn rename(&self, item: &DriveItem, name: &str) -> Fut<()> {
        let (http, root, p, item, name) = (
            self.http.clone(),
            self.drivews.clone(),
            self.params.clone(),
            item.clone(),
            name.to_string(),
        );
        Box::pin(async move {
            quince_icloud::drive::rename(&http, &root, p, &item, &name)
                .await
                .map_err(|e| e.to_string())
        })
    }
    fn move_to(&self, item: &DriveItem, folder: &DriveItem) -> Fut<()> {
        let (http, root, p, item, to) = (
            self.http.clone(),
            self.drivews.clone(),
            self.params.clone(),
            item.clone(),
            folder.drivewsid.clone(),
        );
        Box::pin(async move {
            quince_icloud::drive::move_to(&http, &root, p, &item, &to)
                .await
                .map_err(|e| e.to_string())
        })
    }
    fn upload(&self, folder: &DriveItem, path: &std::path::Path) -> Fut<DriveItem> {
        let (http, docws, drivews, p, folder, path) = (
            self.http.clone(),
            self.docws.clone(),
            self.drivews.clone(),
            self.params.clone(),
            folder.drivewsid.clone(),
            path.to_path_buf(),
        );
        Box::pin(async move {
            let progress: quince_icloud::drive::Progress = Arc::new(|_, _| {});
            quince_icloud::drive::upload(
                &http, &http, &docws, &drivews, p, &folder, &path, progress,
            )
            .await
            .map_err(|e| e.to_string())
        })
    }
    fn delete(&self, item: &DriveItem) -> Fut<()> {
        let (http, root, p, item) = (
            self.http.clone(),
            self.drivews.clone(),
            self.params.clone(),
            item.clone(),
        );
        Box::pin(async move {
            quince_icloud::drive::delete(&http, &root, p, &item)
                .await
                .map_err(|e| e.to_string())
        })
    }
    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        let known = self.links.lock().unwrap().get(&item.drivewsid).cloned();
        let (http, docws, p, item) = (
            self.http.clone(),
            self.docws.clone(),
            self.params.clone(),
            item.clone(),
        );
        let links = Arc::clone(&self.links);
        Box::pin(async move {
            let url = match known {
                Some(u) => u,
                None => {
                    let u = quince_icloud::drive::download_url(&http, &docws, p, &item)
                        .await
                        .map_err(|e| e.to_string())?;
                    links
                        .lock()
                        .unwrap()
                        .insert(item.drivewsid.clone(), u.clone());
                    u
                }
            };
            let r = http
                .get(&url)
                .header("Range", format!("bytes={offset}-{}", offset + len - 1))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            Ok(r.bytes().await.map_err(|e| e.to_string())?.to_vec())
        })
    }
}

struct Library(Arc<Mutex<HashMap<String, Vec<DriveItem>>>>);
impl Store for Library {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        self.0.lock().unwrap().get(folder).cloned()
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let at = std::path::PathBuf::from(args.next().ok_or_else(|| anyhow!("mount folder"))?);
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let rt = tokio::runtime::Runtime::new()?;
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let (http, drivews, docws, params) = rt.block_on(async {
        let mut client = quince_auth::Client::new(&dir)?;
        if !client.resume().await? {
            return Err(anyhow!("no session"));
        }
        let s = client.session();
        let drivews = s
            .service_url("drivews")
            .ok_or_else(|| anyhow!("no drivews"))?;
        let docws = s.service_url("docws").ok_or_else(|| anyhow!("no docws"))?;
        anyhow::Ok((
            client.http().clone(),
            drivews,
            docws,
            client.service_params(),
        ))
    })?;
    let (apple_http, apple_drivews, apple_params) = (http.clone(), drivews.clone(), params.clone());
    let library = Arc::new(Mutex::new(HashMap::new()));
    let apple = Arc::new(Apple {
        http,
        drivews,
        docws,
        params,
        library: Arc::clone(&library),
        links: Arc::new(Mutex::new(HashMap::new())),
    });
    let cache = tempfile_dir()?;
    let writes_dir =
        std::env::temp_dir().join(format!("quince-mount-writes-{}", std::process::id()));
    let fs = fuse::DriveFs::new(
        apple,
        Arc::new(Library(library)),
        Arc::new(Cache::new(&cache)?),
        rt.handle().clone(),
    )
    .with_writes(&writes_dir)?;
    let session = fuse::mount(fs, &at)?;
    println!("mounted at {}", at.display());
    std::thread::sleep(std::time::Duration::from_secs(secs));
    drop(session);
    let _ = std::fs::remove_dir_all(&cache);
    // Its journal left behind only when something is still waiting.
    if std::fs::read(writes_dir.join("pending.json")).is_ok_and(|b| b == b"[]") {
        let _ = std::fs::remove_dir_all(&writes_dir);
    }
    println!("unmounted");
    // This probe's own throwaways, and nothing else, cleared from Recently Deleted.
    if std::env::var("QUINCE_PURGE_TEST").is_ok() {
        let (http, drivews, params) = (apple_http, apple_drivews, apple_params);
        rt.block_on(async {
            let trash = quince_icloud::drive::trash(&http, &drivews, params.clone()).await?;
            let mine: Vec<_> = trash
                .into_iter()
                .filter(|i| i.name.starts_with("Quince mount test"))
                .collect();
            println!("throwaways in Recently Deleted: {}", mine.len());
            if !mine.is_empty() {
                quince_icloud::drive::delete_forever_many(&http, &drivews, params, &mine).await?;
            }
            anyhow::Ok(())
        })?;
    }
    Ok(())
}

fn tempfile_dir() -> Result<std::path::PathBuf> {
    let d = std::env::temp_dir().join(format!("quince-mount-cache-{}", std::process::id()));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}
