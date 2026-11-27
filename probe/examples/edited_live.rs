//! Live, read-only (27 Nov 2026, Free up iCloud space): how many of the library's photos read
//! as edited by quince_backup::free_up::edited, and which edit fields Apple's assets carry, so
//! the check neither keeps every photo (a field on all of them) nor misses real edits.
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let root = client
        .session()
        .service_url("ckdatabasews")
        .ok_or_else(|| anyhow!("no ckdatabasews"))?;
    let (http, p) = (client.http().clone(), client.service_params());
    let mut lib = quince_core::Library::in_memory()?;
    let mut token: Option<String> = None;
    for _ in 0..500 {
        let page = quince_icloud::photos::fetch(&http, &root, p.clone(), token.as_deref()).await?;
        let (done, next) = (page.complete, page.token.clone());
        quince_icloud::photos::apply(&mut lib, page)?;
        if done || next.is_none() || next == token {
            break;
        }
        token = next;
    }
    let photos = lib.photos(quince_core::photos::View::Library)?;
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        &http,
        &root,
        quince_icloud::photos::CONTAINER,
        p.clone(),
    );
    let zone = quince_icloud::photos::library_zone(&zc).await;
    let (mut edited, mut seen) = (0, 0);
    let mut fields: BTreeMap<String, usize> = BTreeMap::new();
    for chunk in photos.chunks(150) {
        let ids: Vec<String> = chunk.iter().map(|x| x.id.clone()).collect();
        let records = zc.lookup_records(&zone, &ids).await?;
        for r in records.values() {
            seen += 1;
            if quince_backup::free_up::edited(r) {
                edited += 1;
            }
            if let Some(f) = r["fields"].as_object() {
                for k in f.keys().filter(|k| {
                    k.contains("djust") || k.starts_with("resJPEG") || k.contains("ender")
                }) {
                    *fields.entry(k.clone()).or_default() += 1;
                }
            }
        }
    }
    println!("assets read {seen}, read as edited {edited}");
    for (k, n) in fields {
        println!("  field {k}: on {n} assets");
    }
    Ok(())
}
