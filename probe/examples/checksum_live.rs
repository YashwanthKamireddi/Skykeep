//! Live, read-only (27 Nov 2026, for Free up iCloud space): does Apple's `fileChecksum` for a
//! photo's original equal base64(0x01 || SHA-1("com.apple.XattrObjectSalt\0com.apple.
//! DataObjectSalt\0" || bytes)) for small, large and video files? Downloads a few originals of
//! the signed-in library into a folder given as the first argument, writes each one's expected
//! checksum beside it (`<n>.expect`, hex), and changes nothing in iCloud. The digest is checked
//! outside (sha1sum), so the probe needs no new crate.
use anyhow::{anyhow, Result};
use base64::Engine;
use quince_core::photos::{Kind, View};

#[tokio::main]
async fn main() -> Result<()> {
    let out = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow!("give a folder to download into"))?,
    );
    std::fs::create_dir_all(&out)?;
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
    // The whole library: Apple hands it over a page at a time, carried on by its token.
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
    let all = lib.photos(View::Library)?;
    println!("library: {} photos and videos", all.len());
    let size = |x: &quince_core::photos::Photo| x.size.unwrap_or(0);
    let mut stills: Vec<_> = all.iter().filter(|x| x.kind != Kind::Video).collect();
    stills.sort_by_key(|x| size(x));
    let mut videos: Vec<_> = all.iter().filter(|x| x.kind == Kind::Video).collect();
    videos.sort_by_key(|x| size(x));
    // Smallest and largest still, a Live Photo, and the largest video (the chunking question).
    let mut picks = Vec::new();
    picks.extend(stills.first().copied());
    picks.extend(stills.last().copied());
    picks.extend(all.iter().find(|x| x.kind == Kind::Live));
    picks.extend(videos.last().copied());
    picks.extend(videos.get(videos.len() / 2).copied());
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        &http,
        &root,
        quince_icloud::photos::CONTAINER,
        p.clone(),
    );
    let zone = quince_icloud::photos::library_zone(&zc).await;
    for (n, photo) in picks.iter().enumerate() {
        let master = zc
            .lookup_record(&zone, &photo.master)
            .await?
            .ok_or_else(|| anyhow!("master gone"))?;
        let asset = zc
            .lookup_record(&zone, &photo.id)
            .await?
            .ok_or_else(|| anyhow!("asset gone"))?;
        let mut parts = vec![("original", "resOriginalRes")];
        if photo.kind == Kind::Live {
            parts.push(("live", "resOriginalVidComplRes"));
        }
        for (part, field) in parts {
            let value = &master["fields"][field]["value"];
            let Some(sum) = value["fileChecksum"].as_str() else {
                println!("[{n}] {part}: no fileChecksum");
                continue;
            };
            let raw = base64::engine::general_purpose::STANDARD.decode(sum)?;
            let url = match part {
                "live" => quince_icloud::photos::links_of(&master).live,
                _ => quince_icloud::photos::download_choice(
                    &asset,
                    &master,
                    quince_icloud::photos::Want::Originals,
                    photo.kind == Kind::Video,
                )
                .map(|(u, _)| u),
            }
            .ok_or_else(|| anyhow!("no link"))?;
            let path = out.join(format!("{n}-{part}"));
            // Streamed to disk: the largest video may not fit in memory.
            let mut resp = http.get(&url).send().await?.error_for_status()?;
            let mut file = std::fs::File::create(&path)?;
            let mut written = 0usize;
            while let Some(chunk) = resp.chunk().await? {
                std::io::Write::write_all(&mut file, &chunk)?;
                written += chunk.len();
            }
            let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
            std::fs::write(path.with_extension("expect"), &hex)?;
            println!(
                "[{n}] {part}: {:?} {} bytes (Apple says {:?}), checksum {} bytes, first byte {:#04x}",
                photo.kind,
                written,
                value["size"].as_u64(),
                raw.len(),
                raw.first().copied().unwrap_or(0)
            );
        }
    }
    Ok(())
}
