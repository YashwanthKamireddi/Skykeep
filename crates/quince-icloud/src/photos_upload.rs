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

/// The types icloud.com's current uploader takes, by extension. Not AVI, though its file
/// picker offers it: its own list of what it uploads leaves AVI out, no Apple app reads one,
/// and a tool on the same upload service finds it refused (isynca compat.py), so an AVI went
/// up whole for nothing.
const TYPES: [&str; 13] = [
    "jpg", "jpeg", "heic", "heif", "png", "gif", "tif", "tiff", "webp", "mov", "mp4", "m4v", "dng",
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
    /// Apple took the bytes and turned the file down for good, in its own words: a video in a
    /// form its server won't convert ("uploaded media is unsupported for transcoding"), say.
    Rejected(String),
    /// A photo larger than Photos takes: its width and height in pixels.
    Dimensions(u32, u32),
    /// Read before sending, a form Apple's server is known to turn down: what it is, in words
    /// ("an AVI video", "Motion JPEG video, …").
    Unsupported(String),
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
            // Sending it again gets the same answer. An iPhone converts a video before it
            // uploads; here it goes as it is, so saved as MP4 (H.264) or JPEG it can be added.
            Refusal::Rejected(said) | Refusal::Unsupported(said) => format!(
                "iCloud Photos can\u{2019}t take this file as it is ({said}). Saved as an MP4 \
                 video (H.264) or a JPEG photo, it can be added."
            ),
            Refusal::Dimensions(w, h) => format!(
                "This photo is too large for iCloud Photos ({w} \u{d7} {h} pixels). It can be \
                 up to {MAX_SIDE} pixels a side and {} million pixels in all.",
                MAX_AREA / 1_000_000
            ),
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

/// A file's contents checked against what Photos is known to refuse, reading only its headers
/// (crate::upload_probe): a photo past Photos' size in pixels, a container no Apple app reads,
/// a video codec Apple's server turns down. `None` when it may go, or can't be read here: then
/// Apple decides. Blocking, but a few small reads.
#[must_use]
pub fn inspect(path: &std::path::Path) -> Option<Refusal> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    crate::upload_probe::inspect(&mut file, len)
}

/// The name a file goes to Photos under. Apple refuses a `.m4v` by its name alone (415,
/// "unsupported file type"), even holding plain H.264 and AAC, and takes the same bytes named
/// otherwise (isynca PR #9, live on the same upload service): an M4V is an MP4 by another
/// name, so it goes as one. The file itself is never renamed.
fn upload_name(name: &str) -> std::borrow::Cow<'_, str> {
    match name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("m4v") => format!("{stem}.mp4").into(),
        _ => name.into(),
    }
}

