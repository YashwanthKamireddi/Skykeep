//! Live, throwaway only: "Quince folder upload test" made in the root, 24 tiny files sent into
//! it three at a time as the app sends them — each one timed — and the folder deleted while
//! they are still going. Then: does the folder stay gone? Everything is left in Recently
//! Deleted and purged at the end.
use anyhow::{anyhow, Result};
use futures_util::StreamExt;
use quince_icloud::drive;
const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let docws = client
        .session()
        .service_url("docws")
        .ok_or_else(|| anyhow!("no docws"))?;
    let dws = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let folder =
        drive::create_folder(&http, &dws, p.clone(), ROOT, "Quince folder upload test").await?;
    let tmp = std::env::temp_dir().join("quince-folder-test");
    std::fs::create_dir_all(&tmp)?;
    let files: Vec<_> = (0..24)
        .map(|n| {
            let f = tmp.join(format!("file {n}.txt"));
            std::fs::write(&f, format!("safe to delete {n}")).unwrap();
            f
        })
        .collect();
    let start = std::time::Instant::now();
    let deleting = {
        let (http, dws, p, folder) = (http.clone(), dws.clone(), p.clone(), folder.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            let t = std::time::Instant::now();
            let r = drive::delete(&http, &dws, p, &folder).await;
            println!(
                "delete at 8s: {:?} after {:?}",
                r.as_ref().map(|_| "ok").map_err(|e| e.to_string()),
                t.elapsed()
            );
        })
    };
    let results: Vec<_> = futures_util::stream::iter(files.iter().cloned().enumerate())
        .map(|(n, f)| {
            let (http, docws, dws, p, id) = (
                http.clone(),
                docws.clone(),
                dws.clone(),
                p.clone(),
                folder.drivewsid.clone(),
            );
            async move {
                let t = std::time::Instant::now();
                let r = drive::upload(
                    &http,
                    &http,
                    &docws,
                    &dws,
                    p,
                    &id,
                    &f,
                    std::sync::Arc::new(|_, _| {}),
                )
                .await;
                (
                    n,
                    t.elapsed(),
                    start.elapsed(),
                    r.map(|_| ()).map_err(|e| e.to_string()),
                )
            }
        })
        .buffer_unordered(3)
        .collect()
        .await;
    deleting.await?;
    let mut ok = 0;
    for (n, took, at, r) in &results {
        if r.is_ok() {
            ok += 1;
        }
        if *n < 4 || r.is_err() {
            println!("file {n}: {:?} (finished at {:?}) {:?}", took, at, r);
        }
    }
    let avg: f64 = results.iter().map(|r| r.1.as_secs_f64()).sum::<f64>() / results.len() as f64;
    println!(
        "{ok}/24 uploaded, average {avg:.1}s each, all done at {:?}",
        start.elapsed()
    );
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let mut lib = quince_core::Library::in_memory()?;
    drive::sync_folder(&http, &dws, p.clone(), &mut lib, ROOT).await?;
    let back = lib
        .drive_children(ROOT)?
        .into_iter()
        .any(|i| i.drivewsid == folder.drivewsid);
    println!("folder back in the root afterwards: {back}");
    // Clean up wherever it is.
    if back {
        let _ = drive::delete(&http, &dws, p.clone(), &folder).await;
    }
    let t = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| i.drivewsid == folder.drivewsid)
        .collect::<Vec<_>>();
    println!(
        "purged: {:?}",
        drive::delete_forever_many(&http, &dws, p.clone(), &t).await?
    );
    Ok(())
}
