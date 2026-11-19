//! Live, throwaway file only: "Skykeep test long upload.bin", sized (MB=…) so sending it takes
//! longer than a minute, sent to the iCloud Drive root through the session's read-timeout
//! client (cut off part way before quince_auth::Client::uploads existed) and through the
//! uploads client, which must take it whole. Every copy that landed is deleted for good.
//!
//!   MB=400 cargo run -p quince-probe --example drive_upload_long
use anyhow::{anyhow, Result};
use std::time::Instant;

const ROOT: &str = "FOLDER::com.apple.CloudDocs::root";

#[tokio::main]
async fn main() -> Result<()> {
    let dir = std::path::PathBuf::from(std::env::var("HOME")?).join(".local/share/quince-probe");
    let mut client = quince_auth::Client::new(&dir)?;
    if !client.resume().await? {
        return Err(anyhow!("no session"));
    }
    let need = |s: &str| {
        client
            .session()
            .service_url(s)
            .ok_or_else(|| anyhow!("no {s}"))
    };
    let (docws, drivews) = (need("docws")?, need("drivews")?);
    let (http, uploads) = (client.http().clone(), client.uploads().clone());
    let params = client.service_params();

    let mb: usize = std::env::var("MB")
        .ok()
        .and_then(|m| m.parse().ok())
        .unwrap_or(400);
    let tmp = std::env::temp_dir().join("skykeep-long-upload");
    std::fs::create_dir_all(&tmp)?;
    let file = tmp.join("Skykeep test long upload.bin");
    {
        // Incompressible, so nothing on the way makes it smaller than it is.
        use std::io::Write;
        let mut out = std::io::BufWriter::new(std::fs::File::create(&file)?);
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut block = vec![0u8; 1 << 20];
        for _ in 0..mb {
            for b in block.chunks_mut(8) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                b.copy_from_slice(&x.to_le_bytes()[..b.len()]);
            }
            out.write_all(&block)?;
        }
    }
    println!("local file: {mb} MB");

    let mut landed = Vec::new();
    for (how, sender) in [("read-timeout client", &http), ("uploads client", &uploads)] {
        let started = Instant::now();
        let shown = std::sync::Arc::new(std::sync::Mutex::new(Instant::now()));
        let progress: quince_icloud::drive::Progress = std::sync::Arc::new(move |sent, total| {
            let mut last = shown.lock().unwrap();
            if last.elapsed().as_secs() >= 10 || sent == total {
                *last = Instant::now();
                println!("  {} of {} MB", sent >> 20, total >> 20);
            }
        });
        let r = quince_icloud::drive::upload(
            &http,
            sender,
            &docws,
            &drivews,
            params.clone(),
            ROOT,
            &file,
            progress,
        )
        .await;
        match r {
            Ok(item) => {
                println!(
                    "{how}: landed whole ({} bytes) after {:?}",
                    item.size.unwrap_or(-1),
                    started.elapsed()
                );
                landed.push(item);
            }
            Err(e) => println!("{how}: FAILED after {:?}: {e}", started.elapsed()),
        }
    }

    // Take it all back out, for good.
    for item in &landed {
        quince_icloud::drive::delete(&http, &drivews, params.clone(), item).await?;
    }
    let trash = quince_icloud::drive::trash(&http, &drivews, params.clone()).await?;
    for t in trash
        .iter()
        .filter(|t| landed.iter().any(|l| l.drivewsid == t.drivewsid))
    {
        quince_icloud::drive::delete_forever(&http, &drivews, params.clone(), t).await?;
    }
    std::fs::remove_dir_all(&tmp).ok();
    println!("cleaned up: {} copies deleted for good", landed.len());
    Ok(())
}