/// Whether Apple's answer about one file (`putAsset`'s `response`) is final. Apple says so
/// with `isRetryable` when it says; otherwise its status does: a 4xx is a verdict on the file
/// (415, a form its server won't convert; 413, too large), save 408 and 429, which are about
/// the moment. A 5xx, or no status, may go better another time.
fn settled(response: &Value) -> bool {
    if let Some(again) = response.get("isRetryable").and_then(Value::as_bool) {
        return !again;
    }
    response
        .get("status")
        .and_then(Value::as_i64)
        .is_some_and(|s| (400..500).contains(&s) && s != 408 && s != 429)
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
                    let text = crate::bounded::reply(r).await?;
                    if crate::session_lapsed(status) || status == 401 {
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
    uploads: &reqwest::Client,
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
        uploads,
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
    uploads: &reqwest::Client,
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
    // What is inside, before a byte goes: Apple only says no once it has the whole file.
    let looked = path.to_path_buf();
    if let Ok(Some(r)) = tokio::task::spawn_blocking(move || inspect(&looked)).await {
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
        (file, counted.clone(), progress.clone()),
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
    // Through the uploads client, watched while it goes (crate::upload): a video that takes
    // longer than a minute to send was cut off by the read timeout part way.
    let request = uploads
        .post(&url)
        .header("content-type", "text/plain")
        .header("content-length", size)
        .body(reqwest::Body::wrap_stream(stream));
    let (sent_status, text) = crate::upload::send(request, &counted, size).await?;
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
                    "fileName": upload_name(name),
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
        // A verdict on the file is final: tried again, the whole file went three times over
        // for the same answer (D#262).
        if first.get("response").is_some_and(settled) {
            return Ok(Sending::Refused(Refusal::Rejected(said.to_string())));
        }
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

    /// Apple's own verdict on a file ends its upload there, said in words that say what to do:
    /// sent again, a whole video went three times over for the same answer (D#262). A refusal
    /// about the moment (5xx) still errs, to be tried again.
    #[tokio::test]
    async fn a_file_apple_will_not_convert_is_refused_once() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for (response, final_) in [
            (
                json!({"status": 415, "isRetryable": false,
                       "errorMessage": "uploaded media is unsupported for transcoding"}),
                true,
            ),
            (
                json!({"status": 415, "errorMessage": "uploaded media is unsupported for transcoding"}),
                true,
            ),
            (json!({"status": 503, "errorMessage": "try later"}), false),
            (json!({"status": 429, "errorMessage": "slow down"}), false),
        ] {
            let server = MockServer::start().await;
            let base = server.uri();
            // Any key: the address is the same whichever uuid was asked for.
            Mock::given(method("POST"))
                .and(path_regex(r".*/photosupload/createUploadUrl$"))
                .respond_with(move |req: &wiremock::Request| {
                    let body: Value = serde_json::from_slice(&req.body).unwrap();
                    let key = body["assets"]
                        .as_object()
                        .unwrap()
                        .keys()
                        .next()
                        .unwrap()
                        .clone();
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"uploadUrls": {key: format!("{base}/bytes")}}))
                })
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/bytes$"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "singleFile": {"fileChecksum": "c", "size": 200, "receipt": "r"}
                })))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/photosupload/putAsset$"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!([{ "response": response }])),
                )
                .mount(&server)
                .await;
            let file = std::env::temp_dir().join(format!(
                "skykeep-refusal-{}-{}.mp4",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::write(&file, vec![7u8; 200]).unwrap();
            let http = reqwest::Client::new();
            let got = send(
                &http,
                &http,
                &server.uri(),
                "1",
                "PrimarySync",
                &file,
                "clip.mp4",
                0,
                "UTC",
                0,
                "g",
                std::sync::Arc::new(|_| {}),
            )
            .await;
            let _ = std::fs::remove_file(&file);
            if final_ {
                let Ok(Sending::Refused(r)) = got else {
                    panic!("a final refusal: {got:?}");
                };
                assert_eq!(
                    r,
                    Refusal::Rejected("uploaded media is unsupported for transcoding".into())
                );
                assert!(r.words().contains("MP4"), "says what to do: {}", r.words());
            } else {
                assert!(
                    matches!(got, Err(Error::Refused(_))),
                    "about the moment, tried again: {got:?}"
                );
            }
        }
    }

    /// A file whose contents Apple is known to refuse is refused before any request at all:
    /// no upload address asked for, no byte sent. One that is fine still goes, and an M4V goes
    /// named as the MP4 it is.
    #[tokio::test]
    async fn contents_are_checked_before_any_request() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        // A JPEG 30 000 px wide: SOI, then its frame header, then a scan.
        let mut wide = vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8];
        wide.extend_from_slice(&2000u16.to_be_bytes());
        wide.extend_from_slice(&30_000u16.to_be_bytes());
        wide.extend_from_slice(&[1, 1, 0x11, 0, 0xFF, 0xDA, 0, 8, 1, 1, 0, 0, 0x3F, 0]);
        wide.resize(400, 0);
        // An AVI, named as an MP4.
        let mut avi = b"RIFF\0\0\0\0AVI LIST".to_vec();
        avi.resize(400, 0);
        // An MP4 of MPEG-4 Part 2 video: ftyp, then moov → trak → mdia → hdlr + minf → stbl →
        // stsd → mp4v.
        let bx = |kind: &[u8; 4], body: &[u8]| {
            let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out
        };
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(b"vide");
        hdlr.extend_from_slice(&[0u8; 13]);
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend(bx(b"mp4v", &[0u8; 30]));
        let stbl = bx(b"stbl", &bx(b"stsd", &stsd));
        let mut mdia = bx(b"hdlr", &hdlr);
        mdia.extend(bx(b"minf", &stbl));
        let mut mp4v = bx(b"ftyp", b"isom\0\0\0\0isom");
        mp4v.extend(bx(b"moov", &bx(b"trak", &bx(b"mdia", &mdia))));
        mp4v.extend(bx(b"mdat", &[0u8; 300]));

        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("POST"))
            .and(path_regex(r".*/photosupload/createUploadUrl$"))
            .respond_with(move |req: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                let key = body["assets"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .next()
                    .unwrap()
                    .clone();
                ResponseTemplate::new(200)
                    .set_body_json(json!({"uploadUrls": {key: format!("{base}/bytes")}}))
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/bytes$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "singleFile": {"fileChecksum": "c", "size": 400, "receipt": "r"}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/photosupload/putAsset$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!([{ "uploadJobId": "j", "cplAsset": "a", "cplMaster": "m" }]),
                ),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let go = |bytes: Vec<u8>, name: &'static str| {
            let http = http.clone();
            let root = server.uri();
            async move {
                let file = std::env::temp_dir().join(format!(
                    "skykeep-precheck-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
                ));
                std::fs::write(&file, bytes).unwrap();
                let got = send(
                    &http,
                    &http,
                    &root,
                    "1",
                    "PrimarySync",
                    &file,
                    name,
                    0,
                    "UTC",
                    0,
                    "g",
                    std::sync::Arc::new(|_| {}),
                )
                .await;
                let _ = std::fs::remove_file(&file);
                got
            }
        };
        async fn asked(server: &MockServer) -> usize {
            server
                .received_requests()
                .await
                .unwrap_or_default()
                .iter()
                .filter(|r| r.url.path().ends_with("createUploadUrl"))
                .count()
        }
        for (bytes, name, want) in [
            (wide, "pano.jpg", Refusal::Dimensions(30_000, 2000)),
            (avi, "clip.mp4", Refusal::Unsupported("an AVI video".into())),
        ] {
            let got = go(bytes, name).await;
            assert!(
                matches!(&got, Ok(Sending::Refused(r)) if *r == want),
                "{got:?}"
            );
        }
        let got = go(mp4v, "old.mp4").await;
        assert!(
            matches!(&got, Ok(Sending::Refused(Refusal::Unsupported(w))) if w.contains("MPEG-4 Part 2")),
            "{got:?}"
        );
        assert_eq!(asked(&server).await, 0, "nothing asked for a refused file");

        // A file that is fine goes; an M4V goes as an MP4.
        let got = go(vec![7u8; 400], "clip.M4V").await;
        assert!(matches!(got, Ok(Sending::Sent(_))), "{got:?}");
        assert_eq!(asked(&server).await, 1);
        let put = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.url.path().ends_with("putAsset"))
            .unwrap();
        let body: Value = serde_json::from_slice(&put.body).unwrap();
        assert_eq!(body["files"][0]["fileName"], "clip.mp4");
    }

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
        assert_eq!(
            refusal("clip.AVI", 2_000),
            Some(Refusal::Kind("avi".into()))
        );
        assert_eq!(refusal("clip.m4v", 2_000), None);
        assert_eq!(refusal("a.jpg", 10), Some(Refusal::TooSmall));
        assert_eq!(refusal("a.jpg", MAX_BYTES + 1), Some(Refusal::TooLarge));
        assert_eq!(
            refusal(&format!("{}.jpg", "x".repeat(300)), 2_000),
            Some(Refusal::NameTooLong)
        );
    }
}
