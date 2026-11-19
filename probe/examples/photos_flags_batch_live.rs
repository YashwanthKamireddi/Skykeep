//! Live, on three synthetic photos it uploads itself: one batched `records/modify` per step —
//! favorite on and off, delete and recover, one of them sent on a stale change tag to prove the
//! retry — each read back from Apple; then all three deleted and deleted for good in two batches,
//! as "Delete All" in Recently Deleted does. Nothing of his is touched.
use anyhow::{anyhow, Result};
use quince_icloud::photos::Flag;
use quince_icloud::photos_upload::Outcome;
#[tokio::main]
async fn main() -> Result<()> {
    let files: Vec<std::path::PathBuf> = std::env::args().skip(1).map(Into::into).collect();
    anyhow::ensure!(files.len() == 3, "three jpeg paths");
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let up = client
        .session()
        .service_url("photosupload")
        .ok_or_else(|| anyhow!("no upload"))?;
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ck"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let dsid = p
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, v)| v.clone())
        .ok_or_else(|| anyhow!("no dsid"))?;
    let mut ids = Vec::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let quiet: std::sync::Arc<dyn Fn(quince_icloud::photos_upload::Stage) + Send + Sync> =
            std::sync::Arc::new(|_| {});
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let library =
            quince_icloud::photos::library_zone(&quince_icloud::cloudkit::ZoneClient::new(
                &http,
                &root,
                quince_icloud::photos::CONTAINER,
                p.clone(),
            ))
            .await;
        match quince_icloud::photos_upload::upload(
            &http,
            &http,
            &up,
            &dsid,
            &library,
            f,
            &name,
            now,
            "Asia/Kolkata",
            -330,
            &uuid::Uuid::new_v4().to_string(),
            quiet,
        )
        .await?
        {
            Outcome::Uploaded(u) => ids.push(u.asset),
            other => return Err(anyhow!("{name}: {other:?}")),
        }
    }
    println!("uploaded three throwaways: {ids:?}");
    let zc =
        quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.photos.cloud", p.clone());
    let read = |id: String| {
        let zc = &zc;
        async move {
            let r = zc.lookup_record("PrimarySync", &id).await?;
            anyhow::Ok(r.map(|r| {
                (
                    quince_icloud::cloudkit::field::int(&r, "isFavorite").unwrap_or(0) == 1,
                    quince_icloud::cloudkit::field::int(&r, "isDeleted").unwrap_or(0) == 1,
                    quince_icloud::cloudkit::field::int(&r, "isExpunged").unwrap_or(0) == 1,
                    r["recordChangeTag"].as_str().map(str::to_string),
                )
            }))
        }
    };
    let tags = |stale: Option<String>| {
        let ids = ids.clone();
        let read = &read;
        async move {
            let mut out = Vec::new();
            for (n, id) in ids.iter().enumerate() {
                let tag = read(id.clone()).await?.ok_or_else(|| anyhow!("gone"))?.3;
                out.push((
                    id.clone(),
                    if n == 0 && stale.is_some() {
                        stale.clone()
                    } else {
                        tag
                    },
                ));
            }
            anyhow::Ok(out)
        }
    };
    let first_tag = read(ids[0].clone())
        .await?
        .ok_or_else(|| anyhow!("gone"))?
        .3;
    let steps = [
        (Flag::Favorite, true, None),
        (Flag::Favorite, false, first_tag.clone()), // the first on its long-stale tag
        (Flag::Deleted, true, None),
        (Flag::Deleted, false, None),
        (Flag::Deleted, true, None),
        (Flag::Expunged, true, None),
    ];
    for (flag, on, stale) in steps {
        let assets = tags(stale.clone()).await?;
        let t = std::time::Instant::now();
        let out =
            quince_icloud::photos::set_flags(&http, &root, p.clone(), None, &assets, flag, on)
                .await?;
        let refused: Vec<_> = out.iter().filter_map(|r| r.as_ref().err()).collect();
        println!(
            "{flag:?}={on}{}: one batch in {:?}, refused {refused:?}",
            if stale.is_some() {
                " (one stale tag)"
            } else {
                ""
            },
            t.elapsed()
        );
        anyhow::ensure!(refused.is_empty());
        for id in &ids {
            let s = read(id.clone()).await?;
            let ok = match (flag, s) {
                (Flag::Favorite, Some(s)) => s.0 == on,
                (Flag::Deleted, Some(s)) => s.1 == on,
                (Flag::Expunged, s) => s.is_none_or(|s| s.2),
                _ => false,
            };
            anyhow::ensure!(
                ok,
                "{id} did not take {flag:?}={on}: {s:?}",
                s = read(id.clone()).await?
            );
        }
        println!("  read back from Apple: all three");
    }
    println!("all three throwaways deleted for good; nothing of his touched");
    Ok(())
}
