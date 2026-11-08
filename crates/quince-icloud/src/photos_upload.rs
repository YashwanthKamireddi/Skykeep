//! Uploading to iCloud Photos, as icloud.com does it now (measured live 2026-10-19; the older
//! `uploadimagews …/upload` answers 410 Gone). Four steps on the `photosupload` service, each a
//! `POST {photosupload}/photosupload/<endpoint>?dsid=…` with a JSON body sent as text/plain:
//!
//! 1. `createUploadUrl` — where this file's bytes go;
//! 2. the bytes, sent there — answered with a receipt (`singleFile`), or `isDuplicate`;
//! 3. `putAsset` — the receipt made a photo: its asset and master names and a job id;
//! 4. `uploadStatus` — the job polled until Apple has made the photo (progress 100).
//!
//! A file is only ever read. A photo counts as uploaded once Apple's job says done, never on
//! the strength of a byte count. See docs/icloud-web/photos.md.

use crate::{Error, Result};
use serde_json::{json, Value};
use std::time::Duration;

/// What icloud.com checks before sending (photos3 constants): sizes and dimensions.
pub const MIN_BYTES: u64 = 125;
pub const MAX_BYTES: u64 = 524_288_000;
pub const MAX_SIDE: u32 = 20_480;
pub const MAX_AREA: u64 = 419_430_400;
pub const MAX_NAME: usize = 255;

/// The types icloud.com's current uploader takes, by extension.
const TYPES: [&str; 14] = [
    "jpg", "jpeg", "heic", "heif", "png", "gif", "tif", "tiff", "webp", "mov", "mp4", "m4v", "avi",
    "dng",
];

/// Why a file was not uploaded, in words a person can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The type Photos takes none of.
    Kind(String),
    TooSmall,
    TooLarge,
    NameTooLong,
    /// The photo is already in the library.
    Duplicate,
    /// iCloud is full.
    OverQuota,
}

impl Refusal {
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            // icloud.com's own words, from its uploader (Uploader.* in ui.main.js).
            Refusal::Kind(_) => "This item’s file type isn’t supported.".into(),
            Refusal::TooSmall => "This item is too small to upload.".into(),
            Refusal::TooLarge => "Item is too large to upload.".into(),
            Refusal::NameTooLong => "File name too long.".into(),
            Refusal::Duplicate => "Duplicate item.".into(),
            Refusal::OverQuota => "Not enough space to upload item.".into(),
        }
    }
}

/// A file checked against Photos' limits before anything is sent; `None` when it may go.
#[must_use]
pub fn refusal(name: &str, size: u64) -> Option<Refusal> {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !TYPES.contains(&ext.as_str()) {
        return Some(Refusal::Kind(if ext.is_empty() { "?".into() } else { ext }));
    }
    if name.chars().count() > MAX_NAME {
        return Some(Refusal::NameTooLong);
    }
    if size < MIN_BYTES {
        return Some(Refusal::TooSmall);
    }
    if size > MAX_BYTES {
        return Some(Refusal::TooLarge);
    }
    None
}

/// Where an upload stands, for the window's tray.
#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Bytes on their way: how many of how many.
    Sending(u64, u64),
    /// Apple has named the photo it is making: kept, so an upload resumed after a crash finds
    /// it rather than making a second copy.
    Named(Uploaded),
    /// Apple making the photo from them: 0 to 100.
    Processing(f64),
}

/// A finished upload: the photo Apple made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uploaded {
    pub asset: String,
    pub master: String,
}

/// How an upload ended, short of an error in the request itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Uploaded(Uploaded),
    Refused(Refusal),
}

struct Service<'a> {
    http: &'a reqwest::Client,
    base: String,
    dsid: String,
}

impl Service<'_> {
    /// One call, retried as icloud.com retries it: up to three tries on a server error or
    /// throttling, the wait doubling from half a second, never past ten.
    async fn call(&self, endpoint: &str, body: &Value) -> Result<(u16, Value)> {
        let mut wait = Duration::from_millis(500);
        let mut last = None;
        for attempt in 1..=3 {
            let sent = self
                .http
                .post(format!("{}/photosupload/{endpoint}", self.base))
                .query(&[("dsid", &self.dsid)])
                .header("content-type", "text/plain;charset=UTF-8")
                .body(body.to_string())
                .send()
                .await;
            match sent {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let text = r.text().await.unwrap_or_default();
                    if status == 450 || status == 401 {
                        return Err(Error::ReauthRequired);
                    }
                    if status == 429 || status >= 500 {
                        last = Some(Error::Protocol(format!("{endpoint} answered {status}")));
                    } else {
                        let value = serde_json::from_str(&text).unwrap_or(Value::Null);
                        if text.contains("QUOTA_EXCEEDED") {
                            return Ok((507, value));
                        }
                        return Ok((status, value));
                    }
                }
                Err(e) => last = Some(e.into()),
            }
            if attempt < 3 {
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(10));
            }
        }
        Err(last.unwrap_or_else(|| Error::Protocol(format!("{endpoint} did not answer"))))
    }
}

