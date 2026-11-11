//! Live, read-only: one Reminders List record, every field; or, given a name, every List
//! record whose name has it (a test list's state after a change, for instance).
use anyhow::{anyhow, Result};
use serde_json::Value;
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
    let zc = quince_icloud::cloudkit::ZoneClient::new(
        client.http(),
        &root,
        "com.apple.reminders",
        client.service_params(),
    );
    let named = std::env::args().nth(1);
    let walk = zc.all_records("Reminders", None).await?;
    for r in walk
        .records
        .iter()
        .filter(|r| {
            r["recordType"] == "List"
                && r["fields"]["IsGroup"]["value"] != 1
                && match &named {
                    Some(n) => r["fields"]["Name"]["value"]
                        .as_str()
                        .is_some_and(|s| s.contains(n.as_str())),
                    None => r["fields"].get("Color").is_some(),
                }
        })
        .take(if named.is_some() { 20 } else { 1 })
    {
        let mut r = r.clone();
        if let Some(f) = r.get_mut("fields").and_then(Value::as_object_mut) {
            for (k, v) in f.iter_mut() {
                if k == "ReminderIDs" {
                    *v = Value::from("…");
                }
            }
        }
        println!("{}", serde_json::to_string_pretty(&r["fields"])?);
    }
    Ok(())
}
