//! Live, throwaway file only: "Quince purge test.txt" uploaded, moved to Recently Deleted,
//! then deleted for good — first asked the old way (no clientId) to see Apple's answer, then
//! the app's way — and the trash read again twenty seconds later.
use anyhow::{anyhow, Result};
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
    let tmp = std::env::temp_dir().join("quince-purge");
    std::fs::create_dir_all(&tmp)?;
    let f = tmp.join("Quince purge test.txt");
    std::fs::write(&f, b"safe to delete")?;
    let up = drive::upload(
        &http,
        &http,
        &docws,
        &dws,
        p.clone(),
        ROOT,
        &f,
        std::sync::Arc::new(|_, _| {}),
    )
    .await?;
    drive::delete(&http, &dws, p.clone(), &up).await?;
    let in_trash = drive::trash(&http, &dws, p.clone()).await?;
    let t = in_trash
        .iter()
        .find(|i| i.drivewsid == up.drivewsid)
        .cloned()
        .ok_or_else(|| anyhow!("not in trash"))?;
    println!("in Recently Deleted: {}", t.name);
    // The old request, as Quince sent it.
    let r = http
        .post(format!("{}/deleteItems", dws.trim_end_matches('/')))
        .query(&p)
        .json(&serde_json::json!({ "items": [{ "drivewsid": t.drivewsid, "etag": t.etag }] }))
        .send()
        .await?;
    println!(
        "old way: {} {}",
        r.status(),
        r.text()
            .await
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect::<String>()
    );
    let still = drive::trash(&http, &dws, p.clone())
        .await?
        .iter()
        .any(|i| i.drivewsid == up.drivewsid);
    println!("still in trash after the old way: {still}");
    if still {
        let t = drive::trash(&http, &dws, p.clone())
            .await?
            .into_iter()
            .find(|i| i.drivewsid == up.drivewsid)
            .unwrap();
        drive::delete_forever(&http, &dws, p.clone(), &t).await?;
        println!("fixed way: OK");
    }
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    let later = drive::trash(&http, &dws, p.clone())
        .await?
        .iter()
        .any(|i| i.drivewsid == up.drivewsid);
    println!("in trash 20s later: {later}");
    Ok(())
}
