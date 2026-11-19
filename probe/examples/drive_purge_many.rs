//! Live, throwaway files only: three "Quince purge test N.txt" uploaded, moved to Recently
//! Deleted, then deleted for good in one request as Delete All sends it; the trash read again
//! twenty seconds later, so a listing that merely lagged cannot pass for a delete.
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
    let tmp = std::env::temp_dir().join("quince-purge-many");
    std::fs::create_dir_all(&tmp)?;
    let mut ids = Vec::new();
    for n in 1..=3 {
        let f = tmp.join(format!("Quince purge test {n}.txt"));
        std::fs::write(&f, format!("safe to delete {n}"))?;
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
        ids.push(up.drivewsid);
    }
    let trash = drive::trash(&http, &dws, p.clone()).await?;
    let ours: Vec<_> = trash
        .into_iter()
        .filter(|i| ids.contains(&i.drivewsid))
        .collect();
    println!("{} in Recently Deleted", ours.len());
    let failed = drive::delete_forever_many(&http, &dws, p.clone(), &ours).await?;
    println!("refused: {failed:?}");
    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    let left = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| ids.contains(&i.drivewsid))
        .count();
    println!("still in Recently Deleted 20s later: {left}");
    Ok(())
}