/// A file sent and made into a photo, Apple still finishing it: the job to ask about
/// ([`job_states`]), and the photo it is making.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub job: String,
    pub made: Uploaded,
}

/// What [`send`] ended with, short of an error in a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sending {
    Sent(Sent),
    Refused(Refusal),
}

/// One file uploaded to the library: checked, sent, made into a photo, and waited on until
/// Apple says it is there. `library` is the zone the library lives in
/// ([`crate::photos::library_zone`]). `progress` hears each stage. For many files, [`send`]
/// each and ask about them together with [`job_states`]: nothing waits behind a slow one.
#[allow(clippy::too_many_arguments)]
pub async fn upload(
    http: &reqwest::Client,
    photosupload_root: &str,
    dsid: &str,
    library: &str,
    path: &std::path::Path,
    name: &str,
    modified_ms: i64,
    time_zone: &str,
    offset_minutes: i64,
    import_group: &str,
    progress: std::sync::Arc<dyn Fn(Stage) + Send + Sync>,
) -> Result<Outcome> {
    let sent = match send(
        http,
        photosupload_root,
        dsid,
        library,
        path,
        name,
        modified_ms,
        time_zone,
        offset_minutes,
        import_group,
        progress.clone(),
    )
    .await?
    {
        Sending::Refused(r) => return Ok(Outcome::Refused(r)),
        Sending::Sent(sent) => sent,
    };
    // Waited on until Apple says the photo is made: a second apart at first, then easing off,
    // for as long as the job is moving — a large video takes a while.
    progress(Stage::Processing(0.0));
    let mut wait = Duration::from_millis(800);
    let mut stalled = 0;
    let mut last = -1.0;
    loop {
        tokio::time::sleep(wait).await;
        let state = job_states(
            http,
            photosupload_root,
            dsid,
            std::slice::from_ref(&sent.job),
        )
        .await?
        .remove(&sent.job)
        .unwrap_or(JobState::Working(0.0));
        match state {
            JobState::Failed(code) => {
                return Err(Error::Refused(format!(
                    "iCloud couldn\u{2019}t make that photo (error {code})."
                )))
            }
            JobState::Done => return Ok(Outcome::Uploaded(sent.made)),
            JobState::Working(done) => {
                progress(Stage::Processing(done));
                if done > last {
                    last = done;
                    stalled = 0;
                } else {
                    stalled += 1;
                }
            }
        }
        // Five minutes without any movement is a job that will not finish.
        if stalled as u64 * wait.as_millis() as u64 > 300_000 {
            return Err(Error::Refused("iCloud stopped making that photo.".into()));
        }
        wait = (wait + Duration::from_millis(400)).min(Duration::from_secs(4));
    }
}

/// Where an upload job is, as Apple says.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JobState {
    /// Still making the photo: how far, 0 to 100.
    Working(f64),
    Done,
    /// It couldn't: Apple's error code.
    Failed(i64),
}

/// Many upload jobs asked about in one request (`uploadStatus` takes a list), as icloud.com
/// asks about a whole import: a job Apple doesn't mention is still working.
pub async fn job_states(
    http: &reqwest::Client,
    photosupload_root: &str,
    dsid: &str,
    jobs: &[String],
) -> Result<std::collections::HashMap<String, JobState>> {
    let service = Service {
        http,
        base: photosupload_root.trim_end_matches('/').to_string(),
        dsid: dsid.to_string(),
    };
    let mut out = std::collections::HashMap::new();
    for some in jobs.chunks(100) {
        let (_, status) = service
            .call("uploadStatus", &json!({ "uploadJobIds": some }))
            .await?;
        for job in some {
            let state = status.get(job).cloned().unwrap_or(Value::Null);
            let s = if let Some(code) = state.get("errorCode").and_then(Value::as_i64) {
                JobState::Failed(code)
            } else {
                let done = state.get("progress").and_then(Value::as_f64).unwrap_or(0.0);
                if done >= 100.0 {
                    JobState::Done
                } else {
                    JobState::Working(done)
                }
            };
            out.insert(job.clone(), s);
        }
    }
    Ok(out)
}

