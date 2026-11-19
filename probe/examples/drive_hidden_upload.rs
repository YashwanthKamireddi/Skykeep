//! Throwaway: whether iCloud Drive keeps hidden things a folder upload carries — a file and a
//! folder whose names begin with a dot — made in a test folder that is then deleted.
use anyhow::{anyhow, Result};

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
    let drivews = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let http = client.http().clone();
    let params = client.service_params();
    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;

    let test = quince_icloud::drive::create_folder(
        &http,
        &drivews,
        params.clone(),
        ROOT,
        "Quince hidden test",
    )
    .await?;
    println!("test folder {}", test.drivewsid);

    let local = std::env::temp_dir().join(".hidden-quince.txt");
    std::fs::write(&local, "Quince hidden-file test - safe to delete\n")?;
    let file = quince_icloud::drive::upload(
        &http,
        &http,
        &docws,
        &drivews,
        params.clone(),
        &test.drivewsid,
        &local,
        std::sync::Arc::new(|_, _| {}),
    )
    .await;
    println!(
        "hidden file upload: {:?}",
        file.as_ref()
            .map(|f| (&f.name, &f.extension))
            .map_err(ToString::to_string)
    );

    let made = quince_icloud::drive::create_folders(
        &http,
        &drivews,
        params.clone(),
        &test.drivewsid,
        &[".dotdir".to_string()],
    )
    .await;
    println!("hidden folder: {:?}", made.map_err(|e| e.to_string()));

    quince_icloud::drive::sync_folder(
        &http,
        &drivews,
        params.clone(),
        &mut library,
        &test.drivewsid,
    )
    .await?;
    let inside: Vec<String> = library
        .drive_children(&test.drivewsid)?
        .iter()
        .map(|i| {
            format!(
                "{}{}",
                i.name,
                i.extension
                    .as_deref()
                    .map(|e| format!(".{e}"))
                    .unwrap_or_default()
            )
        })
        .collect();
    println!("iCloud holds: {inside:?}");

    quince_icloud::drive::delete(&http, &drivews, params.clone(), &test).await?;
    println!("test folder deleted (to Recently Deleted)");
    Ok(())
}
