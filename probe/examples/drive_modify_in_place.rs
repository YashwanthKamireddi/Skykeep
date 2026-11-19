//! Throwaway (#24): `drive::replace_contents` against Apple. One test file in the Drive root:
//! uploaded, its contents replaced in place (same id, new bytes, nothing in Recently Deleted),
//! then replaced again from the stale first version, which Apple must refuse as a conflict with
//! the newer bytes kept. Removed for good at the end.
use anyhow::{anyhow, Result};

const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";
const NAME: &str = "Skykeep in-place test - safe to delete.txt";

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session: sign the probe in first"));
    }
    let docws = client
        .session()
        .service_url("docws")
        .ok_or_else(|| anyhow!("no docws"))?;
    let drivews = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let http = client.http().clone();
    let params = client.service_params();
    let quiet = || -> quince_icloud::drive::Progress { std::sync::Arc::new(|_, _| {}) };

    let work = dir.join("inplace");
    std::fs::create_dir_all(&work)?;
    let file = work.join(NAME);
    let result = async {
        std::fs::write(&file, "version one\n")?;
        let first = quince_icloud::drive::upload(&http, &http, &docws, &drivews, params.clone(), ROOT, &file, quiet()).await?;
        println!("v1: size={:?} etag={:?}", first.size, first.etag);
        let trash_before = quince_icloud::drive::trash(&http, &drivews, params.clone()).await?.len();

        let v2 = "version two, longer than the first\n".repeat(3);
        std::fs::write(&file, &v2)?;
        let second = quince_icloud::drive::replace_contents(&http, &http, &docws, &drivews, params.clone(), &first, &file, quiet()).await?;
        println!("v2: same id={} size={:?} etag={:?}", second.drivewsid == first.drivewsid, second.size, second.etag);
        let link = quince_icloud::drive::download_url(&http, &docws, params.clone(), &second).await?;
        println!("v2 bytes at Apple match: {}", http.get(&link).send().await?.text().await? == v2);

        std::fs::write(&file, "version three, from a stale read\n")?;
        let stale = quince_icloud::drive::replace_contents(&http, &http, &docws, &drivews, params.clone(), &first, &file, quiet()).await;
        println!("stale write: {}", match &stale { Err(quince_icloud::Error::Conflict) => "refused as a conflict".to_string(), other => format!("UNEXPECTED {other:?}") });
        let link = quince_icloud::drive::download_url(&http, &docws, params.clone(), &second).await?;
        println!("v2 still what Apple holds: {}", http.get(&link).send().await?.text().await? == v2);

        let trash_after = quince_icloud::drive::trash(&http, &drivews, params.clone()).await?.len();
        let copies = quince_icloud::drive::fetch_folder(&http, &drivews, params.clone(), ROOT).await?
            .into_iter().filter(|i| i.name.starts_with("Skykeep in-place test")).count();
        println!("Recently Deleted: {trash_before} before, {trash_after} after; copies in root: {copies}");
        anyhow::Ok(())
    }
    .await;

    // Clean up whatever happened: every copy of the test file, to Recently Deleted and out.
    for i in quince_icloud::drive::fetch_folder(&http, &drivews, params.clone(), ROOT)
        .await?
        .iter()
        .filter(|i| i.name.starts_with("Skykeep in-place test"))
    {
        quince_icloud::drive::delete(&http, &drivews, params.clone(), i).await?;
    }
    for i in quince_icloud::drive::trash(&http, &drivews, params.clone())
        .await?
        .iter()
        .filter(|i| i.name.starts_with("Skykeep in-place test"))
    {
        quince_icloud::drive::delete_forever(&http, &drivews, params.clone(), i).await?;
    }
    std::fs::remove_dir_all(&work)?;
    println!("cleaned up");
    result
}