/// One file checked, sent, and made into a photo, returning as soon as Apple has the job: what
/// is left is Apple's to finish ([`job_states`]).
#[allow(clippy::too_many_arguments)]
pub async fn send(
    http: &reqwest::Client,
    photosupload_root: &str,
    dsid: &str,
    library: &str,
    path: &std::path::Path,
    name: &str,
    modified_ms: i64,
    time_zone: &str,
    offset_minutes: i64,
    import_group: &str,
    progress: std::sync::Arc<dyn Fn(Stage) + Send + Sync>,
) -> Result<Sending> {
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not read {name}: {e}")))?
        .len();
    if let Some(r) = refusal(name, size) {
        return Ok(Sending::Refused(r));
    }
    let service = Service {
        http,
        base: photosupload_root.trim_end_matches('/').to_string(),
        dsid: dsid.to_string(),
    };

    // 1. Where the bytes go.
    let key = uuid::Uuid::new_v4().to_string();
    let (status, answer) = service
        .call(
            "createUploadUrl",
            &json!({ "zoneName": library, "assets": { &key: size } }),
        )
        .await?;
    if status == 507 {
        return Ok(Sending::Refused(Refusal::OverQuota));
    }
    let url = answer
        .pointer(&format!("/uploadUrls/{key}"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol(format!("createUploadUrl gave no address ({status})")))?
        .to_string();

    // 2. The bytes, straight off disk in pieces and counted on the way past, never all in
    //    memory: a video can be half a gigabyte.
    progress(Stage::Sending(0, size));
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not open {name}: {e}")))?;
    let counted = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stream = futures_util::stream::try_unfold(
        (file, counted, progress.clone()),
        move |(mut file, counted, progress)| async move {
            use tokio::io::AsyncReadExt;
            let mut buf = vec![0u8; 256 * 1024];
            let n = file.read(&mut buf).await?;
            if n == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            buf.truncate(n);
            let so_far =
                counted.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed) + n as u64;
            progress(Stage::Sending(so_far, size));
            Ok(Some((buf, (file, counted, progress))))
        },
    );
    let sent = http
        .post(&url)
        .header("content-type", "text/plain")
        .header("content-length", size)
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await?;
    let sent_status = sent.status();
    let text = sent.text().await.unwrap_or_default();
    if text.contains("QUOTA_EXCEEDED") {
        return Ok(Sending::Refused(Refusal::OverQuota));
    }
    if !sent_status.is_success() {
        return Err(Error::Protocol(format!(
            "the upload answered {sent_status}"
        )));
    }
    let receipt: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("the upload's receipt was not JSON: {e}")))?;
    if receipt.get("isDuplicate").and_then(Value::as_bool) == Some(true) {
        return Ok(Sending::Refused(Refusal::Duplicate));
    }
    let single = receipt
        .get("singleFile")
        .filter(|s| s.get("receipt").is_some())
        .ok_or_else(|| Error::Protocol("the upload gave no receipt".into()))?
        .clone();
    progress(Stage::Sending(size, size));

    // 3. The receipt made a photo.
    let (status, made) = service
        .call(
            "putAsset",
            &json!({
                "zoneName": library,
                "localTimeZoneId": time_zone,
                "importGroup": import_group,
                "files": [{
                    "fileName": name,
                    "lastModDate": modified_ms,
                    "timeZoneOffset": offset_minutes,
                    "singleFileUploadRequest": single,
                }],
            }),
        )
        .await?;
    if status == 507 {
        return Ok(Sending::Refused(Refusal::OverQuota));
    }
    let first = made.get(0).cloned().unwrap_or(Value::Null);
    let (Some(job), Some(asset), Some(master)) = (
        first.get("uploadJobId").and_then(Value::as_str),
        first.get("cplAsset").and_then(Value::as_str),
        first.get("cplMaster").and_then(Value::as_str),
    ) else {
        let said = first
            .pointer("/response/errorMessage")
            .and_then(Value::as_str)
            .unwrap_or("no photo was made");
        return Err(Error::Refused(format!(
            "iCloud didn’t take that photo: {said}"
        )));
    };

    let made = Uploaded {
        asset: asset.to_string(),
        master: master.to_string(),
    };
    progress(Stage::Named(made.clone()));
    Ok(Sending::Sent(Sent {
        job: job.to_string(),
        made,
    }))
}

#[cfg(test)]
mod tests {

    /// Every job asked about in one request, each answered as Apple has it: done, still
    /// working, or failed; one Apple doesn't mention is still working.
    #[tokio::test]
    async fn many_jobs_are_asked_about_at_once() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/photosupload/uploadStatus$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "a": { "progress": 100.0 },
                "b": { "progress": 40.0 },
                "c": { "errorCode": 7 },
            })))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let jobs: Vec<String> = ["a", "b", "c", "d"].map(String::from).to_vec();
        let states = job_states(&http, &server.uri(), "1", &jobs).await.unwrap();
        assert_eq!(states["a"], JobState::Done);
        assert_eq!(states["b"], JobState::Working(40.0));
        assert_eq!(states["c"], JobState::Failed(7));
        assert_eq!(states["d"], JobState::Working(0.0));
    }
    use super::*;

    /// Files Photos would refuse are refused here, with the reason, before anything is sent.
    #[test]
    fn files_are_checked_before_sending() {
        assert_eq!(refusal("IMG_0001.JPG", 2_000_000), None);
        assert_eq!(refusal("clip.mov", 80_000_000), None);
        assert_eq!(
            refusal("notes.txt", 2_000),
            Some(Refusal::Kind("txt".into()))
        );
        assert_eq!(refusal("noext", 2_000), Some(Refusal::Kind("?".into())));
        assert_eq!(refusal("a.jpg", 10), Some(Refusal::TooSmall));
        assert_eq!(refusal("a.jpg", MAX_BYTES + 1), Some(Refusal::TooLarge));
        assert_eq!(
            refusal(&format!("{}.jpg", "x".repeat(300)), 2_000),
            Some(Refusal::NameTooLong)
        );
    }
}
