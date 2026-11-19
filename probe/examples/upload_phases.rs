//! Live, throwaway file only: a 40 MB "Quince upload timing test.bin" sent to the root, timing
//! the bytes and then the filing ("Checking") apart, then deleted for good.
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
    let tmp = std::env::temp_dir().join("quince-timing");
    std::fs::create_dir_all(&tmp)?;
    let f = tmp.join("Quince upload timing test.bin");
    let mut bytes = vec![0u8; 40 * 1024 * 1024];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = ((i * 2654435761usize) >> 13) as u8;
    }
    std::fs::write(&f, &bytes)?;
    let start = std::time::Instant::now();
    let sent_at = std::sync::Arc::new(std::sync::Mutex::new(None::<std::time::Duration>));
    let s2 = sent_at.clone();
    let up = drive::upload(
        &http,
        &http,
        &docws,
        &dws,
        p.clone(),
        ROOT,
        &f,
        std::sync::Arc::new(move |sent, total| {
            if sent >= total {
                s2.lock().unwrap().get_or_insert(start.elapsed());
            }
        }),
    )
    .await?;
    let all = start.elapsed();
    let bytes_done = sent_at.lock().unwrap().unwrap_or_default();
    println!(
        "bytes up after {bytes_done:?}; filed and checked after {all:?} — Checking took {:?}",
        all - bytes_done
    );
    drive::delete(&http, &dws, p.clone(), &up).await?;
    let t = drive::trash(&http, &dws, p.clone())
        .await?
        .into_iter()
        .filter(|i| i.drivewsid == up.drivewsid)
        .collect::<Vec<_>>();
    println!(
        "cleanup refused: {:?}",
        drive::delete_forever_many(&http, &dws, p.clone(), &t).await?
    );
    Ok(())
}
