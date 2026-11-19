//! Throwaway: the real upload path, into the root and into a folder, then cleaned up.
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
    let uploads = client.uploads().clone();
    let params = client.service_params();

    // A file big enough that streaming rather than slurping actually matters.
    let tmp = std::env::temp_dir().join("quince-upload-test.txt");
    let body = "Quince upload test - safe to delete\n".repeat(200_000);
    std::fs::write(&tmp, &body)?;
    println!("local file: {} bytes", body.len());

    // Into the root.
    let landed = quince_icloud::drive::upload(
        &http,
        &uploads,
        &docws,
        &drivews,
        params.clone(),
        ROOT,
        &tmp,
        std::sync::Arc::new(|_, _| {}),
    )
    .await?;
    println!(
        "root: {} ({:?}) {} bytes  parent={:?}",
        landed.name,
        landed.extension,
        landed.size.unwrap_or(-1),
        landed.parent_id
    );
    if landed.size != Some(body.len() as i64) {
        return Err(anyhow!("the size Apple holds is not the size sent"));
    }

    // Into a folder, which is the case `starting_document_id` exists for.
    quince_icloud::drive::create_folder(
        &http,
        &drivews,
        params.clone(),
        ROOT,
        "quince-upload-folder",
    )
    .await?;
    let store = std::env::temp_dir().join(format!("quince-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&store)?;
    let mut library = quince_core::Library::open(&store)?;
    quince_icloud::drive::sync_folder(&http, &drivews, params.clone(), &mut library, ROOT).await?;
    let folder = library
        .drive_children(ROOT)?
        .into_iter()
        .find(|i| i.name == "quince-upload-folder")
        .ok_or_else(|| anyhow!("the test folder was not made"))?;

    let inner = quince_icloud::drive::upload(
        &http,
        &uploads,
        &docws,
        &drivews,
        params.clone(),
        &folder.drivewsid,
        &tmp,
        std::sync::Arc::new(|_, _| {}),
    )
    .await?;
    println!("folder: {} in {:?}", inner.name, inner.parent_id);
    if inner.parent_id.as_deref() != Some(folder.drivewsid.as_str()) {
        return Err(anyhow!(
            "it landed somewhere other than the folder asked for"
        ));
    }

    // Take it all back out.
    quince_icloud::drive::delete(&http, &drivews, params.clone(), &landed).await?;
    quince_icloud::drive::delete(&http, &drivews, params.clone(), &folder).await?;
    std::fs::remove_file(&tmp).ok();
    println!("\ncleaned up. OK — upload works into the root and into a folder.");
    Ok(())
}
