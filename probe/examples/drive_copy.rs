//! Live, throwaway file only: upload, duplicate as Quince does, list what came back,
//! then delete both for good.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let drivews = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let docws = client
        .session()
        .service_url("docws")
        .ok_or_else(|| anyhow!("no docws"))?;
    let http = client.http().clone();
    let p = client.service_params();
    let tmp = std::env::temp_dir().join("quince-copy-test.txt");
    std::fs::write(&tmp, b"Quince duplicate test. Safe to delete.")?;
    let item = quince_icloud::drive::upload(
        &http,
        &http,
        &docws,
        &drivews,
        p.clone(),
        quince_core::drive::ROOT,
        &tmp,
        std::sync::Arc::new(|_, _| {}),
    )
    .await?;
    quince_icloud::drive::duplicate(&http, &docws, &drivews, p.clone(), &item).await?;
    println!("duplicated through quince_icloud::drive::duplicate");
    let mut lib = quince_core::Library::open(
        &std::env::temp_dir().join(format!("qcp-{}", uuid::Uuid::new_v4())),
    )?;
    quince_icloud::drive::sync_folder(
        &http,
        &drivews,
        p.clone(),
        &mut lib,
        quince_core::drive::ROOT,
    )
    .await?;
    for i in lib
        .drive_children(quince_core::drive::ROOT)?
        .into_iter()
        .filter(|i| i.name.starts_with("quince-copy-test"))
    {
        println!("in root: {} {}", i.drivewsid, i.display_name());
        quince_icloud::drive::delete(&http, &drivews, p.clone(), &i).await?;
        quince_icloud::drive::delete_forever(&http, &drivews, p.clone(), &i).await?;
        println!("  removed for good");
    }
    Ok(())
}
