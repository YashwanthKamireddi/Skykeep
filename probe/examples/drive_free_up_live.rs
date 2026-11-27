//! Live, on a throwaway file only (27 Nov 2026, Free up iCloud space for Drive): a file named
//! "Skykeep test free-up <time>.txt" is put at the top of iCloud Drive, its backup copy made in
//! a temporary folder, and the Drive free-up calls are proven against Apple: `checksums` gives
//! the same AppleChecksum as the bytes on disk; `trash_versions` with a wrong version is
//! refused (the file stays); with the version the copy was made of, it goes to Recently
//! Deleted. Then it is deleted for good, so nothing is left behind.
use anyhow::{anyhow, Result};
use std::io::Write;

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let s = client.session();
    let drivews = s
        .service_url("drivews")
        .ok_or_else(|| anyhow!("no drivews"))?;
    let docws = s.service_url("docws").ok_or_else(|| anyhow!("no docws"))?;
    let (http, params) = (client.http().clone(), client.service_params());
    let root = quince_core::drive::ROOT;
    if std::env::args().any(|a| a == "--clean") {
        for i in quince_icloud::drive::fetch_folder(&http, &drivews, params.clone(), root)
            .await?
            .iter()
            .filter(|i| i.name.starts_with("Skykeep test free-up"))
        {
            quince_icloud::drive::delete(&http, &drivews, params.clone(), i).await?;
            println!("to Recently Deleted: {}", i.name);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        for t in quince_icloud::drive::trash_items(&http, &drivews, params.clone())
            .await?
            .iter()
            .filter(|t| t.item.name.starts_with("Skykeep test free-up"))
        {
            quince_icloud::drive::delete_forever(&http, &drivews, params.clone(), &t.item).await?;
            println!("deleted for good: {}", t.item.name);
        }
        return Ok(());
    }

    let work = std::env::temp_dir().join(format!("skykeep-freeup-probe-{}", std::process::id()));
    std::fs::create_dir_all(&work)?;
    let name = format!(
        "Skykeep test free-up {}.txt",
        jiff::Timestamp::now().as_second()
    );
    let local = work.join(&name);
    std::fs::File::create(&local)?.write_all(b"Skykeep free-up probe: safe to delete.\n")?;
    let quiet: quince_icloud::drive::Progress = std::sync::Arc::new(|_, _| {});
    let item = quince_icloud::drive::upload(
        &http,
        &http,
        &docws,
        &drivews,
        params.clone(),
        root,
        &local,
        quiet,
    )
    .await?;
    println!(
        "uploaded: {} etag set {}",
        item.drivewsid,
        item.etag.is_some()
    );

    // The listing's own etag for it, as the backup records it.
    let listed = quince_icloud::drive::fetch_folder(&http, &drivews, params.clone(), root).await?;
    let etag = listed
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .and_then(|i| i.etag.clone())
        .ok_or_else(|| anyhow!("not listed"))?;

    let doc = item
        .drivewsid
        .rsplit("::")
        .next()
        .unwrap_or_default()
        .to_string();
    let sums = quince_icloud::drive::checksums(
        &http,
        &docws,
        params.clone(),
        &[doc.clone(), "00000000-0000-0000-0000-000000000000".into()],
    )
    .await?;
    let mut sum = quince_icloud::drive::AppleChecksum::default();
    sum.update(&std::fs::read(&local)?);
    let ours = sum.finish();
    println!(
        "checksum from Apple matches the bytes on disk: {}",
        sums.get(&doc.to_lowercase()).cloned().flatten().as_deref() == Some(ours.as_str())
    );
    println!(
        "an unknown id is left out: {}",
        !sums.contains_key("00000000-0000-0000-0000-000000000000")
    );

    // A real stale version: the file renamed (a new etag), then asked to go as the old one.
    let mut listed_item = listed
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .cloned()
        .ok_or_else(|| anyhow!("not listed"))?;
    quince_icloud::drive::rename(
        &http,
        &drivews,
        params.clone(),
        &listed_item,
        &format!("{} renamed.txt", name.trim_end_matches(".txt")),
    )
    .await?;
    let wrong = quince_icloud::drive::trash_versions(
        &http,
        &drivews,
        params.clone(),
        &[(item.drivewsid.clone(), etag.clone())],
    )
    .await?;
    println!("old version: {:?}", wrong.get(&item.drivewsid));
    let still = quince_icloud::drive::fetch_folder(&http, &drivews, params.clone(), root).await?;
    println!(
        "still in Drive after the old version: {}",
        still.iter().any(|i| i.drivewsid == item.drivewsid)
    );
    listed_item = still
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .cloned()
        .ok_or_else(|| anyhow!("gone after the old version"))?;
    let etag = listed_item.etag.clone().ok_or_else(|| anyhow!("no etag"))?;
    let right = quince_icloud::drive::trash_versions(
        &http,
        &drivews,
        params.clone(),
        &[(item.drivewsid.clone(), etag)],
    )
    .await?;
    println!("right version: {:?}", right.get(&item.drivewsid));
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let trash = quince_icloud::drive::trash_items(&http, &drivews, params.clone()).await?;
    let in_trash = trash
        .iter()
        .find(|t| t.item.name.starts_with("Skykeep test free-up"));
    println!("in Recently Deleted: {}", in_trash.is_some());

    // Cleaned up: deleted for good, and the local copy removed.
    for t in trash
        .iter()
        .filter(|t| t.item.name.starts_with("Skykeep test free-up"))
    {
        quince_icloud::drive::delete_forever(&http, &drivews, params.clone(), &t.item).await?;
    }
    let _ = std::fs::remove_dir_all(&work);
    let left = quince_icloud::drive::trash_items(&http, &drivews, params.clone()).await?;
    println!(
        "left behind: {}",
        left.iter()
            .filter(|t| t.item.name.starts_with("Skykeep test free-up"))
            .count()
    );
    Ok(())
}
