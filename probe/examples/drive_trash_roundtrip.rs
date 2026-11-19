//! Live, on the probe account, with a throwaway file only: upload, trash, recover, trash,
//! delete for good — each step read back. Touches nothing that was there before.
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
    let tmp = std::env::temp_dir().join("quince-trash-test.txt");
    std::fs::write(&tmp, b"Quince trash round trip. Safe to delete.")?;
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
    println!("uploaded {}", item.drivewsid);
    let in_trash = |id: String| {
        let (h, d, p) = (http.clone(), drivews.clone(), p.clone());
        async move {
            Ok::<bool, anyhow::Error>(
                quince_icloud::drive::trash(&h, &d, p)
                    .await?
                    .iter()
                    .any(|i| i.drivewsid == id),
            )
        }
    };
    quince_icloud::drive::delete(&http, &drivews, p.clone(), &item).await?;
    println!(
        "trashed; in trash = {}",
        in_trash(item.drivewsid.clone()).await?
    );
    quince_icloud::drive::put_back(&http, &drivews, p.clone(), &item).await?;
    println!(
        "recovered; in trash = {}",
        in_trash(item.drivewsid.clone()).await?
    );
    // Re-read its etag where it landed, as Quince would after a refresh.
    let mut lib = quince_core::Library::open(
        &std::env::temp_dir().join(format!("qtt-{}", uuid::Uuid::new_v4())),
    )?;
    quince_icloud::drive::sync_folder(
        &http,
        &drivews,
        p.clone(),
        &mut lib,
        quince_core::drive::ROOT,
    )
    .await?;
    let back = lib
        .drive_item(&item.drivewsid)?
        .ok_or_else(|| anyhow!("not back in the root"))?;
    println!("back in the root: {}", back.display_name());
    quince_icloud::drive::delete(&http, &drivews, p.clone(), &back).await?;
    quince_icloud::drive::delete_forever(&http, &drivews, p.clone(), &back).await?;
    println!(
        "deleted for good; in trash = {}",
        in_trash(item.drivewsid.clone()).await?
    );
    Ok(())
}
