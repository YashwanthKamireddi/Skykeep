//! Live: Quince's own uploader (quince_icloud::photos_upload) sends the small test JPEG, with
//! its progress printed; the photo is confirmed in the library and then sent to Recently
//! Deleted. Run twice in a row, the second must say it is already in the library.
use anyhow::{anyhow, Result};
use quince_icloud::photos::Flag;
use quince_icloud::photos_upload::{Outcome, Stage};
#[tokio::main]
async fn main() -> Result<()> {
    let path = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow!("jpeg path"))?,
    );
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
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let t = std::time::Instant::now();
    let progress: std::sync::Arc<dyn Fn(Stage) + Send + Sync> =
        std::sync::Arc::new(move |s| println!("  {:>6.2}s {s:?}", t.elapsed().as_secs_f64()));
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
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64,
        "Asia/Kolkata",
        -330,
        &uuid::Uuid::new_v4().to_string(),
        progress,
    )
    .await?;
    println!("outcome: {outcome:?}");
    if let Outcome::Uploaded(u) = outcome {
        let zc = quince_icloud::cloudkit::ZoneClient::new(
            &http,
            &ck,
            "com.apple.photos.cloud",
            p.clone(),
        );
        let rec = zc
            .lookup_record("PrimarySync", &u.asset)
            .await?
            .ok_or_else(|| anyhow!("not in the library"))?;
        println!("in the library: yes");
        quince_icloud::photos::set_flag(
            &http,
            &ck,
            p,
            None,
            &u.asset,
            rec["recordChangeTag"].as_str(),
            Flag::Deleted,
            true,
        )
        .await?;
        println!("sent to Recently Deleted");
    }
    Ok(())
}
