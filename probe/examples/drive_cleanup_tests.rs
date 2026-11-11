//! Live cleanup: any folder at the top of Drive named "Quince … test" that a probe left
//! behind, moved to Recently Deleted and deleted for good; and any folder
//! with "Skykeep test" in its name (the Android tests' folders) already in Recently Deleted,
//! deleted for good. Folders named on the command line (exact names, at the top of Drive) are
//! taken too: the Android Send to iPhone test's "From <emulator>" folder, for one. Nothing else
//! is touched.
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
    let dws = client
        .session()
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let named: Vec<String> = std::env::args().skip(1).collect();
    let mut lib = quince_core::Library::in_memory()?;
    drive::sync_folder(&http, &dws, p.clone(), &mut lib, ROOT).await?;
    let left: Vec<_> = lib
        .drive_children(ROOT)?
        .into_iter()
        .filter(|i| {
            i.is_container()
                && ((i.name.starts_with("Quince ") && i.name.ends_with(" test"))
                    || named.iter().any(|n| *n == i.display_name()))
        })
        .collect();
    println!(
        "left behind: {:?}",
        left.iter().map(|i| &i.name).collect::<Vec<_>>()
    );
    for f in &left {
        drive::delete(&http, &dws, p.clone(), f).await?;
    }
    let ids: Vec<_> = left.iter().map(|i| i.drivewsid.clone()).collect();
    let t: Vec<_> = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| {
            ids.contains(&i.drivewsid)
                || (i.is_container()
                    && (i.name.contains("Skykeep test")
                        || named.iter().any(|n| *n == i.display_name())))
        })
        .collect();
    println!(
        "in Recently Deleted to purge: {:?}",
        t.iter().map(|i| &i.name).collect::<Vec<_>>()
    );
    println!(
        "purged, refused: {:?}",
        drive::delete_forever_many(&http, &dws, p.clone(), &t).await?
    );
    Ok(())
}
