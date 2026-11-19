//! Live, with a file made for the purpose (a large 4K test video, nothing of his): uploaded
//! through Quince's own uploader, its original downloaded back from Apple and compared byte for
//! byte by SHA-256, then deleted for good. Proves a heavy upload arrives whole.
use anyhow::{anyhow, Result};
use quince_icloud::photos::{download_choice, set_flag, Flag, Want, CONTAINER};
use quince_icloud::photos_upload::{Outcome, Stage};
use sha2::{Digest, Sha256};

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::path::PathBuf::from(std::env::args().nth(1).ok_or_else(|| anyhow!("file"))?);
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    if !name.starts_with("quince-heavy-upload-test") {
        return Err(anyhow!("only the test file made for this"));
    }
    let local = format!("{:x}", Sha256::digest(std::fs::read(&path)?));
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let up = client
        .session()
        .service_url("photosupload")
        .ok_or_else(|| anyhow!("no photosupload"))?;
    let ck = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let dsid = p
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("no dsid"))?;
    let t = std::time::Instant::now();
    let last = std::sync::Arc::new(std::sync::Mutex::new(0u64));
    let progress: std::sync::Arc<dyn Fn(Stage) + Send + Sync> =
        std::sync::Arc::new(move |s| match s {
            Stage::Sending(sent, total) => {
                let tenth = sent * 10 / total.max(1);
                let mut l = last.lock().unwrap();
                if tenth > *l || sent == total {
                    *l = tenth;
                    println!(
                        "  {:>6.1}s sent {} of {} MB",
                        t.elapsed().as_secs_f64(),
                        sent / 1_048_576,
                        total / 1_048_576
                    );
                }
            }
            other => println!("  {:>6.1}s {other:?}", t.elapsed().as_secs_f64()),
        });
    let modified = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let library = quince_icloud::photos::library_zone(&quince_icloud::cloudkit::ZoneClient::new(
        &http,
        &ck,
        quince_icloud::photos::CONTAINER,
        p.clone(),
    ))
    .await;
    let outcome = quince_icloud::photos_upload::upload(
        &http,
        &http,
        &up,
        &dsid,
        &library,
        &path,
        &name,
        modified,
        "Asia/Kolkata",
        -330,
        &uuid::Uuid::new_v4().to_string(),
        progress,
    )
    .await?;
    let Outcome::Uploaded(u) = outcome else {
        return Err(anyhow!("not uploaded: {outcome:?}"));
    };
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &ck, CONTAINER, p.clone());
    let zone = quince_icloud::photos::library_zone(&zc).await;
    let asset = zc
        .lookup_record(&zone, &u.asset)
        .await?
        .ok_or_else(|| anyhow!("asset missing"))?;
    let master = zc
        .lookup_record(&zone, &u.master)
        .await?
        .ok_or_else(|| anyhow!("master missing"))?;
    let (url, back_name) = download_choice(&asset, &master, Want::Originals, true)
        .ok_or_else(|| anyhow!("no original"))?;
    let bytes = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let remote = format!("{:x}", Sha256::digest(&bytes));
    println!(
        "uploaded in {:.1}s; original back as {back_name}, {} bytes",
        t.elapsed().as_secs_f64(),
        bytes.len()
    );
    println!(
        "sha256 here  {local}\nsha256 Apple {remote}\nidentical: {}",
        local == remote
    );
    // Gone for good: to Recently Deleted, then expunged.
    let tag = asset.get("recordChangeTag").and_then(|x| x.as_str());
    let deleted = set_flag(
        &http,
        &ck,
        p.clone(),
        None,
        &u.asset,
        tag,
        Flag::Deleted,
        true,
    )
    .await?;
    let tag = deleted
        .get("recordChangeTag")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let now = zc.lookup_record(&zone, &u.asset).await?.unwrap();
    let tag = tag.or_else(|| {
        now.get("recordChangeTag")
            .and_then(|x| x.as_str())
            .map(str::to_string)
    });
    set_flag(
        &http,
        &ck,
        p,
        None,
        &u.asset,
        tag.as_deref(),
        Flag::Expunged,
        true,
    )
    .await?;
    let gone = zc.lookup_record(&zone, &u.asset).await?;
    println!(
        "deleted for good: {:?}",
        gone.and_then(|r| r.pointer("/fields/isExpunged/value").cloned())
    );
    Ok(())
}
