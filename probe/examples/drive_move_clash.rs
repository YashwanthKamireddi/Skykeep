//! Throwaway: what Apple does when something is moved into a folder that already holds an item
//! of the same name — refuses, renames, or replaces. Everything made here is deleted after.
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
    let none = || std::sync::Arc::new(|_: u64, _: u64| {}) as drive::Progress;
    let top = drive::create_folder(&http, &dws, p.clone(), ROOT, "Quince move clash test").await?;
    let dest = drive::create_folder(&http, &dws, p.clone(), &top.drivewsid, "dest").await?;
    let tmp = std::env::temp_dir().join("quince-clash");
    std::fs::create_dir_all(&tmp)?;
    let f = tmp.join("same.txt");
    std::fs::write(&f, b"one")?;
    let there = drive::upload(
        &http,
        &http,
        &docws,
        &dws,
        p.clone(),
        &dest.drivewsid,
        &f,
        none(),
    )
    .await?;
    std::fs::write(&f, b"two!")?;
    let moving = drive::upload(
        &http,
        &http,
        &docws,
        &dws,
        p.clone(),
        &top.drivewsid,
        &f,
        none(),
    )
    .await?;
    println!(
        "in dest: {} ({:?}), moving: {} ({:?})",
        there.name, there.size, moving.name, moving.size
    );
    let r = drive::move_to(&http, &dws, p.clone(), &moving, &dest.drivewsid).await;
    println!("move file onto same name: {r:?}");
    let sub = drive::create_folder(&http, &dws, p.clone(), &top.drivewsid, "dest2").await?;
    let sub_in = drive::create_folder(&http, &dws, p.clone(), &dest.drivewsid, "dest2").await?;
    let r2 = drive::move_to(&http, &dws, p.clone(), &sub, &dest.drivewsid).await;
    println!(
        "move folder onto same-named folder: {r2:?} (inner {})",
        sub_in.name
    );
    let mut lib = quince_core::Library::in_memory()?;
    drive::sync_folder(&http, &dws, p.clone(), &mut lib, &dest.drivewsid).await?;
    for i in lib.drive_children(&dest.drivewsid)? {
        println!(
            "  dest holds {} {:?} {:?} {}",
            i.name, i.extension, i.size, i.drivewsid
        );
    }
    drive::delete(&http, &dws, p.clone(), &top).await?;
    println!("cleaned up");
    Ok(())
}
