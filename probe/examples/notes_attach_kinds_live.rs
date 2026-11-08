//! Live, throwaway note only: files of each kind added to a new note one after another, as a
//! drop of several files adds them, each outcome printed; the note then sent to Recently
//! Deleted. Args: the files.
use anyhow::{anyhow, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let files: Vec<String> = std::env::args().skip(1).collect();
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
    let guid = quince_icloud::notes::create_cloudkit(
        &http,
        &root,
        p.clone(),
        None,
        "Skykeep attach test\nsafe to delete",
        None,
    )
    .await?;
    println!("note {guid}");
    for f in &files {
        let r = quince_icloud::notes::attach::attach_file_cloudkit(
            &http,
            &root,
            p.clone(),
            &guid,
            std::path::Path::new(f),
            None,
            0,
        )
        .await;
        match r {
            Ok(a) => println!("{f}: ok {} uti {}", a.attachment, a.uti),
            Err(e) => println!("{f}: FAILED {e}"),
        }
    }
    let zc = quince_icloud::cloudkit::ZoneClient::new(&http, &root, "com.apple.notes", p.clone());
    let rec = zc
        .lookup_record("Notes", &guid)
        .await?
        .ok_or_else(|| anyhow!("gone"))?;
    let note = quince_core::notes::Note {
        guid: guid.clone(),
        folder_guid: None,
        title: "Skykeep attach test".into(),
        snippet: None,
        body: None,
        locked: false,
        pinned: false,
        created_at: None,
        modified_at: None,
        editable: true,
        etag: rec["recordChangeTag"].as_str().map(String::from),
        runs: None,
    };
    quince_icloud::notes::delete_cloudkit(&http, &root, p, &note).await?;
    println!("sent to Recently Deleted");
    Ok(())
}
