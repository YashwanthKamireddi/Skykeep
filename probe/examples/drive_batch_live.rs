//! Live, read-only (27 Nov 2026, Free up iCloud space for Drive): what `download/batch` answers
//! per file (keys, with values only for sizes and whether a signature is there), so the Drive
//! plan can confirm a backed-up file still exists at Apple, and its checksum, in one request.
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

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
    let (http, p) = (client.http().clone(), client.service_params());
    let items =
        quince_icloud::drive::fetch_folder(&http, &drivews, p.clone(), quince_core::drive::ROOT)
            .await?;
    let files: Vec<_> = items.iter().filter(|i| i.kind == "FILE").take(2).collect();
    let mut body: Vec<Value> = files
        .iter()
        .map(|f| json!({ "document_id": f.drivewsid.rsplit("::").next().unwrap_or_default() }))
        .collect();
    body.push(json!({ "document_id": "00000000-0000-0000-0000-000000000000" }));
    let url = format!(
        "{}/ws/com.apple.CloudDocs/download/batch",
        docws.trim_end_matches('/')
    );
    let answer: Value = http
        .post(url)
        .query(&p)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    for (n, e) in answer.as_array().into_iter().flatten().enumerate() {
        let keys: Vec<&String> = e
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        let token_keys: Vec<&String> = e["data_token"]
            .as_object()
            .map(|o| o.keys().collect())
            .unwrap_or_default();
        println!(
            "[{n}] keys {keys:?}\n     data_token keys {token_keys:?} size {:?} signature {} status {}",
            e["data_token"]["size"].as_u64().or(e["size"].as_u64()),
            e["data_token"]["signature"].is_string(),
            e["status"]
        );
    }
    for f in files {
        println!("listed size {:?} etag set {}", f.size, f.etag.is_some());
    }
    Ok(())
}
