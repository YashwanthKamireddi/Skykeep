//! iCloud Drive's web service.
//!
//! Drive is fetched a folder at a time from `retrieveItemDetailsInFolders`, which takes a
//! list of folder identifiers and answers with their contents.

use crate::{Error, Result};
use quince_core::drive::DriveItem;
use serde_json::{json, Value};

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Apple sends dates as milliseconds in some fields and ISO strings in others.
fn seconds(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64().map(|ms| ms / 1000),
        Value::String(s) => jiff::Timestamp::strptime("%Y-%m-%dT%H:%M:%S%z", s)
            .ok()
            .map(|t| t.as_second())
            .or_else(|| s.parse::<jiff::Timestamp>().ok().map(|t| t.as_second())),
        _ => None,
    }
}

fn item_from(v: &Value) -> Option<DriveItem> {
    Some(DriveItem {
        drivewsid: string(v, "drivewsid")?,
        parent_id: string(v, "parentId"),
        name: string(v, "name").unwrap_or_else(|| "Untitled".into()),
        extension: string(v, "extension"),
        kind: string(v, "type").unwrap_or_else(|| "FILE".into()),
        // Apple sends -1 for things that have no size, which is not a size.
        size: v.get("size").and_then(Value::as_i64).filter(|n| *n >= 0),
        child_count: v.get("directChildrenCount").and_then(Value::as_i64),
        created_at: seconds(v, "dateCreated"),
        modified_at: seconds(v, "dateModified").or_else(|| seconds(v, "dateChanged")),
        etag: string(v, "etag"),
    })
}

/// Read the contents of one folder from a `retrieveItemDetailsInFolders` response.
#[must_use]
pub fn parse_folder(body: &Value) -> Vec<DriveItem> {
    body.as_array()
        .and_then(|folders| folders.first())
        .and_then(|folder| folder.get("items"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(item_from).collect())
        .unwrap_or_default()
}

/// A folder's contents, from a `retrieveItemDetailsInFolders` answer that has been checked
/// to be one: a stored folder is replaced by whatever this returns, so an answer that is not
/// a listing must never read as an empty folder.
///
/// Measured: a folder that no longer exists comes back `200` with `"items": []` and
/// `"status": "ID_INVALID"` — exactly what an empty folder looks like, but for the status.
/// A real folder's status is `"OK"`.
fn listing(body: &Value) -> Result<Vec<DriveItem>> {
    let folder = body
        .as_array()
        .and_then(|folders| folders.first())
        .ok_or_else(|| Error::Protocol("Drive answered without the folder asked for".into()))?;
    match folder.get("status").and_then(Value::as_str) {
        None | Some("OK") => {}
        Some("ID_INVALID") => return Err(Error::NotFound),
        Some(other) => {
            return Err(Error::Protocol(format!(
                "Drive could not list the folder: {other}"
            )))
        }
    }
    if !folder.get("items").is_some_and(Value::is_array) {
        return Err(Error::Protocol(
            "Drive answered without the folder's items".into(),
        ));
    }
    Ok(parse_folder(body))
}

/// Fetch one folder's contents and store them.
pub async fn sync_folder(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
    folder_id: &str,
) -> Result<usize> {
    let items = fetch_folder(http, root, params, folder_id).await?;
    store_folder(library, folder_id, &items)
}

/// Store a folder's contents fetched by [`fetch_folder`].
pub fn store_folder(
    library: &mut quince_core::Library,
    folder_id: &str,
    items: &[DriveItem],
) -> Result<usize> {
    library.put_drive_children(folder_id, items)?;
    library.mark_synced("drive", None)?;
    Ok(items.len())
}

/// Ask Apple for one folder's contents. Touches no local state, so it can run without
/// holding the library every screen reads.
pub async fn fetch_folder(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    folder_id: &str,
) -> Result<Vec<DriveItem>> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));

    let url = format!(
        "{}/retrieveItemDetailsInFolders",
        root.trim_end_matches('/')
    );
    let body = json!([{ "drivewsid": folder_id, "partialData": false }]);

    let response = http.post(url).query(&query).json(&body).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if busy_status(status) {
        return Err(Error::Refused(BUSY.into()));
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }

    let parsed: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))?;
    listing(&parsed)
}

/// Bare docwsid: `FILE::com.apple.CloudDocs::<uuid>` carries it after the last `::`.
fn docwsid_of(drivewsid: &str) -> &str {
    drivewsid.rsplit("::").next().unwrap_or(drivewsid)
}

/// Where to fetch a file's bytes from — a signed, time-limited URL Apple hands out per
/// request.
///
/// Downloads are served from the `docws` host, not `drivews`: a different subdomain from
/// everything else in this module, measured against the real service rather than assumed.
pub async fn download_url(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<String> {
    link(http, docws_root, params, item, false).await
}

/// [`download_url`] for exactly the version `item` names: refused ([`Error::Conflict`]) when
/// Apple has a newer one. For reading a file in parts, kept part by part under its version.
pub async fn download_url_of_version(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<String> {
    link(http, docws_root, params, item, true).await
}

async fn link(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    exact: bool,
) -> Result<String> {
    // The zone in the path is the file's own. An app library — Keynote's, Shortcuts', a
    // third party's — is a zone of its own, and asking for one of its files under the
    // ordinary Drive zone gets "Could not find document" rather than the file. Every file
    // in every app library was undownloadable while this said com.apple.CloudDocs.
    let url = format!(
        "{}/ws/{}/download/by_id",
        docws_root.trim_end_matches('/'),
        zone_of(&item.drivewsid).unwrap_or(DOCUMENTS_ZONE)
    );
    let mut query = params;
    query.push((
        "document_id".into(),
        docwsid_of(&item.drivewsid).to_string(),
    ));

    let response = http.get(url).query(&query).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if busy_status(status) {
        return Err(Error::Refused(BUSY.into()));
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "couldn't prepare a download ({status}): {}",
            text.chars().take(160).collect::<String>()
        )));
    }

    let body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("download reply was not JSON: {e}")))?;
    // The link is to the version Apple has now (measured: `double_etag` is the item's own
    // etag). A file changed elsewhere since it was listed would be read in parts of two
    // versions and kept as one: refused instead, and read again once listed afresh.
    if let (true, Some(known), Some(serving)) = (
        exact,
        item.etag.as_deref(),
        body.get("double_etag").and_then(Value::as_str),
    ) {
        if known != serving {
            return Err(Error::Conflict);
        }
    }
    download_link_in(&body)
        .ok_or_else(|| Error::Protocol("Apple's reply carried no download URL".into()))
}

/// The address a `download/by_id` reply gives for the bytes: `data_token` for a file, and
/// `package_token` for a document that is a package — a Pages, Numbers or Keynote file, an
/// app's bundle — which Apple serves whole from `packageDownload` (timlaing/pyicloud
/// `drive.py`, mandarons/icloudpy#1, mandarons/icloud-docker#461). Read for `data_token`
/// alone, every such document was "Apple's reply carried no download URL".
fn download_link_in(body: &Value) -> Option<String> {
    ["/data_token/url", "/package_token/url"]
        .iter()
        .find_map(|at| body.pointer(at).and_then(Value::as_str))
        .filter(|url| !url.is_empty())
        .map(ToString::to_string)
}

/// Download a file's bytes to `destination`, streaming rather than holding the whole file
/// in memory — Drive is exactly where a large file is likely.
pub async fn download_to(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    destination: &std::path::Path,
) -> Result<()> {
    download_with_progress(
        http,
        docws_root,
        params,
        item,
        destination,
        None,
        Placing::New,
    )
    .await
}

/// How a download takes its place at its destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placing {
    /// Only where nothing is: anything already under the name is left as it is, and the
    /// download refused.
    New,
    /// In place of what is there: for a download resumed into the folder made for it, where a
    /// file of the name is an earlier copy of this one. Never over a folder; a link there is
    /// replaced itself, never followed.
    Replace,
}

/// A partial download's own file, removed when the download stops short for any reason — an
/// error, or the download itself being dropped (cancelled) part-way, which no `?` sees.
struct Partial(Option<std::path::PathBuf>);

impl Partial {
    /// It is whole and in place: nothing to remove.
    fn keep(&mut self) {
        self.0 = None;
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// When `item` was last changed in iCloud, as the time a file on disk carries; `None` when
/// Apple didn't say (or said before 1970).
#[must_use]
pub fn changed_at(item: &DriveItem) -> Option<std::time::SystemTime> {
    let seconds = u64::try_from(item.modified_at?).ok()?;
    std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(seconds))
}

/// [`download_to`], telling `progress` how many bytes have arrived, of how many (the size
/// Apple sends with the bytes, else the item's own; 0 when neither is known), and placed as
/// `placing` says. The file is only ever moved into place whole: every byte Apple said it would
/// send, and no fewer.
pub async fn download_with_progress(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    destination: &std::path::Path,
    progress: Option<Progress>,
    placing: Placing,
) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    // An empty file has no bytes to fetch, and Apple answers asking for a link to them with a
    // 400 (picklepete/pyicloud#301): it is saved empty without asking.
    let response = if item.size == Some(0) {
        None
    } else {
        let url = download_url(http, docws_root, params, item).await?;
        let response = http.get(&url).send().await?;
        if busy_status(response.status()) {
            return Err(Error::Refused(BUSY.into()));
        }
        if !response.status().is_success() {
            return Err(Error::Protocol(format!(
                "the download itself failed ({})",
                response.status()
            )));
        }
        Some(response)
    };
    let promised = response
        .as_ref()
        .and_then(reqwest::Response::content_length);
    let total = promised
        .or_else(|| item.size.and_then(|n| u64::try_from(n).ok()))
        .unwrap_or(0);

    // Into a folder that is already there, never one made on the way: the caller chose the
    // folder (Downloads), and a name that reached for others would have them made for it.
    if !destination.parent().is_some_and(std::path::Path::is_dir) {
        return Err(Error::Protocol(
            "the folder to save into is not there".into(),
        ));
    }
    // Into a file of its own beside the destination, moved into place only once it is all
    // there and only if nothing has taken the name meanwhile. Written straight to the name,
    // a download that broke off left a truncated file under the real one.
    let base = destination.with_extension(match destination.extension() {
        Some(ext) => format!("{}.part", ext.to_string_lossy()),
        None => "part".into(),
    });
    // A partial file left by a download cut off (a crash, a power cut) keeps its name: this one
    // takes the next free one beside it, rather than failing on "File exists" for good.
    let mut partial = base.clone();
    let mut opened = None;
    for n in 1..=100 {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .await
        {
            Ok(file) => {
                opened = Some(file);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut name = base.as_os_str().to_owned();
                name.push(format!(".{n}"));
                partial = std::path::PathBuf::from(name);
            }
            Err(e) => return Err(e.into()),
        }
    }
    let Some(mut file) = opened else {
        return Err(Error::Protocol(
            "too many unfinished downloads of this file are in the way".into(),
        ));
    };
    // From here on, however this ends short of the file in place, its partial file goes.
    let mut cleanup = Partial(Some(partial.clone()));
    let mut received = 0u64;
    if let Some(response) = response {
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            received += chunk.len() as u64;
            if let Some(progress) = &progress {
                progress(received, total.max(received));
            }
        }
    }
    file.flush().await?;
    file.sync_all().await?;
    drop(file);
    // Fewer bytes than were promised is a connection that ended early, whatever it said: never
    // kept, and asked again.
    if promised.is_some_and(|n| n != received) {
        return Err(Error::Refused(BUSY.into()));
    }
    let placed = place(&partial, destination, placing, &mut cleanup).await;
    // The file keeps the date it was last changed in iCloud, as a copy from any other disk
    // does: sorted by date it sits where it belongs, and a resumed folder download tells by it
    // which version it already has. Set once it is whole, closed and in place: set on the open
    // part file, Windows put today's date back over it as the written file was closed.
    // Best effort: a disk that won't take a date keeps today's.
    if placed.is_ok() {
        if let Some(when) = changed_at(item) {
            let at = destination.to_path_buf();
            let _ = tokio::task::spawn_blocking(move || {
                std::fs::File::options()
                    .write(true)
                    .open(&at)
                    .and_then(|f| f.set_modified(when))
            })
            .await;
        }
    }
    placed
}

/// The whole part file moved to `destination` as `placing` says, and `cleanup` told to keep it.
async fn place(
    partial: &std::path::Path,
    destination: &std::path::Path,
    placing: Placing,
    cleanup: &mut Partial,
) -> Result<()> {
    let partial = partial.to_path_buf();
    if placing == Placing::Replace {
        if std::fs::symlink_metadata(destination).is_ok_and(|m| m.is_dir()) {
            return Err(Error::Protocol(
                "a folder is where the file would go".into(),
            ));
        }
        let (from, to) = (partial.clone(), destination.to_path_buf());
        let moved = tokio::task::spawn_blocking(move || quince_core::files::rename(&from, &to))
            .await
            .map_err(std::io::Error::other)
            .and_then(|r| r);
        if moved.is_ok() {
            cleanup.keep();
        }
        return moved.map_err(Into::into);
    }
    // A link, unlike a rename, refuses an existing name: nothing already there is replaced.
    match tokio::fs::hard_link(&partial, destination).await {
        Ok(()) => {
            cleanup.keep();
            tokio::fs::remove_file(&partial).await.ok();
            Ok(())
        }
        // A filesystem without links — a FAT or exFAT drive — takes a rename, checked first.
        // So does a file Windows holds for a moment (the virus scanner, looking at it as it was
        // closed): the rename waits for it to be let go (quince_core::files).
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists && !destination.exists() => {
            let (from, to) = (partial.clone(), destination.to_path_buf());
            let moved = tokio::task::spawn_blocking(move || quince_core::files::rename(&from, &to))
                .await
                .map_err(std::io::Error::other)
                .and_then(|r| r);
            if moved.is_ok() {
                cleanup.keep();
            }
            moved.map_err(Into::into)
        }
        Err(e) => Err(e.into()),
    }
}

/// Whether a status is Drive saying it is too busy for now: asked again shortly, it passes.
fn busy_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

/// Whether `e` is one that passes if the same request is made again shortly: the connection
/// dropping or timing out, or Drive saying it is busy. Anything else — a sign-in to renew, a
/// file no longer there, a full disk — is said at once.
#[must_use]
pub fn transient(e: &Error) -> bool {
    match e {
        Error::Network(_) | Error::Busy(_) => true,
        Error::Refused(m) => m == BUSY,
        _ => false,
    }
}

/// How many times [`patiently`] makes a request before it gives up.
pub const TRIES: u32 = 4;

/// `op`, made again while it fails in a way that passes ([`transient`]) — up to [`TRIES`]
/// times in all, waiting longer each time (about one, two, then four seconds, a little apart
/// for each caller so many at once do not come back together). A folder of a thousand files
/// meets a dropped connection or a busy moment somewhere; each is ridden out here rather than
/// reported as a file that did not download.
pub async fn patiently<T, F, Fut>(mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    patiently_after(std::time::Duration::from_secs(1), &mut op).await
}

/// [`patiently`], with the first wait given: short in tests.
pub async fn patiently_after<T, F, Fut>(first: std::time::Duration, op: &mut F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut tried = 0;
    loop {
        tried += 1;
        match op().await {
            Err(e) if transient(&e) && tried < TRIES => {
                // Up to half the wait again, by chance: callers that met the same busy moment
                // come back apart.
                let jitter = first.mul_f64(f64::from(uuid::Uuid::new_v4().as_bytes()[0]) / 512.0);
                tokio::time::sleep(first * 2u32.pow(tried - 1) + jitter).await;
            }
            done => return done,
        }
    }
}

/// What a `createFolders` reply says happened.
///
/// Apple answers 200 either way. A folder it made comes back described, with `status: "OK"`;
/// one it declined comes back as the client id and a status alone — `UNKNOWN` for a name
/// already used in that folder, which is what a second submit of the same name produced. That
/// used to surface as "Apple created the folder but didn't describe it", which was false.
fn created_folder(body: &Value, name: &str) -> Result<DriveItem> {
    let folder = body
        .pointer("/folders/0")
        .ok_or_else(|| Error::Protocol("Drive's answer named no folder at all".into()))?;
    let status = folder.get("status").and_then(Value::as_str).unwrap_or("OK");
    if status != "OK" {
        return Err(Error::Protocol(format!(
            "iCloud Drive didn't create \u{201c}{name}\u{201d} ({status}). \
             Something with that name may already be here."
        )));
    }
    item_from(folder)
        .ok_or_else(|| Error::Protocol("Drive's answer described the folder without an id".into()))
}

/// Create many folders inside `parent_id` in one request, as `createFolders` takes them — a
/// folder of 256 folders (a git repository's `objects`) is one round trip, not 256. Each name
/// answers with its new folder's id, or why it was not made; the answer is matched by the
/// `clientId` each was sent with.
/// A name iCloud Drive won't take, refused here in icloud.com's words: empty, hidden (a
/// leading dot, which a Mac would never show), or holding ":" or a line break.
pub fn unusable_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Refused(
            "A name can't be empty. Please choose a different name.".into(),
        ));
    }
    if name.starts_with('.') {
        return Err(Error::Refused(
            "A name shouldn\u{2019}t begin with a \u{201c}.\u{201d} (dot). Please choose a different name.".into(),
        ));
    }
    if name.contains([':', '\n', '\r']) {
        return Err(Error::Refused(format!(
            "The name \u{201c}{name}\u{201d} can\u{2019}t be used. Try using a name with fewer characters, or with no punctuation marks."
        )));
    }
    Ok(())
}

pub async fn create_folders(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    parent_id: &str,
    names: &[String],
) -> Result<Vec<(String, std::result::Result<String, String>)>> {
    let mut out = Vec::with_capacity(names.len());
    for chunk in names.chunks(100) {
        // (as asked, as sent, client id): answered under the name asked, which is how the caller
        // looks each one up — "Notes " is made as "Notes".
        let asked: Vec<(String, String, String)> = chunk
            .iter()
            .map(|n| {
                (
                    n.clone(),
                    n.trim().to_string(),
                    uuid::Uuid::new_v4().to_string(),
                )
            })
            .collect();
        let body = json!({
            "destinationDrivewsId": parent_id,
            "folders": asked.iter().map(|(_, name, id)| json!({ "name": name, "clientId": id })).collect::<Vec<_>>(),
        });
        let url = format!("{}/createFolders", root.trim_end_matches('/'));
        let reply = post_json(http, &url, &params, &body).await?;
        let made: Vec<Value> = reply
            .get("folders")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (key, name, client_id) in asked {
            let folder = made
                .iter()
                .find(|f| f.get("clientId").and_then(Value::as_str) == Some(client_id.as_str()))
                .or_else(|| {
                    made.iter()
                        .find(|f| f.get("name").and_then(Value::as_str) == Some(name.as_str()))
                });
            let result = match folder {
                Some(f)
                    if f.get("status")
                        .and_then(Value::as_str)
                        .is_none_or(|s| s == "OK") =>
                {
                    item_from(f).map(|i| i.drivewsid).ok_or_else(|| {
                        "Drive's answer described the folder without an id".to_string()
                    })
                }
                Some(f) => Err(format!(
                    "iCloud Drive didn't create \u{201c}{name}\u{201d} ({}).",
                    f.get("status").and_then(Value::as_str).unwrap_or("?")
                )),
                None => Err(format!(
                    "iCloud Drive said nothing about \u{201c}{name}\u{201d}."
                )),
            };
            out.push((key, result));
        }
    }
    Ok(out)
}

/// Create a folder inside `parent_id`. Returns the item Apple stored.
pub async fn create_folder(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    parent_id: &str,
    name: &str,
) -> Result<DriveItem> {
    let name = name.trim();
    unusable_name(name)?;

    let url = format!("{}/createFolders", root.trim_end_matches('/'));
    let body = json!({
        "destinationDrivewsId": parent_id,
        "folders": [{ "name": name, "clientId": uuid::Uuid::new_v4().to_string() }],
    });

    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive refused the folder ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }

    let body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))?;
    created_folder(&body, name)
}

/// The parent's own listing, straight from Apple — for finding an item's *current* etag
/// and for confirming afterward that a write actually landed.
///
/// `retrieveItemDetailsInFolders` is the only place Drive's web service will hand out an
/// etag, so this is called before a write (to get one that has not gone stale) and after
/// one (to confirm the change is what Apple now has, not just what it answered 200 to).
async fn folder_listing(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    folder_id: &str,
) -> Result<Vec<DriveItem>> {
    listing(&raw_listing(http, root, params, folder_id).await?)
}

/// A folder's listing as Apple sent it, every field kept: a copy is asked for by `item_id`,
/// which only this has.
async fn raw_listing(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    folder_id: &str,
) -> Result<Value> {
    let url = format!(
        "{}/retrieveItemDetailsInFolders",
        root.trim_end_matches('/')
    );
    let body = json!([{ "drivewsid": folder_id, "partialData": false }]);
    let response = http.post(url).query(params).json(&body).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))
}

/// The parent a write on `item` should be verified against: its own folder if it has one,
/// the Drive root otherwise.
fn parent_of(item: &DriveItem) -> &str {
    item.parent_id
        .as_deref()
        .unwrap_or(quince_core::drive::ROOT)
}

/// Rename a file or folder in place.
///
/// The etag on `item` may already be stale by the time this runs — nothing enforces that a
/// caller re-synced first — and CloudKit-style services answer a stale etag by silently
/// keeping the old version while still returning 200. So this fetches the etag Apple
/// currently holds immediately before writing, and afterward re-reads the parent to confirm
/// the name actually changed rather than trusting the HTTP status alone.
pub async fn rename(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    name: &str,
) -> Result<()> {
    let name = name.trim();
    unusable_name(name)?;

    let parent = parent_of(item);
    let current = folder_listing(http, root, &params, parent).await?;
    let etag = current
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .and_then(|i| i.etag.clone())
        .or_else(|| item.etag.clone());

    let url = format!("{}/renameItems", root.trim_end_matches('/'));
    let body = json!({
        "items": [{ "drivewsid": item.drivewsid, "name": name, "etag": etag }],
    });

    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive refused the rename ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }

    let after = folder_listing(http, root, &params, parent).await?;
    // Compared composed: iCloud can hand a name back decomposed (NFD), "é" as "e" and an
    // accent (rclone/rclone#9314), and read as bytes every rename to an accented name — any
    // name typed in French, Vietnamese, Hindi with its marks — read as refused.
    let wanted = quince_core::fold::composed(name);
    let is_wanted = |n: &str| quince_core::fold::composed(n) == wanted;
    match after.iter().find(|i| i.drivewsid == item.drivewsid) {
        // Apple keeps a file's extension apart from its name: "final.pdf" comes back as
        // "final" and "pdf". Compared whole, every rename of a file with an extension read as
        // refused, and the old name was put back on screen.
        Some(found) if is_wanted(&found.name) || is_wanted(&found.display_name()) => Ok(()),
        Some(found) => Err(Error::Protocol(format!(
            "Apple answered success but the name is still {:?}, not {name:?}",
            found.name
        ))),
        None => Err(Error::Protocol(
            "Apple answered success but the item is no longer where it should be".into(),
        )),
    }
}

/// Move an item to Recently Deleted. Apple keeps it there for a time before it is really
/// gone, the same as a Mac's trash.
///
/// Same discipline as [`rename`]: a stale etag is refreshed before the write, and the
/// parent is re-read afterward to confirm the item is genuinely gone from it rather than
/// trusting a 200 that turned out, once, to have silently done nothing.
pub async fn delete(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    let parent = parent_of(item);
    let current = folder_listing(http, root, &params, parent).await?;
    let etag = current
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .and_then(|i| i.etag.clone())
        .or_else(|| item.etag.clone());

    // As icloud.com sends it (main.js, module 2231): the id, the etag, and the item's own id
    // as `clientId`; and the answer read per item — only "OK" is done. A folder still taking
    // uploads changes its etag with every file that lands, so the etag read a moment ago is
    // already old: Apple answered 200 and refused it inside, and the folder stayed where it
    // was. A stale etag is tried again with the one Apple sent back.
    let url = format!("{}/moveItemsToTrash", root.trim_end_matches('/'));
    let mut etag = etag;
    // Measured: deleting a folder while files went on landing in it met ETAG_CONFLICT on
    // three tries in a row — each file landing moved the etag again within the second. The
    // uploads into it are stopped first; the ones already being filed finish within seconds,
    // so the delete keeps trying, spaced out, with the etag Apple says it has now.
    let waits = [0u64, 400, 800, 1500, 2500, 3500];
    for (attempt, wait) in waits.iter().enumerate() {
        if *wait > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(*wait)).await;
        }
        let body = json!({
            "items": [{ "drivewsid": item.drivewsid, "etag": etag, "clientId": item.drivewsid }],
        });
        let reply = post_json(http, &url, &params, &body).await?;
        let answer = reply
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find(|i| {
                    i.get("drivewsid").and_then(Value::as_str) == Some(item.drivewsid.as_str())
                })
            })
            .cloned();
        match answer
            .as_ref()
            .and_then(|a| a.get("status"))
            .and_then(Value::as_str)
        {
            Some("OK") | None => break,
            Some("ETAG_CONFLICT") if attempt + 1 < waits.len() => {
                etag = answer
                    .as_ref()
                    .and_then(|a| a.get("etag"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(etag);
            }
            Some(other) => {
                return Err(Error::Refused(format!(
                    "iCloud Drive did not let that be deleted ({other})."
                )));
            }
        }
    }

    // A large folder takes Apple a moment to move: looked for a few times before it is said
    // to have stayed.
    for wait in [0u64, 800, 1500, 2500] {
        if wait > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
        let after = folder_listing(http, root, &params, parent).await?;
        if !after.iter().any(|i| i.drivewsid == item.drivewsid) {
            return Ok(());
        }
    }
    Err(Error::Refused(
        "iCloud answered, but the item is still in the folder — try again in a moment.".into(),
    ))
}

/// A folder as Apple holds it now, against how it was shown: asked about before it is deleted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderNow {
    /// Everything in it now.
    pub items: usize,
    /// Of those, the ones that were not there when it was shown.
    pub added: usize,
    /// Ones that were there, changed since.
    pub changed: usize,
    /// Ones shown that are no longer there.
    pub gone: usize,
    /// The version of the folder these numbers are of: deleted anyway, only that version is.
    pub etag: Option<String>,
}

/// What a delete from the window did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Removed {
    /// In Recently Deleted.
    Done,
    /// Not deleted: the folder is not what was shown. Asked again with this.
    Changed(FolderNow),
}

/// How the folder's items now stand against `shown`.
#[must_use]
pub fn folder_now(shown: &[DriveItem], now: &[DriveItem], etag: Option<String>) -> FolderNow {
    let was = |id: &str| shown.iter().find(|s| s.drivewsid == id);
    FolderNow {
        items: now.len(),
        added: now.iter().filter(|n| was(&n.drivewsid).is_none()).count(),
        changed: now
            .iter()
            .filter(|n| was(&n.drivewsid).is_some_and(|s| s.etag != n.etag))
            .count(),
        gone: shown
            .iter()
            .filter(|s| !now.iter().any(|n| n.drivewsid == s.drivewsid))
            .count(),
        etag,
    }
}

/// Delete from the window, only what was shown.
///
/// A file goes against the version shown (its stored etag): changed since — on another
/// device — Apple refuses it, and that is said rather than the newer file deleted. A folder
/// goes only if it is the version shown (`expect`, the version a question already showed, or
/// else its stored etag — every file landing in it moves its etag on): otherwise nothing is
/// deleted and its contents as they now stand come back, to be asked about. Taken from the
/// folder it is in, the latest etag deleted whatever had been added to it on another device.
///
/// `shown` is the folder's items as the window last held them.
pub async fn delete_as_shown(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    shown: &[DriveItem],
    expect: Option<&str>,
) -> Result<Removed> {
    let parent = parent_of(item);
    let listed = folder_listing(http, root, &params, parent).await?;
    let Some(current) = listed.iter().find(|i| i.drivewsid == item.drivewsid) else {
        // Gone already: deleted elsewhere, or by an earlier try of this one.
        return Ok(Removed::Done);
    };
    let changed_words = || {
        Error::Refused(format!(
            "“{}” changed on another device since it was shown here, so it wasn’t deleted. Take a look, then delete it again.",
            item.display_name()
        ))
    };
    let etag = if item.is_container() {
        let seen = expect.map(str::to_string).or_else(|| item.etag.clone());
        let unchanged = match (&seen, &current.etag) {
            (Some(seen), Some(now)) => seen == now,
            // No version to hold it to: its contents are compared instead.
            _ => {
                let now = folder_listing(http, root, &params, &item.drivewsid).await?;
                let n = folder_now(shown, &now, current.etag.clone());
                n.added == 0 && n.changed == 0 && n.gone == 0
            }
        };
        if !unchanged {
            let now = folder_listing(http, root, &params, &item.drivewsid).await?;
            return Ok(Removed::Changed(folder_now(
                shown,
                &now,
                current.etag.clone(),
            )));
        }
        current.etag.clone()
    } else {
        item.etag.clone().or_else(|| current.etag.clone())
    };

    let url = format!("{}/moveItemsToTrash", root.trim_end_matches('/'));
    let body = json!({
        "items": [{ "drivewsid": item.drivewsid, "etag": etag, "clientId": item.drivewsid }],
    });
    let reply = post_json(http, &url, &params, &body).await?;
    let status = reply
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|i| {
                i.get("drivewsid").and_then(Value::as_str) == Some(item.drivewsid.as_str())
            })
        })
        .and_then(|a| a.get("status"))
        .and_then(Value::as_str)
        .map(str::to_string);
    match status.as_deref() {
        Some("OK") | None => {}
        // Moved on between the look and the delete: a folder is asked about again as it
        // now stands; a file is not deleted.
        Some("ETAG_CONFLICT") if item.is_container() => {
            let listed = folder_listing(http, root, &params, parent).await?;
            let etag = listed
                .iter()
                .find(|i| i.drivewsid == item.drivewsid)
                .and_then(|i| i.etag.clone());
            let now = folder_listing(http, root, &params, &item.drivewsid).await?;
            return Ok(Removed::Changed(folder_now(shown, &now, etag)));
        }
        Some("ETAG_CONFLICT") => return Err(changed_words()),
        Some(other) => {
            return Err(Error::Refused(format!(
                "iCloud Drive did not let that be deleted ({other})."
            )))
        }
    }
    for wait in [0u64, 800, 1500, 2500] {
        if wait > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
        let after = folder_listing(http, root, &params, parent).await?;
        if !after.iter().any(|i| i.drivewsid == item.drivewsid) {
            return Ok(Removed::Done);
        }
    }
    Err(Error::Refused(
        "iCloud answered, but the item is still in the folder — try again in a moment.".into(),
    ))
}

/// Duplicate a file as icloud.com does: `POST {docws}/v1/item/copy/{item_id}` with
/// `{info_to_update:{}}`. Apple names the copy as Finder would — "Plan 2.pdf" — in the same
/// folder. `item_id` is not the drivewsid's tail (that answers 404): it is a field of the
/// item in its folder's listing, so the listing is read first. Measured on a real account.
pub async fn duplicate(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    ask_for_copy(http, docws_root, drivews_root, &params, item)
        .await
        .map(|_| ())
}

/// Asks Apple for a copy of `item` beside itself ([`duplicate`]); answers the folder as it was
/// just before, to tell the copy by.
async fn ask_for_copy(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: &[(String, String)],
    item: &DriveItem,
) -> Result<Vec<DriveItem>> {
    let listing = raw_listing(http, drivews_root, params, parent_of(item)).await?;
    let item_id = listing
        .pointer("/0/items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|i| {
                i.get("drivewsid").and_then(Value::as_str) == Some(item.drivewsid.as_str())
            })
        })
        .and_then(|i| i.get("item_id").and_then(Value::as_str))
        .ok_or_else(|| {
            Error::Refused("That item is no longer in its folder, so it was not duplicated.".into())
        })?
        .to_string();
    let before = parse_folder(&listing);
    let url = format!(
        "{}/v1/item/copy/{item_id}",
        docws_root.trim_end_matches('/')
    );
    let response = http
        .post(url)
        .query(params)
        .json(&json!({ "info_to_update": {} }))
        .send()
        .await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive refused the copy ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    Ok(before)
}

/// A copy of `item` made beside it, and found: what Apple answers the copy with says nothing
/// of it, so it is the item in the folder that was not there before, of the same kind and
/// extension — the one named after the original if another device added something meanwhile.
/// Looked for a few times, spaced out, as a write takes Apple a moment to list.
async fn copy_beside(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: &[(String, String)],
    item: &DriveItem,
) -> Result<DriveItem> {
    let before = ask_for_copy(http, docws_root, drivews_root, params, item).await?;
    let parent = parent_of(item);
    let stem = without_copy(&item.name).to_lowercase();
    for wait in [0u64, 400, 800, 1500, 2500, 4000] {
        if wait > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
        let mut new: Vec<DriveItem> = folder_listing(http, drivews_root, params, parent)
            .await?
            .into_iter()
            .filter(|i| {
                i.kind == item.kind
                    && i.extension == item.extension
                    && !before.iter().any(|b| b.drivewsid == i.drivewsid)
            })
            .collect();
        new.sort_by_key(|i| !i.name.to_lowercase().starts_with(&stem));
        if let Some(copy) = new.into_iter().next() {
            return Ok(DriveItem {
                parent_id: Some(parent.to_string()),
                ..copy
            });
        }
    }
    Err(Error::Refused(
        "iCloud answered, but the copy hasn’t shown up yet — try again in a moment.".into(),
    ))
}

/// Apple's own thumbnails for the files in one folder, as `(drivewsid, image bytes)`.
///
/// Measured on a real account: `POST {docws}/ws/com.apple.CloudDocs/download/batch` with
/// `[{document_id}]` answers each file with a `data_token`, and — for the kinds Apple renders
/// itself, Office and iWork documents among them — a `thumbnail_token.url` to fetch as-is.
/// PDFs come back without one; those keep their icon. One batch for the folder, then each
/// image; a file whose thumbnail fails is simply left out.
pub async fn thumbnails(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
    files: &[DriveItem],
) -> Result<Vec<(String, Vec<u8>)>> {
    let wanted: Vec<(&DriveItem, &str)> = files
        .iter()
        .filter(|f| f.kind == "FILE")
        .map(|f| (f, docwsid_of(&f.drivewsid)))
        .collect();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let body: Vec<Value> = wanted
        .iter()
        .map(|(_, d)| json!({ "document_id": d }))
        .collect();
    let url = format!(
        "{}/ws/com.apple.CloudDocs/download/batch",
        docws_root.trim_end_matches('/')
    );
    let response = http.post(url).query(&params).json(&body).send().await?;
    if response.status().as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!("thumbnails returned {status}")));
    }
    let answer: Value = response
        .json()
        .await
        .map_err(|e| Error::Protocol(format!("thumbnails: {e}")))?;
    let mut out = Vec::new();
    for entry in answer.as_array().into_iter().flatten() {
        let (Some(doc), Some(thumb)) = (
            entry.get("document_id").and_then(Value::as_str),
            entry
                .pointer("/thumbnail_token/url")
                .and_then(Value::as_str),
        ) else {
            continue;
        };
        let Some((item, _)) = wanted.iter().find(|(_, d)| d.eq_ignore_ascii_case(doc)) else {
            continue;
        };
        if let Ok(r) = http.get(thumb).send().await {
            if r.status().is_success() {
                if let Ok(bytes) = r.bytes().await {
                    out.push((item.drivewsid.clone(), bytes.to_vec()));
                }
            }
        }
    }
    Ok(out)
}

/// Drive's Recently Deleted, as icloud.com lists it: a folder Apple names `TRASH_ROOT`.
pub const TRASH: &str = "TRASH_ROOT";

/// One item in Recently Deleted: when Apple will delete it for good, and where Recover
/// puts it back — measured on a real account as `dateExpiration` and `restorePath`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    #[serde(flatten)]
    pub item: DriveItem,
    /// Unix seconds.
    pub expires_at: Option<i64>,
    pub restore_path: Option<String>,
}

/// A file in Recents, with when it was last opened.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentItem {
    #[serde(flatten)]
    pub item: DriveItem,
    /// Unix seconds.
    pub opened_at: Option<i64>,
}

/// The files opened most recently, newest first, as icloud.com's Recents lists them. Read
/// from the document service (`docws`), not the folder one. Read-only.
pub async fn recents(
    http: &reqwest::Client,
    docws_root: &str,
    params: Vec<(String, String)>,
) -> Result<Vec<RecentItem>> {
    let mut query = params;
    query.push(("limit".into(), "50".into()));
    query.push(("unified_format".into(), "true".into()));
    let url = format!(
        "{}/ws/com.apple.CloudDocs/list/recents",
        docws_root.trim_end_matches('/')
    );
    let response = http.get(url).query(&query).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))?;
    Ok(parse_recents(&parsed))
}

/// The documents of a `list/recents` answer, newest opened first.
#[must_use]
pub fn parse_recents(body: &Value) -> Vec<RecentItem> {
    let mut out: Vec<RecentItem> = body
        .get("documents")
        .and_then(Value::as_array)
        .map(|docs| {
            docs.iter()
                .filter_map(|d| {
                    Some(RecentItem {
                        item: item_from(d)?,
                        opened_at: seconds(d, "lastOpenTime"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|r| std::cmp::Reverse(r.opened_at));
    out
}

/// What is in Recently Deleted now. Read-only.
pub async fn trash(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Vec<DriveItem>> {
    Ok(trash_items(http, root, params)
        .await?
        .into_iter()
        .map(|t| t.item)
        .collect())
}

/// [`trash`], with each item's expiry and the folder it came from.
pub async fn trash_items(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Vec<TrashItem>> {
    let url = format!(
        "{}/retrieveItemDetailsInFolders",
        root.trim_end_matches('/')
    );
    let body = json!([{ "drivewsid": TRASH, "partialData": false }]);
    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))?;
    let raw: Vec<&Value> = parsed
        .pointer("/0/items")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    Ok(listing(&parsed)?
        .into_iter()
        .map(|item| {
            let found = raw.iter().find(|r| {
                r.get("drivewsid").and_then(Value::as_str) == Some(item.drivewsid.as_str())
            });
            let expires_at = found
                .and_then(|r| r.get("dateExpiration"))
                .and_then(Value::as_str)
                .and_then(|d| d.parse::<jiff::Timestamp>().ok())
                .map(|t| t.as_second());
            let restore_path = found
                .and_then(|r| r.get("restorePath"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            TrashItem {
                item,
                expires_at,
                restore_path,
            }
        })
        .collect())
}

/// Put items back where they were deleted from (`putBackItemsFromTrash`), or delete them for
/// good (`deleteItems`), as icloud.com's Recover and Delete do — all of them in one request,
/// as its Recover All and Delete All send them. One at a time and all at once, thirty items
/// were thirty writes and sixty reads of the trash against Apple together; it turned them
/// away as busy, and what was not deleted came back on the next read.
///
/// Apple's answer is per item, and only `"OK"` is done: a 200 can carry refusals inside it.
/// An etag gone stale is tried once more with the one Apple sent back, as icloud.com does.
/// The trash is read again afterwards and each item must have left it. Returns what could not
/// be done, each with the reason.
pub async fn out_of_trash_many(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    items: &[DriveItem],
    endpoint: &str,
    what: &str,
) -> Result<Vec<(String, String)>> {
    // Fifty at a time: hundreds in one request — a Recently Deleted filled by a folder upload
    // — was too large for the connection, and nothing was deleted at all. A dropped connection
    // is tried once more before the rest is reported.
    // Recently Deleted is read once before every batch and once after them all — not twice a
    // batch: a large one takes seconds to read, and sixty items took most of a minute and a
    // half that way.
    let current = folder_listing(http, root, &params, TRASH).await?;
    let mut failed = Vec::new();
    for chunk in items.chunks(50) {
        let mut outcome =
            out_of_trash_batch(http, root, params.clone(), chunk, endpoint, what, &current).await;
        if matches!(outcome, Err(Error::Network(_))) {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            outcome =
                out_of_trash_batch(http, root, params.clone(), chunk, endpoint, what, &current)
                    .await;
        }
        match outcome {
            Ok(f) => failed.extend(f),
            Err(e) => failed.extend(
                chunk
                    .iter()
                    .map(|item| (item.drivewsid.clone(), e.to_string())),
            ),
        }
    }
    let after = folder_listing(http, root, &params, TRASH).await?;
    for item in items {
        if after.iter().any(|i| i.drivewsid == item.drivewsid)
            && !failed.iter().any(|(id, _)| id == &item.drivewsid)
        {
            failed.push((
                item.drivewsid.clone(),
                format!("iCloud answered, but the item is still in Recently Deleted, so it was not {what}."),
            ));
        }
    }
    Ok(failed)
}

async fn out_of_trash_batch(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    items: &[DriveItem],
    endpoint: &str,
    what: &str,
    current: &[DriveItem],
) -> Result<Vec<(String, String)>> {
    let mut failed: Vec<(String, String)> = Vec::new();
    // Each with the etag Apple has for it now.
    let mut todo: Vec<(DriveItem, Option<String>)> = Vec::new();
    for item in items {
        match current.iter().find(|i| i.drivewsid == item.drivewsid) {
            Some(found) => todo.push((
                item.clone(),
                found.etag.clone().or_else(|| item.etag.clone()),
            )),
            None => failed.push((
                item.drivewsid.clone(),
                format!("That item is no longer in Recently Deleted, so it could not be {what}."),
            )),
        }
    }
    let url = format!("{}/{endpoint}", root.trim_end_matches('/'));
    for attempt in 0..2 {
        if todo.is_empty() {
            break;
        }
        // `deleteItems` takes each item's own id as `clientId` too (main.js, module 2231).
        let entries: Vec<Value> = todo
            .iter()
            .map(|(item, etag)| {
                if endpoint == "deleteItems" {
                    json!({ "drivewsid": item.drivewsid, "etag": etag, "clientId": item.drivewsid })
                } else {
                    json!({ "drivewsid": item.drivewsid, "etag": etag })
                }
            })
            .collect();
        let reply = post_json(http, &url, &params, &json!({ "items": entries })).await?;
        let answers: Vec<Value> = reply
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut again = Vec::new();
        for (item, etag) in todo {
            let answer = answers.iter().find(|a| {
                a.get("drivewsid").and_then(Value::as_str) == Some(item.drivewsid.as_str())
            });
            match answer.and_then(|a| a.get("status")).and_then(Value::as_str) {
                Some("OK") | None => {}
                Some("ETAG_CONFLICT") if attempt == 0 => {
                    let fresh = answer
                        .and_then(|a| a.get("etag"))
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    again.push((item, fresh.or(etag)));
                }
                Some(other) => failed.push((
                    item.drivewsid.clone(),
                    format!("iCloud Drive did not let that be {what} ({other})."),
                )),
            }
        }
        todo = again;
    }
    for (item, _) in todo {
        failed.push((
            item.drivewsid,
            format!("That item changed in iCloud Drive, so it was not {what}."),
        ));
    }
    Ok(failed)
}

/// [`out_of_trash_many`] for one item, its refusal as the error.
async fn out_of_trash(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    endpoint: &str,
    what: &str,
) -> Result<()> {
    let failed = out_of_trash_many(
        http,
        root,
        params,
        std::slice::from_ref(item),
        endpoint,
        what,
    )
    .await?;
    match failed.into_iter().next() {
        None => Ok(()),
        Some((_, why)) => Err(Error::Refused(why)),
    }
}

/// Recover all of these, in one request. What could not be, with why.
pub async fn put_back_many(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    items: &[DriveItem],
) -> Result<Vec<(String, String)>> {
    out_of_trash_many(
        http,
        root,
        params,
        items,
        "putBackItemsFromTrash",
        "recovered",
    )
    .await
}

/// Delete all of these for good, in one request. What could not be, with why.
pub async fn delete_forever_many(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    items: &[DriveItem],
) -> Result<Vec<(String, String)>> {
    out_of_trash_many(http, root, params, items, "deleteItems", "deleted").await
}

/// Recover: back to the folder it was deleted from.
pub async fn put_back(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    out_of_trash(
        http,
        root,
        params,
        item,
        "putBackItemsFromTrash",
        "recovered",
    )
    .await
}

/// Delete Now: gone for good. Only ever from Recently Deleted, and only when asked.
pub async fn delete_forever(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
) -> Result<()> {
    out_of_trash(http, root, params, item, "deleteItems", "deleted").await
}

/// Move an item into a different folder.
///
/// Verified the same way: a fresh etag before the write, and the destination re-read
/// afterward to confirm the item actually arrived there.
pub async fn move_to(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    destination_id: &str,
) -> Result<()> {
    let parent = parent_of(item);
    let current = folder_listing(http, root, &params, parent).await?;
    let etag = current
        .iter()
        .find(|i| i.drivewsid == item.drivewsid)
        .and_then(|i| i.etag.clone())
        .or_else(|| item.etag.clone());

    // Unlike rename and delete, moveItems 400s outright without a clientId on the item —
    // measured against the real service, not documented anywhere.
    let url = format!("{}/moveItems", root.trim_end_matches('/'));
    let body = json!({
        "items": [{
            "drivewsid": item.drivewsid,
            "etag": etag,
            "clientId": uuid::Uuid::new_v4().to_string(),
        }],
        "destinationDrivewsId": destination_id,
    });

    let response = http.post(url).query(&params).json(&body).send().await?;
    let status = response.status();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Drive refused the move ({status}): {}",
            text.chars().take(200).collect::<String>()
        )));
    }

    let arrived = folder_listing(http, root, &params, destination_id).await?;
    if !arrived.iter().any(|i| i.drivewsid == item.drivewsid) {
        return Err(Error::Protocol(
            "Apple answered success but the item never arrived at the destination".into(),
        ));
    }
    Ok(())
}

/// The name Finder gives a copy put into a folder that already holds `taken`: its own name
/// where that is free, else "Report copy.pdf", then "Report copy 2.pdf", "Report copy 3.pdf".
/// A copy of a copy is not "Report copy copy.pdf": the " copy" or " copy 2" it ends with is
/// counted on from, as Finder counts. Names are compared without regard to case, as iCloud
/// Drive compares them. `extension` is Apple's own, kept apart from the name; a folder has none.
#[must_use]
pub fn copy_name(stem: &str, extension: Option<&str>, taken: &[String]) -> String {
    let ext = extension
        .filter(|e| !e.is_empty())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let key = quince_core::fold::name_key;
    let taken: std::collections::HashSet<String> = taken.iter().map(|t| key(t)).collect();
    let free = |name: &String| !taken.contains(&key(name));
    let whole = format!("{stem}{ext}");
    if free(&whole) {
        return whole;
    }
    let base = without_copy(stem);
    std::iter::once(format!("{base} copy{ext}"))
        .chain((2u32..).map(|n| format!("{base} copy {n}{ext}")))
        .find(free)
        .unwrap_or(whole)
}

/// "Report copy" and "Report copy 3" as "Report": the name a copy's number counts from.
fn without_copy(stem: &str) -> &str {
    if let Some(rest) = stem.strip_suffix(" copy").filter(|r| !r.is_empty()) {
        return rest;
    }
    match stem.rsplit_once(" copy ") {
        Some((rest, n))
            if !rest.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) =>
        {
            rest
        }
        _ => stem,
    }
}

/// Copy a file or folder into another folder, as Finder's Copy and then Paste: the copy keeps
/// the original's name where that is free there and is named as Finder names a copy where it
/// is not ([`copy_name`]). Answers the copy, where it now is.
///
/// iCloud Drive's web service copies only beside the original (`v1/item/copy`, [`duplicate`]),
/// so a file is copied there, given a name free in both folders, moved, and named last: never
/// two things of one name in a folder at any step. A folder is made anew at the destination
/// and what is in it copied the same way, folder by folder. Into its own folder it is a
/// duplicate named as Finder names one ("Report copy.pdf"). Anything that fails part-way sends
/// what it made to Recently Deleted again, so a failed paste leaves no half copy behind.
pub async fn copy_to(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    destination_id: &str,
) -> Result<DriveItem> {
    match item.kind.as_str() {
        "FILE" => {
            copy_file_to(
                http,
                docws_root,
                drivews_root,
                &params,
                item,
                destination_id,
            )
            .await
        }
        "FOLDER" => {
            copy_folder_to(
                http,
                docws_root,
                drivews_root,
                &params,
                item,
                destination_id,
            )
            .await
        }
        _ => Err(Error::Refused(format!(
            "“{}” belongs to an app and can’t be copied.",
            item.display_name()
        ))),
    }
}

async fn copy_folder_to(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: &[(String, String)],
    folder: &DriveItem,
    destination_id: &str,
) -> Result<DriveItem> {
    if folder.drivewsid == destination_id {
        return Err(Error::Refused(format!(
            "“{}” can’t be copied into itself.",
            folder.name
        )));
    }
    let taken = names_in(
        &folder_listing(http, drivews_root, params, destination_id).await?,
        None,
    );
    let name = copy_name(&folder.name, None, &taken);
    let mut made =
        create_folder(http, drivews_root, params.to_vec(), destination_id, &name).await?;
    made.parent_id = Some(destination_id.to_string());
    // Each folder of the original beside the one made for it, walked a level at a time. What
    // was made here is never walked into: a folder pasted into itself would go on for ever.
    let mut ours = vec![made.drivewsid.clone()];
    let mut to_fill = vec![(folder.drivewsid.clone(), made.drivewsid.clone())];
    let filled: Result<()> = async {
        while let Some((from, into)) = to_fill.pop() {
            for child in folder_listing(http, drivews_root, params, &from).await? {
                if ours.contains(&child.drivewsid) {
                    continue;
                }
                let child = DriveItem {
                    parent_id: Some(from.clone()),
                    ..child
                };
                match child.kind.as_str() {
                    "FOLDER" => {
                        let sub =
                            create_folder(http, drivews_root, params.to_vec(), &into, &child.name)
                                .await?;
                        ours.push(sub.drivewsid.clone());
                        to_fill.push((child.drivewsid, sub.drivewsid));
                    }
                    "FILE" => {
                        copy_file_to(http, docws_root, drivews_root, params, &child, &into).await?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
    .await;
    match filled {
        Ok(()) => Ok(made),
        Err(e) => {
            delete(http, drivews_root, params.to_vec(), &made)
                .await
                .ok();
            Err(e)
        }
    }
}

async fn copy_file_to(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: &[(String, String)],
    file: &DriveItem,
    destination_id: &str,
) -> Result<DriveItem> {
    let source = parent_of(file).to_string();
    let mut copy = copy_beside(http, docws_root, drivews_root, params, file).await?;
    let placed: Result<String> = async {
        let ext = file.extension.as_deref();
        let source_names = names_in(
            &folder_listing(http, drivews_root, params, &source).await?,
            Some(&copy.drivewsid),
        );
        if source == destination_id {
            // A duplicate, named as Finder names one: Apple's "Report 2.pdf" is "Report copy.pdf".
            let want = copy_name(&file.name, ext, &source_names);
            if copy.display_name() != want {
                rename(http, drivews_root, params.to_vec(), &copy, &want).await?;
            }
            return Ok(want);
        }
        let dest_names = names_in(
            &folder_listing(http, drivews_root, params, destination_id).await?,
            None,
        );
        let want = copy_name(&file.name, ext, &dest_names);
        let free_in = |names: &[String], name: &str| {
            let name = quince_core::fold::name_key(name);
            !names.iter().any(|n| quince_core::fold::name_key(n) == name)
        };
        // The name it travels under: free in the folder it leaves and in the one it goes to.
        let travel = if free_in(&source_names, &want) {
            want.clone()
        } else if free_in(&dest_names, &copy.display_name()) {
            copy.display_name()
        } else {
            copy_name(&file.name, ext, &[source_names, dest_names].concat())
        };
        if copy.display_name() != travel {
            rename(http, drivews_root, params.to_vec(), &copy, &travel).await?;
        }
        move_to(http, drivews_root, params.to_vec(), &copy, destination_id).await?;
        copy.parent_id = Some(destination_id.to_string());
        if travel != want {
            rename(http, drivews_root, params.to_vec(), &copy, &want).await?;
        }
        Ok(want)
    }
    .await;
    match placed {
        Ok(named) => {
            let ext = copy.extension.as_deref().map(|e| format!(".{e}"));
            copy.name = match ext {
                Some(ext) if named.ends_with(&ext) => named[..named.len() - ext.len()].to_string(),
                _ => named,
            };
            Ok(copy)
        }
        Err(e) => {
            delete(http, drivews_root, params.to_vec(), &copy)
                .await
                .ok();
            Err(e)
        }
    }
}

/// The names in a folder as a person reads them, all but `except`'s.
fn names_in(items: &[DriveItem], except: Option<&str>) -> Vec<String> {
    items
        .iter()
        .filter(|i| Some(i.drivewsid.as_str()) != except)
        .map(DriveItem::display_name)
        .collect()
}

#[cfg(test)]
mod checksum_tests {
    use super::*;
    use serde_json::json;

    fn pattern(size: usize) -> Vec<u8> {
        (0..size)
            .map(|i| u8::try_from(i * 31 % 251).unwrap())
            .collect()
    }

    /// Apple's own answers for these exact bytes, measured on real uploads.
    #[test]
    fn the_checksum_is_apples() {
        for (size, apple) in [
            (37, "AcbV5Q1Z72Y4XYc0UObScQwypTml"),
            (3 * 1024 * 1024, "AbnSbKOQ813hyTYHFLl6Dh/oYsl2"),
        ] {
            let bytes = pattern(size);
            let mut whole = AppleChecksum::default();
            whole.update(&bytes);
            assert_eq!(whole.finish(), apple, "{size} bytes");
            // Fed in pieces as a stream feeds it, the answer is the same.
            let mut pieces = AppleChecksum::default();
            for chunk in bytes.chunks(256 * 1024 + 7) {
                pieces.update(chunk);
            }
            assert_eq!(pieces.finish(), apple, "{size} bytes in pieces");
        }
    }

    fn filed(signature: &str, size: u64, code: i64) -> Value {
        json!({ "results": [{ "document": {
            "document_id": "06502863-6910-440B-A1B9-29536C125463",
            "name": "photo 2.jpg", "size": size,
            "data": { "signature": signature },
            "status": { "status_code": code, "error_message": "OK" }
        }}]})
    }

    /// Found by the id Apple gave it, whatever it was renamed to.
    #[test]
    fn a_filed_document_is_accepted_only_when_it_is_the_file_sent() {
        let sig = "AcbV5Q1Z72Y4XYc0UObScQwypTml";
        assert_eq!(
            filed_document(&filed(sig, 37, 0), sig, 37, "photo.jpg", DOCUMENTS_ZONE).unwrap(),
            "FILE::com.apple.CloudDocs::06502863-6910-440B-A1B9-29536C125463"
        );
        assert!(filed_document(
            &filed("Aother", 37, 0),
            sig,
            37,
            "photo.jpg",
            DOCUMENTS_ZONE
        )
        .is_err());
        assert!(filed_document(&filed(sig, 36, 0), sig, 37, "photo.jpg", DOCUMENTS_ZONE).is_err());
        assert!(filed_document(&filed(sig, 37, 1), sig, 37, "photo.jpg", DOCUMENTS_ZONE).is_err());
        assert!(filed_document(&json!({}), sig, 37, "photo.jpg", DOCUMENTS_ZONE).is_err());
    }
}

#[cfg(test)]
mod created_folder_tests {
    use super::*;
    use serde_json::json;

    /// The two replies Apple gives, both measured: a made folder, and a second create of the
    /// same name.
    #[test]
    fn a_declined_folder_is_reported_as_declined() {
        let made = json!({ "folders": [{ "drivewsid": "FOLDER::com.apple.CloudDocs::A",
            "name": "Wallp", "type": "FOLDER", "status": "OK", "etag": "2y8" }] });
        assert_eq!(created_folder(&made, "Wallp").unwrap().name, "Wallp");

        let declined = json!({ "folders": [{ "clientId": "7222bf20", "status": "UNKNOWN" }] });
        let message = created_folder(&declined, "Wallp").unwrap_err().to_string();
        assert!(message.contains("didn't create"), "{message}");
        assert!(!message.contains("created the folder"), "{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names icloud.com refuses are refused here too, before anything is sent.
    #[test]
    fn hidden_empty_and_colon_names_are_refused() {
        assert!(unusable_name("").is_err());
        assert!(unusable_name(".env").is_err());
        assert!(unusable_name("a:b").is_err());
        assert!(unusable_name("two\nlines").is_err());
        assert!(unusable_name("Semester Plan.pdf").is_ok());
        assert!(unusable_name("v1.2 notes").is_ok());
    }

    fn response() -> Value {
        json!([{
            "drivewsid": "FOLDER::com.apple.CloudDocs::root",
            "items": [
                { "drivewsid": "FOLDER::com.apple.CloudDocs::abc", "parentId": "FOLDER::com.apple.CloudDocs::root",
                  "name": "Downloads", "type": "FOLDER", "directChildrenCount": 3,
                  "dateCreated": "2026-08-01T10:00:00Z", "etag": "e1", "size": -1 },
                { "drivewsid": "FILE::com.apple.CloudDocs::def", "parentId": "FOLDER::com.apple.CloudDocs::root",
                  "name": "report", "extension": "pdf", "type": "FILE", "size": 20480,
                  "dateModified": 1757000000000_i64, "etag": "e2" },
                { "drivewsid": "APPLIBRARY::com.apple.Keynote::x", "name": "Keynote",
                  "type": "APP_LIBRARY", "size": -1 }
            ]
        }])
    }

    #[test]
    fn a_folder_listing_is_read() {
        let items = parse_folder(&response());
        assert_eq!(items.len(), 3);

        let folder = &items[0];
        assert_eq!(folder.name, "Downloads");
        assert!(folder.is_container());
        assert_eq!(folder.child_count, Some(3));
        assert!(folder.size.is_none(), "-1 is not a size");
        assert!(folder.created_at.is_some(), "an ISO date should parse");

        let file = &items[1];
        assert_eq!(file.display_name(), "report.pdf");
        assert_eq!(file.size, Some(20480));
        assert_eq!(
            file.modified_at,
            Some(1_757_000_000),
            "milliseconds become seconds"
        );

        assert!(
            items[2].is_container(),
            "an app library opens like a folder"
        );
    }

    #[test]
    fn an_item_without_an_identifier_is_skipped() {
        let body = json!([{ "items": [{ "name": "ghost", "type": "FILE" }] }]);
        assert!(parse_folder(&body).is_empty());
    }

    #[test]
    fn an_empty_or_odd_response_is_an_empty_folder() {
        assert!(parse_folder(&json!([])).is_empty());
        assert!(parse_folder(&json!({})).is_empty());
        assert!(parse_folder(&json!([{ "drivewsid": "x" }])).is_empty());
    }

    #[tokio::test]
    async fn syncing_a_folder_stores_its_contents() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/retrieveItemDetailsInFolders$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response()))
            .mount(&server)
            .await;

        let mut library = quince_core::Library::in_memory().unwrap();
        let stored = sync_folder(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &mut library,
            quince_core::drive::ROOT,
        )
        .await
        .unwrap();

        assert_eq!(stored, 3);
        let children = library.drive_children(quince_core::drive::ROOT).unwrap();
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].name, "Downloads", "folders sort first");
    }

    /// A folder deleted elsewhere answers like an empty one but for its status. Read as empty,
    /// it would wipe the folder this computer holds; it must fail instead.
    #[tokio::test]
    async fn a_folder_apple_no_longer_has_never_empties_the_stored_one() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mut library = quince_core::Library::in_memory().unwrap();
        let root = quince_core::drive::ROOT;
        library
            .put_drive_children(root, &parse_folder(&response()))
            .unwrap();

        for (answer, gone) in [
            (
                json!([{ "items": [], "numberOfItems": 0, "status": "ID_INVALID" }]),
                true,
            ),
            (json!([{ "drivewsid": root, "status": "OK" }]), false),
            (json!([]), false),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/retrieveItemDetailsInFolders$"))
                .respond_with(ResponseTemplate::new(200).set_body_json(answer))
                .mount(&server)
                .await;
            let result = sync_folder(
                &reqwest::Client::new(),
                &server.uri(),
                vec![],
                &mut library,
                root,
            )
            .await;
            assert_eq!(matches!(result, Err(Error::NotFound)), gone, "{result:?}");
            assert!(result.is_err());
            assert_eq!(
                library.drive_children(root).unwrap().len(),
                3,
                "the stored folder is kept"
            );
        }

        // A folder that really is empty still reads as empty.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/retrieveItemDetailsInFolders$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{ "drivewsid": root, "status": "OK", "items": [] }])),
            )
            .mount(&server)
            .await;
        assert_eq!(
            sync_folder(
                &reqwest::Client::new(),
                &server.uri(),
                vec![],
                &mut library,
                root
            )
            .await
            .unwrap(),
            0
        );
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn file() -> DriveItem {
        DriveItem {
            drivewsid: "FILE::com.apple.CloudDocs::ABC-123".into(),
            parent_id: Some("FOLDER::com.apple.CloudDocs::root".into()),
            name: "report".into(),
            extension: Some("pdf".into()),
            kind: "FILE".into(),
            size: Some(12),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: Some("e1".into()),
        }
    }

    async fn sent(server: &MockServer, matching: &str) -> Value {
        let request = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.url.path().ends_with(matching))
            .unwrap_or_else(|| panic!("no request to {matching}"));
        serde_json::from_slice(&request.body).unwrap()
    }

    #[test]
    fn a_bare_docwsid_is_the_last_segment() {
        assert_eq!(docwsid_of("FILE::com.apple.CloudDocs::ABC-123"), "ABC-123");
        assert_eq!(docwsid_of("no-separator"), "no-separator");
    }

    #[tokio::test]
    async fn a_download_url_comes_from_the_docws_host_not_drivews() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": "https://cvws.icloud-content.com/x" }
            })))
            .mount(&server)
            .await;

        let url = download_url(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap();
        assert_eq!(url, "https://cvws.icloud-content.com/x");

        let request: Request = server.received_requests().await.unwrap().remove(0);
        let query: std::collections::HashMap<_, _> = request.url.query_pairs().collect();
        assert_eq!(
            query["document_id"], "ABC-123",
            "the bare id, not the full drivewsid"
        );
    }

    #[tokio::test]
    async fn a_reply_with_no_url_is_a_clear_error_not_a_panic() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        assert!(
            download_url(&reqwest::Client::new(), &server.uri(), vec![], &file())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_empty_folder_name_is_refused_before_anything_is_sent() {
        let server = MockServer::start().await;
        assert!(create_folder(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            "   ",
        )
        .await
        .is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_created_folder_is_returned_as_a_real_item() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/createFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "folders": [{
                    "drivewsid": "FOLDER::com.apple.CloudDocs::NEW",
                    "parentId": "FOLDER::com.apple.CloudDocs::root",
                    "name": "Scans",
                    "type": "FOLDER",
                    "etag": "e9"
                }]
            })))
            .mount(&server)
            .await;

        let created = create_folder(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            "Scans",
        )
        .await
        .unwrap();
        assert_eq!(created.name, "Scans");
        assert_eq!(created.kind, "FOLDER");

        let body = sent(&server, "/createFolders").await;
        assert_eq!(
            body["destinationDrivewsId"],
            "FOLDER::com.apple.CloudDocs::root"
        );
        assert_eq!(body["folders"][0]["name"], "Scans");
    }

    /// A folder made with its siblings in one request answers under the name it was asked by:
    /// "Notes " (a space at the end, as folders copied from a Mac often have) was made as
    /// "Notes" and answered as "Notes", so the upload never found it, and every file in it
    /// failed against the folder it had just made.
    #[tokio::test]
    async fn folders_made_together_answer_under_the_names_asked() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/createFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "folders": [{
                    "drivewsid": "FOLDER::com.apple.CloudDocs::N1",
                    "parentId": "FOLDER::com.apple.CloudDocs::root",
                    "name": "Notes",
                    "type": "FOLDER",
                    "status": "OK"
                }]
            })))
            .mount(&server)
            .await;
        let made = create_folders(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            &["Notes ".to_string()],
        )
        .await
        .unwrap();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].0, "Notes ");
        assert_eq!(made[0].1.as_deref(), Ok("FOLDER::com.apple.CloudDocs::N1"));
        assert_eq!(
            sent(&server, "/createFolders").await["folders"][0]["name"],
            "Notes"
        );
    }

    /// A response shaped like a real `retrieveItemDetailsInFolders` reply, holding `item`
    /// (possibly under a different, current etag) as the sole child of its folder.
    fn listing_of(folder_id: &str, item: &DriveItem, etag: &str) -> serde_json::Value {
        serde_json::json!([{
            "drivewsid": folder_id,
            "items": [{
                "drivewsid": item.drivewsid,
                "parentId": folder_id,
                "name": item.name,
                "extension": item.extension,
                "type": item.kind,
                "etag": etag,
            }]
        }])
    }

    fn empty_listing_of(folder_id: &str) -> serde_json::Value {
        serde_json::json!([{ "drivewsid": folder_id, "items": [] }])
    }

    #[tokio::test]
    async fn renaming_uses_the_etag_apple_currently_holds_not_a_stale_one() {
        let server = MockServer::start().await;
        // The caller's copy says "e1"; Apple has moved on to "e2" since — a rename sent
        // with "e1" is exactly the write that used to silently do nothing.
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e2",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/renameItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        // The confirming re-read after the write: the rename really landed.
        let mut renamed = file();
        renamed.name = "final report".into();
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &renamed,
                "e3",
            )))
            .mount(&server)
            .await;

        rename(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "final report",
        )
        .await
        .unwrap();

        let body = sent(&server, "/renameItems").await;
        assert_eq!(body["items"][0]["name"], "final report");
        assert_eq!(
            body["items"][0]["etag"], "e2",
            "the fresh etag, not the stale e1"
        );
    }

    /// The whole point of this rewrite: a 200 that did not actually change anything must
    /// surface as a failure, not as success.
    #[tokio::test]
    async fn a_rename_that_silently_did_nothing_is_reported_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/renameItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;

        let err = rename(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "final report",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("still"), "{err}");
    }

    /// A name iCloud hands back decomposed (NFD) is the name asked for, composed (NFC): the
    /// rename is done, not "Apple answered success but the name is still …" (rclone#9314).
    #[tokio::test]
    async fn a_rename_read_back_decomposed_is_done() {
        let server = MockServer::start().await;
        let mut renamed = file();
        renamed.name = "Re\u{301}sume\u{301}".into();
        renamed.extension = Some("pdf".into());
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &renamed,
                "e2",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/renameItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        rename(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "R\u{e9}sum\u{e9}.pdf",
        )
        .await
        .unwrap();
    }

    /// A copy's name is free only if no name there is the same once composed and cased alike:
    /// "Résumé.pdf" written decomposed by a Mac is taken for one typed here.
    #[test]
    fn a_decomposed_name_is_the_same_name() {
        assert_eq!(
            copy_name(
                "R\u{e9}sum\u{e9}",
                Some("pdf"),
                &["RE\u{301}SUME\u{301}.PDF".into()]
            ),
            "R\u{e9}sum\u{e9} copy.pdf"
        );
    }

    /// Apple keeps the extension apart from the name: a rename to "final.pdf" reads back as
    /// "final" + "pdf", which is the rename done — not "the name is still final".
    #[tokio::test]
    async fn a_rename_with_an_extension_reads_back_as_done() {
        let server = MockServer::start().await;
        let mut renamed = file();
        renamed.name = "final".into();
        renamed.extension = Some("pdf".into());
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &renamed,
                "e2",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/renameItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        rename(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "final.pdf",
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn deleting_moves_to_trash_rather_than_destroying() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItemsToTrash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        // Confirmed gone: the parent no longer lists it.
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(empty_listing_of("FOLDER::com.apple.CloudDocs::root")),
            )
            .mount(&server)
            .await;

        delete(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap();

        let body = sent(&server, "/moveItemsToTrash").await;
        assert_eq!(
            body["items"][0]["drivewsid"],
            "FILE::com.apple.CloudDocs::ABC-123"
        );
    }

    /// Recover sends the item, with the etag Recently Deleted holds now, and is only a success
    /// once the item has left Recently Deleted.
    #[tokio::test]
    async fn recovering_puts_back_and_checks_it_left_the_trash() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                TRASH,
                &file(),
                "fresh",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/putBackItemsFromTrash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_listing_of(TRASH)))
            .mount(&server)
            .await;

        put_back(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap();
        let body = sent(&server, "/putBackItemsFromTrash").await;
        assert_eq!(
            body["items"][0]["drivewsid"],
            "FILE::com.apple.CloudDocs::ABC-123"
        );
        assert_eq!(body["items"][0]["etag"], "fresh");
    }

    /// Delete All is one request with every item, each carrying its own id as `clientId`, as
    /// icloud.com sends it; a stale etag is tried again with Apple's; a refusal inside the 200
    /// is reported for its item alone.
    #[tokio::test]
    async fn delete_all_is_one_request_and_reads_each_answer() {
        let server = MockServer::start().await;
        let item = |id: &str| DriveItem {
            drivewsid: format!("FILE::com.apple.CloudDocs::{id}"),
            ..file()
        };
        let (a, b, c) = (item("A"), item("B"), item("C"));
        let trash = serde_json::json!([{ "drivewsid": TRASH, "status": "OK", "items": [
            { "drivewsid": a.drivewsid, "name": "a", "type": "FILE", "etag": "ea" },
            { "drivewsid": b.drivewsid, "name": "b", "type": "FILE", "etag": "eb" },
            { "drivewsid": c.drivewsid, "name": "c", "type": "FILE", "etag": "ec" },
        ]}]);
        // Read before: all three. Read after: only the refused one is left.
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(trash))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!([{ "drivewsid": TRASH, "status": "OK", "items": [
                    { "drivewsid": c.drivewsid, "name": "c", "type": "FILE", "etag": "ec" },
                ]}]),
            ))
            .mount(&server)
            .await;
        // First: A done, B's etag stale, C refused. Then B again with the fresh etag: done.
        Mock::given(method("POST"))
            .and(path("/deleteItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [
                    { "drivewsid": a.drivewsid, "status": "OK" },
                    { "drivewsid": b.drivewsid, "status": "ETAG_CONFLICT", "etag": "eb2" },
                    { "drivewsid": c.drivewsid, "status": "UNKNOWN_ERROR" },
                ]})),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/deleteItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [
                    { "drivewsid": b.drivewsid, "status": "OK" },
                ]})),
            )
            .mount(&server)
            .await;
        let failed = delete_forever_many(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &[a.clone(), b.clone(), c.clone()],
        )
        .await
        .unwrap();
        let deletes: Vec<serde_json::Value> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/deleteItems"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert_eq!(
            deletes.len(),
            2,
            "one request for all, one more for the stale etag"
        );
        assert_eq!(deletes[0]["items"].as_array().unwrap().len(), 3);
        assert_eq!(deletes[0]["items"][0]["clientId"], a.drivewsid);
        assert_eq!(deletes[1]["items"][0]["etag"], "eb2");
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert_eq!(failed[0].0, c.drivewsid);
    }

    /// Delete Now that left the item in Recently Deleted is a failure, said so.
    #[tokio::test]
    async fn a_delete_now_that_did_nothing_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                TRASH,
                &file(),
                "e1",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/deleteItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        let err = delete_forever(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("still in Recently Deleted"),
            "{err}"
        );
    }

    /// This is the bug measured against the real account: `moveItemsToTrash` answered 200
    /// while the item was still sitting in the folder, because the etag sent was stale.
    #[tokio::test]
    async fn a_delete_that_left_the_item_in_place_is_reported_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItemsToTrash"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;

        let err = delete(&reqwest::Client::new(), &server.uri(), vec![], &file())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("still in the folder"), "{err}");
    }

    #[tokio::test]
    async fn moving_names_the_destination_folder() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::root",
                &file(),
                "e1",
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;
        // Confirmed arrived: the destination now lists it.
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(listing_of(
                "FOLDER::com.apple.CloudDocs::DEST",
                &file(),
                "e2",
            )))
            .mount(&server)
            .await;

        move_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "FOLDER::com.apple.CloudDocs::DEST",
        )
        .await
        .unwrap();

        let body = sent(&server, "/moveItems").await;
        assert_eq!(
            body["destinationDrivewsId"],
            "FOLDER::com.apple.CloudDocs::DEST"
        );
        // Measured against the real service: moveItems 400s outright with no clientId on
        // the item, unlike rename or delete, which need none.
        assert!(
            body["items"][0]["clientId"]
                .as_str()
                .is_some_and(|c| !c.is_empty()),
            "moveItems requires a clientId per item"
        );
    }

    #[tokio::test]
    async fn a_move_that_never_arrived_is_reported_as_a_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetailsInFolders"))
            .respond_with(ResponseTemplate::new(200).set_body_json(empty_listing_of("x")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/moveItems"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"items": []})),
            )
            .mount(&server)
            .await;

        let err = move_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            "FOLDER::com.apple.CloudDocs::DEST",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("never arrived"), "{err}");
    }

    /// A 450 is Apple asking for a fresh sign-in — the same rule every write path in this
    /// app follows, so it must not be reported as a plain protocol error.
    #[tokio::test]
    async fn a_450_anywhere_means_reauth_not_a_generic_failure() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*"))
            .respond_with(ResponseTemplate::new(450))
            .mount(&server)
            .await;
        assert!(matches!(
            rename(&reqwest::Client::new(), &server.uri(), vec![], &file(), "x").await,
            Err(Error::ReauthRequired)
        ));
    }

    #[tokio::test]
    async fn downloading_to_disk_writes_the_exact_bytes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": format!("{}/bytes", server.uri()) }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bytes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello drive".to_vec()))
            .mount(&server)
            .await;

        let dir = tempdir();
        let destination = dir.join("report.pdf");
        download_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            &destination,
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"hello drive");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A download cut off by a crash or a power cut leaves its partial file behind; the next
    /// download of that file must not trip over it — it failed with "File exists" for good.
    #[tokio::test]
    async fn a_partial_file_left_by_a_crash_does_not_block_the_next_download() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": format!("{}/bytes", server.uri()) }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bytes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"whole film".to_vec()))
            .mount(&server)
            .await;

        let dir = tempdir();
        std::fs::write(dir.join("Film.mov.part"), b"half a fi").unwrap();
        let destination = dir.join("Film.mov");
        download_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            &destination,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"whole film");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left.len(), 2, "only the film and the old partial: {left:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn a_download_never_makes_folders_on_its_way() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": format!("{}/bytes", server.uri()) }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bytes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"x".to_vec()))
            .mount(&server)
            .await;

        let dir = tempdir();
        let destination = dir.join("Startup").join("update.bat");
        let saved = download_to(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            &destination,
        )
        .await;
        assert!(saved.is_err());
        assert!(!dir.join("Startup").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// How far a download has got is told as its bytes arrive, ending at all of them.
    #[tokio::test]
    async fn a_download_tells_how_far_it_has_got() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": format!("{}/bytes", server.uri()) }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bytes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1u8; 300_000]))
            .mount(&server)
            .await;
        let dir = tempdir();
        let told = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress: Progress = {
            let told = told.clone();
            std::sync::Arc::new(move |sent, total| told.lock().unwrap().push((sent, total)))
        };
        download_with_progress(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &file(),
            &dir.join("big.bin"),
            Some(progress),
            Placing::New,
        )
        .await
        .unwrap();
        let told = told.lock().unwrap();
        assert_eq!(told.last(), Some(&(300_000, 300_000)));
        assert!(
            told.windows(2).all(|w| w[0].0 <= w[1].0),
            "only ever forward"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A new download never takes the place of anything; a resumed one replaces the earlier
    /// copy of its own file, but never a folder, and a link only as itself.
    #[tokio::test]
    async fn placing_new_keeps_what_is_there_and_replace_takes_its_place() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data_token": { "url": format!("{}/bytes", server.uri()) }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/bytes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"the whole file".to_vec()))
            .mount(&server)
            .await;
        let get = |to: std::path::PathBuf, placing: Placing| {
            let uri = server.uri();
            async move {
                download_with_progress(
                    &reqwest::Client::new(),
                    &uri,
                    vec![],
                    &file(),
                    &to,
                    None,
                    placing,
                )
                .await
            }
        };
        let dir = tempdir();
        let there = dir.join("report.pdf");
        std::fs::write(&there, b"someone's own").unwrap();
        assert!(get(there.clone(), Placing::New).await.is_err());
        assert_eq!(std::fs::read(&there).unwrap(), b"someone's own");

        std::fs::write(&there, b"half").unwrap();
        get(there.clone(), Placing::Replace).await.unwrap();
        assert_eq!(std::fs::read(&there).unwrap(), b"the whole file");

        let folder = dir.join("folder.pdf");
        std::fs::create_dir(&folder).unwrap();
        assert!(get(folder.clone(), Placing::Replace).await.is_err());
        assert!(folder.is_dir());

        #[cfg(unix)]
        {
            let elsewhere = dir.join("elsewhere.txt");
            std::fs::write(&elsewhere, b"untouched").unwrap();
            let link = dir.join("link.pdf");
            std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
            get(link.clone(), Placing::Replace).await.unwrap();
            assert_eq!(std::fs::read(&elsewhere).unwrap(), b"untouched");
            assert!(!std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".part"))
            .collect();
        assert!(left.is_empty(), "{left:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A download stopped part-way — Cancel drops it, mid-stream — takes its partial file with
    /// it, and nothing is left under the real name.
    #[tokio::test]
    async fn a_download_dropped_part_way_leaves_nothing() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // A server that sends the start of a file and then nothing more, as a slow link does.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bytes_at = format!("http://{}/bytes", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\nthe first bytes")
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        });
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "data_token": { "url": bytes_at } })),
            )
            .mount(&server)
            .await;
        let dir = tempdir();
        let destination = dir.join("film.mov");
        let running = {
            let (uri, destination) = (server.uri(), destination.clone());
            tokio::spawn(async move {
                download_to(&reqwest::Client::new(), &uri, vec![], &file(), &destination).await
            })
        };
        let partial = dir.join("film.mov.part");
        for _ in 0..200 {
            if std::fs::metadata(&partial).is_ok_and(|m| m.len() > 0) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(partial.exists(), "the download was under way");
        running.abort();
        let _ = running.await;
        assert!(!partial.exists(), "its partial file went with it");
        assert!(!destination.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A busy Drive or a dropped connection is asked again; anything else is said at once, and
    /// one that stays busy is given up on.
    #[tokio::test]
    async fn only_what_passes_is_asked_again() {
        let tries = std::sync::atomic::AtomicU32::new(0);
        let short = std::time::Duration::from_millis(1);
        let busy_twice = patiently_after(short, &mut || {
            let n = tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(Error::Refused(BUSY.into()))
                } else {
                    Ok(n)
                }
            }
        })
        .await;
        assert_eq!(busy_twice.unwrap(), 2);

        tries.store(0, std::sync::atomic::Ordering::SeqCst);
        let gone: Result<()> = patiently_after(short, &mut || {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Err(Error::NotFound) }
        })
        .await;
        assert!(matches!(gone, Err(Error::NotFound)));
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 1);

        tries.store(0, std::sync::atomic::Ordering::SeqCst);
        let always: Result<()> = patiently_after(short, &mut || {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Err(Error::Refused(BUSY.into())) }
        })
        .await;
        assert!(always.is_err());
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), TRIES);
        assert!(!transient(&Error::ReauthRequired));
        assert!(!transient(&Error::Io(std::io::Error::other("disk"))));
    }

    /// A busy Drive while a download is being prepared, or its bytes fetched, reads as busy —
    /// something to ask again — not as the file being broken.
    #[tokio::test]
    async fn a_busy_drive_reads_as_busy_for_downloads_and_listings() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let link = download_url(&reqwest::Client::new(), &server.uri(), vec![], &file()).await;
        assert!(link.as_ref().is_err_and(transient), "{link:?}");
        let listed = fetch_folder(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "FOLDER::x::y",
        )
        .await;
        assert!(listed.as_ref().is_err_and(transient), "{listed:?}");
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("quince-drive-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

/// A guess at what kind of file this is, from its name.
///
/// Apple stores whatever it is told here and hands it back on download, so a wrong guess shows
/// up later as a file the browser refuses to preview. The list covers what people actually put
/// in a Drive; anything else is declared as bytes, which is honest rather than wrong.
fn content_type(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("heic") => "image/heic",
        Some("svg") => "image/svg+xml",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("html" | "htm") => "text/html",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("wav") => "audio/wav",
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("zip") => "application/zip",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("pages") => "application/x-iwork-pages-sffpages",
        Some("numbers") => "application/x-iwork-numbers-sffnumbers",
        Some("key") => "application/x-iwork-keynote-sffkey",
        _ => "application/octet-stream",
    }
}

/// Where a document is filed, as `update/documents` wants it: the bare id, or `root`.
fn starting_document_id(folder_id: &str) -> &str {
    if folder_id == quince_core::drive::ROOT {
        "root"
    } else {
        docwsid_of(folder_id)
    }
}

/// The container an item belongs to: the middle of `<TYPE>::<zone>::<id>`.
///
/// Most of a Drive is `com.apple.CloudDocs`, but an app library is its own zone — Keynote's
/// is `com.apple.Keynote`, and a third party's is whatever they registered, such as
/// `iCloud.dk.simonbs.Scriptable`. Everyone has a few of these whether they use the apps or
/// not.
fn zone_of(drivewsid: &str) -> Option<&str> {
    let mut parts = drivewsid.split("::");
    let _kind = parts.next()?;
    let zone = parts.next()?;
    (!zone.is_empty()).then_some(zone)
}

/// The ordinary iCloud Drive's zone: everything not in an app's own library.
const DOCUMENTS_ZONE: &str = "com.apple.CloudDocs";

/// Apple's checksum of a file's bytes, as its storage service reports it back.
///
/// Measured against real uploads of 37 bytes and 3 MiB: the receipt's `fileChecksum` is a
/// `0x01` byte and then SHA-1 of the bytes behind a fixed salt, base64. Knowing it means an
/// upload can be proven intact — the bytes Apple holds are the ones on disk, not merely the
/// same number of them.
pub struct AppleChecksum(sha1::Sha1);

const CHECKSUM_SALT: &[u8] = b"com.apple.XattrObjectSalt\0com.apple.DataObjectSalt\0";

impl Default for AppleChecksum {
    fn default() -> Self {
        use sha1::Digest;
        let mut hash = sha1::Sha1::new();
        hash.update(CHECKSUM_SALT);
        Self(hash)
    }
}

impl AppleChecksum {
    pub fn update(&mut self, bytes: &[u8]) {
        use sha1::Digest;
        self.0.update(bytes);
    }

    #[must_use]
    pub fn finish(self) -> String {
        use base64::Engine;
        use sha1::Digest;
        let mut out = vec![0x01u8];
        out.extend_from_slice(&self.0.finalize());
        base64::engine::general_purpose::STANDARD.encode(out)
    }
}

/// How far one upload has got, for a progress bar.
pub type Progress = std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>;

/// Milliseconds since the epoch for a file time, when the filesystem keeps one.
fn millis(time: std::io::Result<std::time::SystemTime>) -> Option<i64> {
    time.ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

/// A file's bytes at Apple, checked and ready to be filed: steps 1 and 2 of [`upload`], shared
/// with [`replace_contents`].
struct SentBytes {
    /// The slot's document id: a new file's own id, when it is filed as one.
    document_id: String,
    /// Apple's account of what it received (`singleFile`): the receipt and checksums filing
    /// names.
    single: Value,
    /// The checksum of exactly the bytes sent, in Apple's form.
    local: String,
    size: u64,
    meta: std::fs::Metadata,
}

/// Somewhere to put the bytes asked for, the bytes sent, and Apple's receipt checked against
/// what was sent: never filed unless Apple received exactly the file.
async fn send_bytes(
    http: &reqwest::Client,
    docws: &str,
    zone: &str,
    params: &[(String, String)],
    name: &str,
    path: &std::path::Path,
    progress: Progress,
) -> Result<SentBytes> {
    use tokio::io::AsyncReadExt;
    let name = name.to_string();
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not read {name}: {e}")))?;
    let size = meta.len();

    // 1. Somewhere to put the bytes.
    let offered: Value = post_json(
        http,
        &format!("{docws}/ws/{zone}/upload/web"),
        params,
        &json!({
            "filename": name,
            "type": "FILE",
            "content_type": content_type(&name),
            "size": size,
        }),
    )
    .await?;
    let slot = offered
        .as_array()
        .and_then(|a| a.first())
        .ok_or_else(|| Error::Protocol("Apple offered nowhere to put the file".into()))?;
    let url = slot
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("Apple's upload slot carried no address".into()))?;
    let document_id = slot
        .get("document_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("Apple's upload slot carried no document id".into()))?;

    // 2. The bytes, straight off disk and never all in memory, checksummed and counted on the
    //    way past so what was sent is known exactly.
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| Error::Protocol(format!("could not open {name}: {e}")))?;
    let checksum = std::sync::Arc::new(std::sync::Mutex::new(Some(AppleChecksum::default())));
    let sent = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stream = futures_util::stream::try_unfold(
        (file, checksum.clone(), sent.clone(), progress.clone()),
        move |(mut file, checksum, sent, progress)| async move {
            let mut buf = vec![0u8; 256 * 1024];
            let n = file.read(&mut buf).await?;
            if n == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            buf.truncate(n);
            if let Some(hash) = checksum.lock().ok().as_mut().and_then(|h| h.as_mut()) {
                hash.update(&buf);
            }
            let so_far = sent.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed) + n as u64;
            progress(so_far, size);
            Ok(Some((buf, (file, checksum, sent, progress))))
        },
    );
    let part =
        reqwest::multipart::Part::stream_with_length(reqwest::Body::wrap_stream(stream), size)
            .file_name(name.clone())
            .mime_str(content_type(&name))
            .map_err(|e| {
                Error::Protocol(format!("{name} has a content type Apple refused: {e}"))
            })?;
    let response = http
        .post(url)
        .multipart(reqwest::multipart::Form::new().part("files", part))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if matches!(status.as_u16(), 429 | 500..=599) {
        return Err(Error::Refused(BUSY.into()));
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        return Err(Error::Protocol(format!(
            "Apple would not take the file ({status}): {}",
            text.chars().take(160).collect::<String>()
        )));
    }
    let local = checksum
        .lock()
        .ok()
        .and_then(|mut h| h.take())
        .map(AppleChecksum::finish)
        .ok_or_else(|| Error::Protocol(format!("{name} could not be checksummed")))?;
    if sent.load(std::sync::atomic::Ordering::Relaxed) != size {
        return Err(Error::Protocol(format!(
            "{name} changed on disk while it was being sent, so nothing was filed"
        )));
    }
    let receipt: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Apple's upload reply was not JSON: {e}")))?;
    let single = receipt
        .get("singleFile")
        .ok_or_else(|| Error::Protocol("Apple took the bytes but described nothing".into()))?;
    if single.get("fileChecksum").and_then(Value::as_str) != Some(local.as_str()) {
        return Err(Error::Protocol(format!(
            "the bytes Apple received for {name} are not the file's, so it was not filed"
        )));
    }
    Ok(SentBytes {
        document_id: document_id.to_string(),
        single: single.clone(),
        local,
        size,
        meta,
    })
}

/// Upload one file into a folder, and prove it arrived whole.
///
/// Three steps, as iCloud.com does it: ask for somewhere to put the bytes, send them, then
/// file them under a name. Every one is checked rather than trusted — the receipt must carry
/// the checksum of exactly the bytes that were sent, and the filed document must carry it
/// again at the same size — and the folder is read back for the document by its id. A file
/// renamed on the way in because the name was taken ("photo 2.jpg") is found by id too, where
/// matching by name reported it missing, and an older file of the same name could stand in
/// for a failed one.
pub async fn upload(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    folder_id: &str,
    path: &std::path::Path,
    progress: Progress,
) -> Result<DriveItem> {
    // A busy Drive at any step is met again from the start — a new slot, the bytes again —
    // so a file half sent is never filed, and a moment's overload costs nothing.
    let mut attempt = 0;
    loop {
        attempt += 1;
        let result = upload_once(
            http,
            docws_root,
            drivews_root,
            params.clone(),
            folder_id,
            path,
            progress.clone(),
        )
        .await;
        match result {
            Err(Error::Refused(ref m)) if m == BUSY && attempt < 3 => {
                tokio::time::sleep(std::time::Duration::from_secs(3 * attempt)).await;
            }
            other => return other,
        }
    }
}

/// What a busy Drive is said to be, in one place so a retry can know it.
const BUSY: &str = "iCloud Drive is busy right now — try this one again in a minute.";

async fn upload_once(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    folder_id: &str,
    path: &std::path::Path,
    progress: Progress,
) -> Result<DriveItem> {
    if folder_id.trim().is_empty() {
        return Err(Error::Protocol("no folder was named to put this in".into()));
    }
    // Filed in the folder's own zone. An app library (Obsidian's vault, Shortcuts', Keynote's)
    // is a zone of its own, and `update/documents` names the folder by a bare id — so asked
    // of the documents zone, Apple looked for that id there and refused with "out of order
    // children ... parents were missing" (rclone/rclone#9729). Asked of the folder's own zone,
    // the zone in the path says where the id is (rclone/rclone#9731).
    let zone = zone_of(folder_id).unwrap_or(DOCUMENTS_ZONE);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Protocol("that file has no name to give it".into()))?
        .to_string();
    let docws = docws_root.trim_end_matches('/');
    let SentBytes {
        document_id,
        single,
        local,
        size,
        meta,
    } = send_bytes(http, docws, zone, &params, &name, path, progress).await?;
    let of = |key: &str| single.get(key).cloned().unwrap_or(Value::Null);

    // 3. File it under a name, with the file's own dates — as Finder keeps them — rather than
    //    the moment it happened to be uploaded.
    let now = millis(Ok(std::time::SystemTime::now())).unwrap_or(0);
    let modified = millis(meta.modified()).unwrap_or(now);
    let created = millis(meta.created()).unwrap_or(modified);
    let filed = post_json(
        http,
        &format!("{docws}/ws/{zone}/update/documents"),
        &params,
        &json!({
            // Apple renames rather than overwrites when the name is taken, which is what a
            // file manager does and what a person expects: nothing already there is lost.
            "allow_conflict": true,
            "btime": created,
            "mtime": modified,
            "command": "add_file",
            "create_short_guid": true,
            "document_id": document_id,
            "file_flags": { "is_executable": false, "is_hidden": false, "is_writable": true },
            "path": { "starting_document_id": starting_document_id(folder_id), "path": name },
            "data": {
                "receipt": of("receipt"),
                "reference_signature": of("referenceChecksum"),
                "signature": of("fileChecksum"),
                "wrapping_key": of("wrappingKey"),
                "size": of("size"),
            },
        }),
    )
    .await;
    let stored = match filed {
        Ok(filed) => filed_document(&filed, &local, size, &name, zone)?,
        // No answer is not "not filed": Apple may have filed it and the reply been lost. Asked
        // again from the start, the file would go up a second time beside the first — "photo
        // 2.jpg". The slot's document id is this file's alone, so the folder says which.
        // So is "something with that name is already there": measured on a busy zone, the
        // first try was filed, the answer to it lost, and the retry met the file itself. It is
        // asked for by its own id, which is current the moment it is filed, where the folder's
        // listing can lag behind it.
        Err(e @ (Error::Network(_) | Error::Refused(_))) => {
            let id = format!("FILE::{zone}::{document_id}");
            match item_details(http, drivews_root, &params, &id).await {
                Ok(Some(item)) if item.size == Some(size as i64) => id,
                _ => match folder_listing(http, drivews_root, &params, folder_id).await {
                    Ok(items)
                        if items
                            .iter()
                            .any(|i| i.drivewsid == id && i.size == Some(size as i64)) =>
                    {
                        id
                    }
                    _ => return Err(e),
                },
            }
        }
        Err(e) => return Err(e),
    };

    // Apple answers 200 to writes it did not make, so the document is read back from Apple:
    // itself, by id, as icloud.com reads an item (`retrieveItemDetails`) — it must be there,
    // in this folder, at this size. Reading the whole folder back instead took six of the
    // nine seconds "Checking" lasted for a 40 MB file in a full folder, for every file sent.
    // A blip while reading it back is asked again, not taken for a failed send: retried
    // whole, a file Apple had filed came back as a conflicted copy beside itself.
    let mut tries = 0u64;
    let found = loop {
        match item_details(http, drivews_root, &params, &stored).await {
            Ok(found) => break found,
            Err(e) if tries >= 2 => return Err(e),
            Err(_) => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(700 * tries)).await;
            }
        }
    };
    match found {
        Some(item)
            if item.parent_id.as_deref() == Some(folder_id) && item.size == Some(size as i64) =>
        {
            Ok(item)
        }
        Some(item) if item.parent_id.as_deref() != Some(folder_id) => {
            // A parent named another way (the root by its own id) is checked against the
            // folder itself, rather than trusted or refused on the spelling.
            let listed = folder_listing(http, drivews_root, &params, folder_id).await?;
            listed
                .into_iter()
                .find(|i| i.drivewsid == stored)
                .ok_or_else(|| {
                    Error::Protocol(format!(
                        "Apple accepted {name} but the folder does not hold it"
                    ))
                })
        }
        Some(_) => Err(Error::Protocol(format!(
            "Apple holds {name} at a different size from the file sent"
        ))),
        None => Err(Error::Protocol(format!(
            "Apple accepted {name} but the folder does not hold it"
        ))),
    }
}

/// New contents for a file already in iCloud Drive, written into that same file (#24).
///
/// The file keeps its identity — its id, its place, its shares and Apple's history of its
/// versions — so an app on another device that follows it by id (a password manager's
/// database, a document left open) still has it. Saved as a new file given the old one's name,
/// all of that was lost, and every save left a copy in Recently Deleted.
///
/// Apple's `modify_file` names the version written over (`etag`) and is refused when the file
/// changed anywhere since: [`Error::Conflict`], nothing written, so a change made on another
/// device is never written over — Apple decides that in the same step as the write, where a
/// look first and a write after left a moment between them. Measured on a real account (probe
/// `drive_modify_in_place`): the same id, the new bytes, nothing in Recently Deleted; a stale
/// version refused with 409 `OP_LOCK_FAILURE` and the newer bytes kept.
pub async fn replace_contents(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    path: &std::path::Path,
    progress: Progress,
) -> Result<DriveItem> {
    // A busy Drive is met again from the start, as an upload's is.
    let mut attempt = 0;
    loop {
        attempt += 1;
        let result = replace_once(
            http,
            docws_root,
            drivews_root,
            params.clone(),
            item,
            path,
            progress.clone(),
        )
        .await;
        match result {
            Err(Error::Refused(ref m)) if m == BUSY && attempt < 3 => {
                tokio::time::sleep(std::time::Duration::from_secs(3 * attempt)).await;
            }
            other => return other,
        }
    }
}

async fn replace_once(
    http: &reqwest::Client,
    docws_root: &str,
    drivews_root: &str,
    params: Vec<(String, String)>,
    item: &DriveItem,
    path: &std::path::Path,
    progress: Progress,
) -> Result<DriveItem> {
    let name = item.display_name();
    if item.kind != "FILE" {
        return Err(Error::Protocol(format!(
            "{name} is not a file, so it has no contents to replace"
        )));
    }
    let etag = item
        .etag
        .as_deref()
        .filter(|e| !e.is_empty())
        .ok_or_else(|| Error::Protocol(format!("{name} has no version to write over")))?;
    let zone = zone_of(&item.drivewsid).unwrap_or(DOCUMENTS_ZONE);
    let document_id = docwsid_of(&item.drivewsid);
    let docws = docws_root.trim_end_matches('/');
    let sent = send_bytes(http, docws, zone, &params, &name, path, progress).await?;
    let of = |key: &str| sent.single.get(key).cloned().unwrap_or(Value::Null);
    let modified = millis(sent.meta.modified())
        .unwrap_or_else(|| millis(Ok(std::time::SystemTime::now())).unwrap_or(0));
    let filed = post_json(
        http,
        &format!("{docws}/ws/{zone}/update/documents"),
        &params,
        &json!({
            "command": "modify_file",
            "document_id": document_id,
            "etag": etag,
            "mtime": modified,
            "data": {
                "receipt": of("receipt"),
                "reference_signature": of("referenceChecksum"),
                "signature": of("fileChecksum"),
                "wrapping_key": of("wrappingKey"),
                "size": of("size"),
            },
        }),
    )
    .await;
    match filed {
        Ok(reply) => {
            let stored = filed_document(&reply, &sent.local, sent.size, &name, zone)?;
            if stored != item.drivewsid {
                return Err(Error::Protocol(format!(
                    "Apple filed the new {name} as another file"
                )));
            }
        }
        // No answer is not "not written": Apple may have written it and the reply been lost.
        // Apple says which bytes the file holds now: this send's, and it went; the version it
        // was sent over, unchanged, and it is sent again later; anything else, another device
        // changed it — taken for a send of ours, the retry kept a conflicted copy of itself.
        Err(e @ (Error::Network(_) | Error::Refused(_))) => {
            match holding(http, docws, &params, zone, document_id).await {
                Ok(Holding { signature, .. })
                    if signature.as_deref() == Some(sent.local.as_str()) => {}
                Ok(Holding { version, .. }) if version.as_deref() == Some(etag) => return Err(e),
                Ok(_) => return Err(Error::Conflict),
                Err(_) => return Err(e),
            }
        }
        Err(e) => return Err(e),
    }

    // Read back by id, as an upload is: Apple answers 200 to writes it did not make.
    let mut tries = 0u64;
    let found = loop {
        match item_details(http, drivews_root, &params, &item.drivewsid).await {
            Ok(found) => break found,
            Err(e) if tries >= 2 => return Err(e),
            Err(_) => {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(700 * tries)).await;
            }
        }
    };
    match found {
        Some(now) if now.size == Some(sent.size as i64) => Ok(now),
        Some(_) => Err(Error::Protocol(format!(
            "Apple holds {name} at a different size from the file sent"
        ))),
        None => Err(Error::Protocol(format!(
            "{name} is no longer in iCloud Drive"
        ))),
    }
}

/// Which bytes a file holds at Apple now, and its version, as `download/by_id` says: the
/// checksum is the one an upload's receipt gives for the same bytes (measured).
struct Holding {
    signature: Option<String>,
    version: Option<String>,
}

async fn holding(
    http: &reqwest::Client,
    docws: &str,
    params: &[(String, String)],
    zone: &str,
    document_id: &str,
) -> Result<Holding> {
    let response = http
        .get(format!("{docws}/ws/{zone}/download/by_id"))
        .query(params)
        .query(&[("document_id", document_id)])
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(Error::Protocol(format!(
            "Apple would not say what the file holds ({})",
            response.status()
        )));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| Error::Protocol(format!("Apple's answer was not JSON: {e}")))?;
    let text = |at: &str| {
        body.pointer(at)
            .and_then(Value::as_str)
            .map(ToString::to_string)
    };
    Ok(Holding {
        signature: text("/data_token/signature"),
        version: text("/double_etag"),
    })
}

/// One item as Apple holds it now, by id (`retrieveItemDetails`, as icloud.com reads one).
/// `None` when Apple has no such item.
async fn item_details(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    drivewsid: &str,
) -> Result<Option<DriveItem>> {
    let url = format!("{}/retrieveItemDetails", root.trim_end_matches('/'));
    let reply = post_json(
        http,
        &url,
        params,
        &json!({ "items": [{ "drivewsid": drivewsid, "includeHierarchy": false }] }),
    )
    .await?;
    Ok(reply
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|i| i.get("drivewsid").and_then(Value::as_str) == Some(drivewsid))
        })
        .filter(|i| {
            i.get("status")
                .and_then(Value::as_str)
                .is_none_or(|s| s == "OK")
        })
        .and_then(item_from))
}

/// The id of the document an `add_file` reply describes, once it is shown to be the file sent:
/// in `zone`, the folder's own, where it was filed.
fn filed_document(
    reply: &Value,
    checksum: &str,
    size: u64,
    name: &str,
    zone: &str,
) -> Result<String> {
    let document = reply
        .pointer("/results/0/document")
        .ok_or_else(|| Error::Protocol(format!("Apple filed {name} without describing it")))?;
    let ok = document
        .pointer("/status/status_code")
        .and_then(Value::as_i64)
        == Some(0);
    let same_bytes = document.pointer("/data/signature").and_then(Value::as_str) == Some(checksum);
    let same_size = document.get("size").and_then(Value::as_u64) == Some(size);
    if !(ok && same_bytes && same_size) {
        return Err(Error::Protocol(format!(
            "what Apple filed as {name} is not the file that was sent"
        )));
    }
    let id = document
        .get("document_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol(format!("Apple filed {name} without an id")))?;
    Ok(format!("FILE::{zone}::{id}"))
}

/// A JSON POST to Drive that fails loudly rather than returning something unusable.
async fn post_json(
    http: &reqwest::Client,
    url: &str,
    params: &[(String, String)],
    body: &Value,
) -> Result<Value> {
    // A busy Drive (503, 429) asks to be asked again shortly. Ninety files going up three at
    // a time met exactly that, and every one that met it was reported as not uploaded. It is
    // waited out, as the server asks, up to three times; only one still busy is reported.
    let mut tries: u64 = 0;
    let (status, text) = loop {
        tries += 1;
        // The account's client sends its web origin and referer on every request: iCloud.com's,
        // or Apple's regional site's for an account kept there (issue #10).
        let response = http.post(url).query(params).json(body).send().await?;
        let status = response.status();
        let wait = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        let text = response.text().await.unwrap_or_default();
        // A 409 is not always a name taken. Measured with three files going into one folder:
        // `ZONE_BUSY` — "Sync zone CAS Op-Lock failed. There was a concurrent write and this
        // operation was rejected. Retry request" — two writes to the zone at once, the second
        // turned away and asked to come again. Read as "something with that name is already
        // there", five of twenty-four files were reported as not uploaded. It is retried, after a
        // short pause that differs per request so the writers stop colliding.
        let zone_busy = status.as_u16() == 409 && text.contains("ZONE_BUSY");
        if zone_busy && tries < 6 {
            let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]) * 2;
            tokio::time::sleep(std::time::Duration::from_millis(250 * tries + jitter)).await;
            continue;
        }
        if !matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504) || tries >= 3 {
            break (status, text);
        }
        let asked = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("retryAfter").and_then(Value::as_u64))
            .or(wait)
            .unwrap_or(2 * tries);
        tokio::time::sleep(std::time::Duration::from_secs(asked.clamp(1, 30))).await;
    };
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        quiet!("drive: a request was refused ({status})");
        if status.as_u16() == 409 && text.contains("ZONE_BUSY") {
            return Err(Error::Refused(BUSY.into()));
        }
        // A write naming a version of the item that is no longer Apple's: it changed
        // elsewhere first, and nothing was written (measured: `modify_file` with a stale etag).
        if status.as_u16() == 409 && text.contains("OP_LOCK_FAILURE") {
            return Err(Error::Conflict);
        }
        return Err(Error::Refused(match status.as_u16() {
            429 | 500..=599 => {
                "iCloud Drive is busy right now — try this one again in a minute.".into()
            }
            409 => "Something with that name is already there.".into(),
            403 => "iCloud Drive won't let this be changed.".into(),
            code => format!("iCloud Drive turned this down (status {code})."),
        }));
    }
    serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Drive returned invalid JSON: {e}")))
}

#[cfg(test)]
mod busy_tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Ninety files three at a time met a busy Drive, and each was reported as not uploaded.
    #[tokio::test]
    async fn a_busy_drive_is_waited_out() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "1"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })),
            )
            .mount(&server)
            .await;
        let v = post_json(
            &reqwest::Client::new(),
            &server.uri(),
            &[],
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        assert_eq!(v["ok"], true);
    }

    #[tokio::test]
    async fn a_drive_that_stays_busy_says_so_plainly() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "1"))
            .mount(&server)
            .await;
        let e = post_json(
            &reqwest::Client::new(),
            &server.uri(),
            &[],
            &serde_json::json!({}),
        )
        .await
        .unwrap_err();
        assert_eq!(
            e.to_string(),
            "iCloud Drive is busy right now — try this one again in a minute."
        );
    }
}

#[cfg(test)]
mod upload_tests {
    use super::*;

    #[test]
    fn a_file_is_declared_as_what_it_is() {
        assert_eq!(content_type("holiday.JPG"), "image/jpeg");
        assert_eq!(content_type("notes.md"), "text/plain");
        assert_eq!(
            content_type("budget.numbers"),
            "application/x-iwork-numbers-sffnumbers"
        );
        assert_eq!(content_type("archive.tar.gz"), "application/octet-stream");
        assert_eq!(content_type("noextension"), "application/octet-stream");
        assert_eq!(content_type(""), "application/octet-stream");
    }

    #[test]
    fn an_item_names_the_container_it_belongs_to() {
        assert_eq!(
            zone_of("FOLDER::com.apple.CloudDocs::root"),
            Some("com.apple.CloudDocs")
        );
        assert_eq!(
            zone_of("FOLDER::iCloud.dk.simonbs.Scriptable::documents"),
            Some("iCloud.dk.simonbs.Scriptable"),
            "a third party's library is a zone like any other"
        );
        assert_eq!(
            zone_of("FOLDER::iCloud.is.workflow.my.workflows::documents"),
            Some("iCloud.is.workflow.my.workflows")
        );
        assert_eq!(zone_of("nonsense"), None);
        assert_eq!(zone_of("FOLDER::"), None);
    }

    /// The root has a name of its own in the listing and a different one in a document path.
    #[test]
    fn the_root_is_called_root_when_filing_a_document() {
        assert_eq!(starting_document_id(quince_core::drive::ROOT), "root");
        assert_eq!(
            starting_document_id("FOLDER::com.apple.CloudDocs::7B3F-ABCD"),
            "7B3F-ABCD"
        );
    }
}

#[cfg(test)]
mod replace_tests {
    //! `replace_contents` against a stand-in for Apple, answering as the real one did
    //! (probe `drive_modify_in_place`).
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ID: &str = "FILE::com.apple.CloudDocs::DB";

    fn item() -> DriveItem {
        DriveItem {
            drivewsid: ID.into(),
            parent_id: Some("FOLDER::com.apple.CloudDocs::root".into()),
            name: "Passwords".into(),
            extension: Some("kdbx".into()),
            kind: "FILE".into(),
            size: Some(3),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: Some("v1".into()),
        }
    }

    /// Apple taking the bytes: a slot and the upload's receipt for `bytes`.
    async fn takes_bytes(server: &MockServer, bytes: &[u8]) -> String {
        let mut sum = AppleChecksum::default();
        sum.update(bytes);
        let sum = sum.finish();
        Mock::given(method("POST"))
            .and(path("/ws/com.apple.CloudDocs/upload/web"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "url": format!("{}/content", server.uri()), "document_id": "SLOT" }
            ])))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/content"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "singleFile": { "fileChecksum": sum, "size": bytes.len(), "receipt": "r",
                                "referenceChecksum": "ref", "wrappingKey": "k" }
            })))
            .mount(server)
            .await;
        sum
    }

    async fn holds_now(server: &MockServer, size: usize) {
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetails"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [{
                "drivewsid": ID, "parentId": "FOLDER::com.apple.CloudDocs::root", "name": "Passwords",
                "extension": "kdbx", "type": "FILE", "size": size, "etag": "v2", "status": "OK"
            }]})))
            .mount(server)
            .await;
    }

    async fn replace(server: &MockServer, file: &std::path::Path) -> Result<DriveItem> {
        replace_contents(
            &reqwest::Client::new(),
            &server.uri(),
            &server.uri(),
            vec![],
            &item(),
            file,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
    }

    /// The bytes in a folder of their own, removed when the test is done with them.
    struct Staged(std::path::PathBuf);
    impl Drop for Staged {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent() {
                std::fs::remove_dir_all(dir).ok();
            }
        }
    }
    fn staged(bytes: &[u8]) -> (Staged, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("quince-replace-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Passwords.kdbx");
        std::fs::write(&file, bytes).unwrap();
        (Staged(file.clone()), file)
    }

    /// Written into the same document, at the version it was read at; the same id comes back.
    #[tokio::test]
    async fn new_contents_go_into_the_same_document_at_its_version() {
        let server = MockServer::start().await;
        let sum = takes_bytes(&server, b"new db").await;
        Mock::given(method("POST"))
            .and(path("/ws/com.apple.CloudDocs/update/documents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{ "document": { "document_id": "DB", "size": 6,
                    "status": { "status_code": 0 }, "data": { "signature": sum } } }]
            })))
            .mount(&server)
            .await;
        holds_now(&server, 6).await;
        let (_dir, file) = staged(b"new db");
        let now = replace(&server, &file).await.expect("written in place");
        assert_eq!(now.drivewsid, ID);
        let reqs = server.received_requests().await.unwrap();
        let filed = reqs
            .iter()
            .find(|r| r.url.path().ends_with("/update/documents"))
            .unwrap();
        let body: Value = serde_json::from_slice(&filed.body).unwrap();
        assert_eq!(body["command"], "modify_file");
        assert_eq!(
            body["document_id"], "DB",
            "the file's own id, never the slot's"
        );
        assert_eq!(body["etag"], "v1", "the version it was read at");
        assert!(
            body.get("allow_conflict").is_none(),
            "a stale version must be refused"
        );
        assert_eq!(body["data"]["signature"], sum.as_str());
    }

    /// Changed on another device since: Apple's 409 OP_LOCK_FAILURE is a conflict, never a
    /// name clash and never retried as a write over theirs.
    #[tokio::test]
    async fn a_version_changed_elsewhere_is_a_conflict() {
        let server = MockServer::start().await;
        takes_bytes(&server, b"ours").await;
        Mock::given(method("POST"))
            .and(path("/ws/com.apple.CloudDocs/update/documents"))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
                "error_code": "OP_LOCK_FAILURE", "serverErrorCode": "OP_LOCK_FAILURE",
                "reason": "client oplock error updating record"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (_dir, file) = staged(b"ours");
        assert!(matches!(
            replace(&server, &file).await,
            Err(Error::Conflict)
        ));
    }

    /// The answer lost on the way back: what Apple holds says whether the write went.
    #[tokio::test]
    async fn a_lost_answer_is_settled_by_what_apple_holds() {
        for (holding, version, expect) in [
            ("ours", "v2", "written"),
            ("theirs", "v1", "again"),
            ("theirs", "v3", "conflict"),
        ] {
            let server = MockServer::start().await;
            let sum = takes_bytes(&server, b"ours").await;
            Mock::given(method("POST"))
                .and(path("/ws/com.apple.CloudDocs/update/documents"))
                // The write's answer never comes back in time: the connection gives up on it.
                .respond_with(
                    ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(5)),
                )
                .mount(&server)
                .await;
            let signature = if holding == "ours" {
                sum.clone()
            } else {
                "OTHER".into()
            };
            Mock::given(method("GET"))
                .and(path("/ws/com.apple.CloudDocs/download/by_id"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data_token": { "url": "u", "signature": signature }, "double_etag": version
                })))
                .mount(&server)
                .await;
            holds_now(&server, 4).await;
            let (_dir, file) = staged(b"ours");
            let impatient = reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(500))
                .build()
                .unwrap();
            let got = replace_contents(
                &impatient,
                &server.uri(),
                &server.uri(),
                vec![],
                &item(),
                &file,
                std::sync::Arc::new(|_, _| {}),
            )
            .await;
            match expect {
                "written" => assert_eq!(got.expect("it went").drivewsid, ID),
                "again" => assert!(matches!(got, Err(Error::Network(_))), "{got:?}"),
                _ => assert!(matches!(got, Err(Error::Conflict)), "{got:?}"),
            }
        }
    }
}

#[cfg(test)]
mod app_library_tests {
    use super::*;

    /// A file put in an app's library — Obsidian's vault, Shortcuts' — is asked a slot of and
    /// filed in that library's own zone, under the folder's id there, and comes back with an id
    /// in that zone (rclone/rclone#9729). Asked of the documents zone, Apple refused it: the
    /// folder's id is not there.
    #[tokio::test]
    async fn a_file_is_filed_in_its_app_librarys_own_zone() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        const ZONE: &str = "iCloud.md.obsidian";
        let server = MockServer::start().await;
        let mut sum = AppleChecksum::default();
        sum.update(b"# vault");
        let sum = sum.finish();
        Mock::given(method("POST"))
            .and(path(format!("/ws/{ZONE}/upload/web")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "url": format!("{}/content", server.uri()), "document_id": "D1" }
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/content"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "singleFile": { "fileChecksum": sum, "size": 7, "receipt": "r" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/ws/{ZONE}/update/documents")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "results": [{ "document": {
                    "document_id": "D1", "size": 7,
                    "status": { "status_code": 0 }, "data": { "signature": sum }
                } }]
            })))
            .mount(&server)
            .await;
        let folder = format!("FOLDER::{ZONE}::documents");
        Mock::given(method("POST"))
            .and(path("/retrieveItemDetails"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [{
                    "drivewsid": format!("FILE::{ZONE}::D1"), "parentId": folder,
                    "name": "Note", "extension": "md", "type": "FILE", "size": 7, "status": "OK"
                }]})),
            )
            .mount(&server)
            .await;
        let file = std::env::temp_dir().join("quince-app-zone-Note.md");
        std::fs::write(&file, b"# vault").unwrap();
        let item = upload(
            &reqwest::Client::new(),
            &server.uri(),
            &server.uri(),
            vec![],
            &folder,
            &file,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
        .expect("an app's library takes a file in its own zone");
        std::fs::remove_file(&file).ok();
        assert_eq!(item.drivewsid, format!("FILE::{ZONE}::D1"));
        let reqs = server.received_requests().await.unwrap();
        assert!(
            reqs.iter()
                .all(|r| !r.url.path().contains("com.apple.CloudDocs")),
            "nothing asked of the documents zone"
        );
        let filed = reqs
            .iter()
            .find(|r| r.url.path().ends_with("/update/documents"))
            .expect("filed");
        let body: Value = serde_json::from_slice(&filed.body).unwrap();
        assert_eq!(body["path"]["starting_document_id"], "documents");
    }

    /// A busy Drive while the bytes go up: the file is tried again from a fresh slot, up to
    /// three times, and only then reported busy — not dropped at the first refusal.
    #[tokio::test]
    async fn a_file_refused_as_busy_is_sent_again() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/upload/web$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "url": format!("{}/content", server.uri()), "document_id": "D1" }
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let file = std::env::temp_dir().join("quince-busy-test.txt");
        std::fs::write(&file, b"hello").unwrap();
        let err = upload(
            &reqwest::Client::new(),
            &server.uri(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            &file,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
        .unwrap_err();
        let slots = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/upload/web"))
            .count();
        assert_eq!(slots, 3, "three tries, each from a fresh slot");
        assert_eq!(err.to_string(), BUSY);
        std::fs::remove_file(&file).ok();
    }

    /// Filing answered busy every time, though Apple did file it: the folder shows the slot's
    /// document, so that is the upload — not a second copy sent from a fresh slot.
    #[tokio::test]
    async fn a_file_filed_without_an_answer_is_not_sent_twice() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let mut sum = AppleChecksum::default();
        sum.update(b"hello");
        let sum = sum.finish();
        Mock::given(method("POST"))
            .and(path_regex(r".*/upload/web$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "url": format!("{}/content", server.uri()), "document_id": "D1" }
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "singleFile": { "fileChecksum": sum, "size": 5, "receipt": "r" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/update/documents$"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(serde_json::json!({ "retryAfter": 1 })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/retrieveItemDetailsInFolders$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{
                "drivewsid": "FOLDER::com.apple.CloudDocs::root", "status": "OK",
                "items": [{ "drivewsid": "FILE::com.apple.CloudDocs::D1", "name": "quince-lost-reply",
                            "extension": "txt", "type": "FILE", "size": 5 }]
            }])))
            .mount(&server)
            .await;
        // And the document itself, read back by id as uploads now confirm it.
        Mock::given(method("POST"))
            .and(path_regex(r".*/retrieveItemDetails$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "items": [{
                "drivewsid": "FILE::com.apple.CloudDocs::D1", "parentId": "FOLDER::com.apple.CloudDocs::root",
                "name": "quince-lost-reply", "extension": "txt", "type": "FILE", "size": 5, "status": "OK"
            }]})))
            .mount(&server)
            .await;
        let file = std::env::temp_dir().join("quince-lost-reply.txt");
        std::fs::write(&file, b"hello").unwrap();
        let item = upload(
            &reqwest::Client::new(),
            &server.uri(),
            &server.uri(),
            vec![],
            "FOLDER::com.apple.CloudDocs::root",
            &file,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
        .expect("the folder holds it, so it was uploaded");
        assert_eq!(item.drivewsid, "FILE::com.apple.CloudDocs::D1");
        let slots = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/upload/web"))
            .count();
        assert_eq!(slots, 1, "sent once, never again beside itself");
        std::fs::remove_file(&file).ok();
    }

    /// The documents zone is everything else, and must still take a file.
    #[tokio::test]
    async fn the_ordinary_drive_is_not_refused() {
        let file = std::env::temp_dir().join("quince-zone-ok.txt");
        std::fs::write(&file, b"x").unwrap();
        let err = upload(
            &reqwest::Client::new(),
            "http://127.0.0.1:1",
            "http://127.0.0.1:1",
            vec![],
            quince_core::drive::ROOT,
            &file,
            std::sync::Arc::new(|_, _| {}),
        )
        .await
        .expect_err("nothing is listening, so this fails on the network");
        assert!(
            !err.to_string().contains("only the app"),
            "the documents zone must get as far as trying: {err}"
        );
        std::fs::remove_file(&file).ok();
    }
}

#[cfg(test)]
mod download_zone_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn file(drivewsid: &str) -> DriveItem {
        DriveItem {
            drivewsid: drivewsid.into(),
            parent_id: None,
            name: "Morning".into(),
            extension: Some("shortcut".into()),
            kind: "FILE".into(),
            size: Some(10),
            child_count: None,
            created_at: None,
            modified_at: None,
            etag: None,
        }
    }

    /// A file in an app library is asked for under that library's zone. Under the ordinary
    /// Drive zone Apple answers "Could not find document" — measured against a Shortcuts file.
    #[tokio::test]
    async fn a_download_asks_for_the_file_under_its_own_zone() {
        let server = MockServer::start().await;
        for zone in ["iCloud.is.workflow.my.workflows", "com.apple.CloudDocs"] {
            Mock::given(method("GET"))
                .and(path(format!("/ws/{zone}/download/by_id")))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({ "data_token": { "url": format!("https://x/{zone}") } }),
                ))
                .mount(&server)
                .await;
        }
        let http = reqwest::Client::new();
        let got = download_url(
            &http,
            &server.uri(),
            vec![],
            &file("FILE::iCloud.is.workflow.my.workflows::ABC"),
        )
        .await
        .unwrap();
        assert!(got.ends_with("iCloud.is.workflow.my.workflows"), "{got}");

        let ordinary = download_url(
            &http,
            &server.uri(),
            vec![],
            &file("FILE::com.apple.CloudDocs::XYZ"),
        )
        .await
        .unwrap();
        assert!(ordinary.ends_with("com.apple.CloudDocs"), "{ordinary}");
    }

    /// A Pages, Numbers or Keynote document is a package, and Apple answers its download with
    /// a `package_token` rather than a `data_token` (timlaing/pyicloud `drive.py`,
    /// mandarons/icloudpy#1). It downloads from there, rather than failing as a reply with
    /// no address.
    #[tokio::test]
    async fn a_package_downloads_from_its_package_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ws/com.apple.CloudDocs/download/by_id"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "document_id": "KEY",
                "package_token": { "url": "https://cvws.example/packageDownload?k=1" },
                "double_etag": "e1"
            })))
            .mount(&server)
            .await;
        let mut talk = file("FILE::com.apple.CloudDocs::KEY");
        talk.extension = Some("key".into());
        talk.etag = Some("e1".into());
        let http = reqwest::Client::new();
        for exact in [false, true] {
            let got = if exact {
                download_url_of_version(&http, &server.uri(), vec![], &talk).await
            } else {
                download_url(&http, &server.uri(), vec![], &talk).await
            };
            assert_eq!(
                got.unwrap(),
                "https://cvws.example/packageDownload?k=1",
                "exact: {exact}"
            );
        }
        assert_eq!(
            download_link_in(&serde_json::json!({ "data_token": { "url": "https://a" },
                                                   "package_token": { "url": "https://b" } })),
            Some("https://a".into()),
            "a file's own bytes first"
        );
        assert_eq!(download_link_in(&serde_json::json!({})), None);
    }

    /// An empty file is saved empty, without asking Apple for a link to no bytes: Apple
    /// answers that with a 400 (picklepete/pyicloud#301), and the download failed.
    #[tokio::test]
    async fn an_empty_file_downloads_without_asking() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;
        let dir =
            std::env::temp_dir().join(format!("quince-empty-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let to = dir.join("empty.txt");
        let mut empty = file("FILE::com.apple.CloudDocs::EMPTY");
        empty.size = Some(0);
        download_to(&reqwest::Client::new(), &server.uri(), vec![], &empty, &to)
            .await
            .expect("an empty file is saved");
        assert_eq!(std::fs::read(&to).unwrap(), Vec::<u8>::new());
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "nothing asked of Apple"
        );
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "and no partial file left beside it"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// The account's iCloud storage, as icloud.com's "X GB available" reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// `POST {setup}/storageUsageInfo`, no body — what icloud.com's shell asks. Measured on a real
/// account: `storageUsageInfo.{usedStorageInBytes,totalStorageInBytes}`.
pub async fn storage(
    http: &reqwest::Client,
    setup_root: &str,
    params: Vec<(String, String)>,
) -> Result<Storage> {
    let url = format!("{}/storageUsageInfo", setup_root.trim_end_matches('/'));
    let response = http.post(url).query(&params).send().await?;
    let status = response.status();
    if status.as_u16() == 450 || status.as_u16() == 421 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!("storage returned {status}")));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| Error::Protocol(format!("storage: {e}")))?;
    let n = |k: &str| {
        body.pointer(&format!("/storageUsageInfo/{k}"))
            .and_then(Value::as_u64)
    };
    match (n("usedStorageInBytes"), n("totalStorageInBytes")) {
        (Some(used_bytes), Some(total_bytes)) => Ok(Storage {
            used_bytes,
            total_bytes,
        }),
        _ => Err(Error::Protocol("storage: no usage in the answer".into())),
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The shape measured on a real account.
    #[tokio::test]
    async fn storage_is_read_from_apples_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/storageUsageInfo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "storageUsageInfo": { "compStorageInBytes": 0, "usedStorageInBytes": 182_196_857_u64,
                    "totalStorageInBytes": 5_368_709_120_u64, "commerceStorageInBytes": 0 },
                "quotaStatus": { "overQuota": false }
            })))
            .mount(&server)
            .await;
        let got = storage(&reqwest::Client::new(), &server.uri(), vec![])
            .await
            .unwrap();
        assert_eq!(
            got,
            Storage {
                used_bytes: 182_196_857,
                total_bytes: 5_368_709_120
            }
        );
    }
}

#[cfg(test)]
mod recents_tests {
    #[test]
    fn recents_are_read_newest_opened_first() {
        let body = serde_json::json!({ "documents": [
            { "drivewsid": "FILE::z::a", "name": "Old", "extension": "pdf", "type": "FILE", "size": 10, "lastOpenTime": "2026-10-01T10:00:00Z" },
            { "drivewsid": "FILE::z::b", "name": "New", "extension": "pdf", "type": "FILE", "size": 20, "lastOpenTime": "2026-10-16T18:46:11Z" },
        ]});
        let got = super::parse_recents(&body);
        assert_eq!(
            got.iter().map(|r| r.item.name.as_str()).collect::<Vec<_>>(),
            ["New", "Old"]
        );
        assert_eq!(got[0].opened_at, Some(1_792_176_371));
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    const ROOT: &str = quince_core::drive::ROOT;
    const WORK: &str = "FOLDER::com.apple.CloudDocs::work";

    /// Finder's names for a copy, as it gives them.
    #[test]
    fn a_copy_is_named_as_finder_names_one() {
        let taken = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert_eq!(copy_name("Report", Some("pdf"), &[]), "Report.pdf");
        assert_eq!(
            copy_name("Report", Some("pdf"), &taken(&["Report.pdf"])),
            "Report copy.pdf"
        );
        assert_eq!(
            copy_name(
                "Report",
                Some("pdf"),
                &taken(&["report.PDF", "Report copy.pdf"])
            ),
            "Report copy 2.pdf",
            "names are the same whatever their case"
        );
        assert_eq!(
            copy_name(
                "Report",
                Some("pdf"),
                &taken(&["Report.pdf", "Report copy.pdf", "Report copy 2.pdf"])
            ),
            "Report copy 3.pdf"
        );
        // A copy of a copy counts on, never "copy copy".
        assert_eq!(
            copy_name("Report copy", Some("pdf"), &taken(&["Report copy.pdf"])),
            "Report copy 2.pdf"
        );
        assert_eq!(
            copy_name(
                "Report copy 2",
                Some("pdf"),
                &taken(&["Report copy.pdf", "Report copy 2.pdf"])
            ),
            "Report copy 3.pdf"
        );
        // A folder, and a file with no extension.
        assert_eq!(copy_name("Trip", None, &taken(&["Trip"])), "Trip copy");
        assert_eq!(
            copy_name("Makefile", Some(""), &taken(&["Makefile"])),
            "Makefile copy"
        );
        // "copy" inside a name is just a word.
        assert_eq!(
            copy_name("copy", Some("txt"), &taken(&["copy.txt"])),
            "copy copy.txt"
        );
        assert_eq!(
            copy_name("Hard copy 2b", None, &taken(&["Hard copy 2b"])),
            "Hard copy 2b copy"
        );
    }

    #[derive(Clone, Debug)]
    struct Item {
        id: String,
        item_id: String,
        parent: String,
        name: String,
        ext: Option<String>,
        kind: &'static str,
    }

    impl Item {
        fn whole(&self) -> String {
            match &self.ext {
                Some(e) => format!("{}.{e}", self.name),
                None => self.name.clone(),
            }
        }
    }

    #[derive(Default)]
    struct State {
        items: Vec<Item>,
        made: u32,
        /// Each write, in order: what was done, and to which name.
        writes: Vec<String>,
        refuse_moves: bool,
    }

    /// A stand-in for iCloud Drive's web service that keeps its folders as Apple does — a copy
    /// lands beside its original as "Name 2", and a name already in a folder is refused there —
    /// so a paste's steps are checked against what the service would do with them.
    #[derive(Clone)]
    struct FakeDrive(Arc<Mutex<State>>);

    impl FakeDrive {
        fn new(items: &[(&str, &str, &str, Option<&str>, &'static str)]) -> Self {
            let state = State {
                items: items
                    .iter()
                    .map(|(id, parent, name, ext, kind)| Item {
                        id: (*id).into(),
                        item_id: format!("i-{id}"),
                        parent: (*parent).into(),
                        name: (*name).into(),
                        ext: ext.map(Into::into),
                        kind,
                    })
                    .collect(),
                ..State::default()
            };
            Self(Arc::new(Mutex::new(state)))
        }

        fn names(&self, folder: &str) -> Vec<String> {
            let mut names: Vec<String> = self
                .0
                .lock()
                .unwrap()
                .items
                .iter()
                .filter(|i| i.parent == folder)
                .map(Item::whole)
                .collect();
            names.sort();
            names
        }

        fn item(&self, id: &str) -> DriveItem {
            let state = self.0.lock().unwrap();
            let i = state.items.iter().find(|i| i.id == id).unwrap();
            DriveItem {
                drivewsid: i.id.clone(),
                parent_id: Some(i.parent.clone()),
                name: i.name.clone(),
                extension: i.ext.clone(),
                kind: i.kind.into(),
                size: None,
                child_count: None,
                created_at: None,
                modified_at: None,
                etag: Some("e".into()),
            }
        }

        fn writes(&self) -> Vec<String> {
            self.0.lock().unwrap().writes.clone()
        }
    }

    fn taken_in(state: &State, folder: &str, name: &str, except: &str) -> bool {
        state
            .items
            .iter()
            .any(|i| i.parent == folder && i.id != except && i.whole().eq_ignore_ascii_case(name))
    }

    impl Respond for FakeDrive {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let mut state = self.0.lock().unwrap();
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let path = request.url.path().to_string();
            let refused = ResponseTemplate::new(409);
            if path.ends_with("/retrieveItemDetailsInFolders") {
                let folder = body[0]["drivewsid"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let known = folder == ROOT || state.items.iter().any(|i| i.id == folder);
                let items: Vec<Value> = state
                    .items
                    .iter()
                    .filter(|i| i.parent == folder)
                    .map(|i| {
                        json!({ "drivewsid": i.id, "parentId": i.parent, "name": i.name,
                                "extension": i.ext, "type": i.kind, "etag": "e", "item_id": i.item_id })
                    })
                    .collect();
                let status = if known { "OK" } else { "ID_INVALID" };
                return ResponseTemplate::new(200).set_body_json(
                    json!([{ "drivewsid": folder, "status": status, "items": items }]),
                );
            }
            if let Some(item_id) = path.strip_prefix("/v1/item/copy/") {
                let Some(original) = state.items.iter().find(|i| i.item_id == item_id).cloned()
                else {
                    return ResponseTemplate::new(404);
                };
                // Apple's own name for a copy beside its original: "Report 2.pdf".
                let name = (2..)
                    .map(|n| format!("{} {n}", original.name))
                    .find(|n| {
                        let whole = Item {
                            name: n.clone(),
                            ..original.clone()
                        }
                        .whole();
                        !taken_in(&state, &original.parent, &whole, "")
                    })
                    .unwrap();
                state.made += 1;
                let id = format!("FILE::com.apple.CloudDocs::copy{}", state.made);
                state.writes.push(format!("copy {}", original.whole()));
                state.items.push(Item {
                    item_id: format!("i-{id}"),
                    id,
                    name,
                    ..original
                });
                return ResponseTemplate::new(200).set_body_json(json!({}));
            }
            if path.ends_with("/renameItems") {
                let id = body["items"][0]["drivewsid"].as_str().unwrap().to_string();
                let whole = body["items"][0]["name"].as_str().unwrap().to_string();
                let at = state.items.iter().position(|i| i.id == id).unwrap();
                let parent = state.items[at].parent.clone();
                if taken_in(&state, &parent, &whole, &id) {
                    return refused;
                }
                let item = &mut state.items[at];
                match (&item.ext, whole.rsplit_once('.')) {
                    (Some(_), Some((stem, ext))) if item.kind == "FILE" => {
                        item.name = stem.into();
                        item.ext = Some(ext.into());
                    }
                    _ => item.name.clone_from(&whole),
                }
                state.writes.push(format!("rename {whole}"));
                return ResponseTemplate::new(200).set_body_json(json!({ "items": [] }));
            }
            if path.ends_with("/moveItems") {
                if state.refuse_moves {
                    return ResponseTemplate::new(403);
                }
                let id = body["items"][0]["drivewsid"].as_str().unwrap().to_string();
                let to = body["destinationDrivewsId"].as_str().unwrap().to_string();
                let at = state.items.iter().position(|i| i.id == id).unwrap();
                let whole = state.items[at].whole();
                if taken_in(&state, &to, &whole, &id) {
                    return refused;
                }
                state.items[at].parent.clone_from(&to);
                state.writes.push(format!("move {whole}"));
                return ResponseTemplate::new(200).set_body_json(json!({ "items": [] }));
            }
            if path.ends_with("/createFolders") {
                let parent = body["destinationDrivewsId"].as_str().unwrap().to_string();
                let name = body["folders"][0]["name"].as_str().unwrap().to_string();
                if taken_in(&state, &parent, &name, "") {
                    return ResponseTemplate::new(200).set_body_json(
                        json!({ "folders": [{ "clientId": "x", "status": "UNKNOWN" }] }),
                    );
                }
                state.made += 1;
                let id = format!("FOLDER::com.apple.CloudDocs::made{}", state.made);
                state.writes.push(format!("folder {name}"));
                state.items.push(Item {
                    id: id.clone(),
                    item_id: format!("i-{id}"),
                    parent: parent.clone(),
                    name: name.clone(),
                    ext: None,
                    kind: "FOLDER",
                });
                return ResponseTemplate::new(200).set_body_json(json!({ "folders": [{
                    "drivewsid": id, "parentId": parent, "name": name, "type": "FOLDER", "status": "OK"
                }] }));
            }
            if path.ends_with("/moveItemsToTrash") {
                let id = body["items"][0]["drivewsid"].as_str().unwrap().to_string();
                let whole = state.items.iter().find(|i| i.id == id).map(Item::whole);
                state.items.retain(|i| i.id != id);
                state
                    .writes
                    .push(format!("trash {}", whole.unwrap_or_default()));
                return ResponseTemplate::new(200)
                    .set_body_json(json!({ "items": [{ "drivewsid": id, "status": "OK" }] }));
            }
            ResponseTemplate::new(404)
        }
    }

    async fn serve(drive: &FakeDrive) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(drive.clone())
            .mount(&server)
            .await;
        server
    }

    async fn paste(server: &MockServer, item: &DriveItem, into: &str) -> Result<DriveItem> {
        let uri = server.uri();
        copy_to(&reqwest::Client::new(), &uri, &uri, vec![], item, into).await
    }

    const REPORT: &str = "FILE::com.apple.CloudDocs::report";

    /// Into a folder without one of its name, the copy keeps its name: copied beside the
    /// original, named there, moved — and the original is where it was.
    #[tokio::test]
    async fn a_file_pasted_elsewhere_keeps_its_name() {
        let drive = FakeDrive::new(&[
            (REPORT, ROOT, "Report", Some("pdf"), "FILE"),
            (WORK, ROOT, "Work", None, "FOLDER"),
        ]);
        let server = serve(&drive).await;
        let copy = paste(&server, &drive.item(REPORT), WORK).await.unwrap();
        assert_eq!(drive.names(WORK), ["Report.pdf"]);
        assert_eq!(drive.names(ROOT), ["Report.pdf", "Work"]);
        assert_eq!(copy.display_name(), "Report.pdf");
        assert_eq!(copy.parent_id.as_deref(), Some(WORK));
        // Apple's "Report 2.pdf" is free in Work, so it travels under that and is named there.
        assert_eq!(
            drive.writes(),
            ["copy Report.pdf", "move Report 2.pdf", "rename Report.pdf"]
        );
    }

    /// Where the name is taken, the copy is "Report copy.pdf", then "Report copy 2.pdf".
    #[tokio::test]
    async fn a_file_pasted_where_its_name_is_taken_is_named_as_finder_names_a_copy() {
        let drive = FakeDrive::new(&[
            (REPORT, ROOT, "Report", Some("pdf"), "FILE"),
            (WORK, ROOT, "Work", None, "FOLDER"),
            (
                "FILE::com.apple.CloudDocs::theirs",
                WORK,
                "Report",
                Some("pdf"),
                "FILE",
            ),
            // Taken in Work too: the name Apple gives the copy cannot be the one it travels under.
            (
                "FILE::com.apple.CloudDocs::two",
                WORK,
                "Report 2",
                Some("pdf"),
                "FILE",
            ),
        ]);
        let server = serve(&drive).await;
        paste(&server, &drive.item(REPORT), WORK).await.unwrap();
        assert_eq!(
            drive.names(WORK),
            ["Report 2.pdf", "Report copy.pdf", "Report.pdf"]
        );
        paste(&server, &drive.item(REPORT), WORK).await.unwrap();
        assert_eq!(
            drive.names(WORK),
            [
                "Report 2.pdf",
                "Report copy 2.pdf",
                "Report copy.pdf",
                "Report.pdf"
            ]
        );
        assert_eq!(
            drive.names(ROOT),
            ["Report.pdf", "Work"],
            "nothing left beside the original"
        );
    }

    /// Pasted into its own folder, it is a duplicate named "Report copy.pdf", not Apple's
    /// "Report 2.pdf".
    #[tokio::test]
    async fn a_file_pasted_into_its_own_folder_is_a_finder_duplicate() {
        let drive = FakeDrive::new(&[(REPORT, ROOT, "Report", Some("pdf"), "FILE")]);
        let server = serve(&drive).await;
        paste(&server, &drive.item(REPORT), ROOT).await.unwrap();
        paste(&server, &drive.item(REPORT), ROOT).await.unwrap();
        assert_eq!(
            drive.names(ROOT),
            ["Report copy 2.pdf", "Report copy.pdf", "Report.pdf"]
        );
    }

    /// A folder is made anew where it is pasted, with all that is in it copied in under the
    /// same names, folders within folders too.
    #[tokio::test]
    async fn a_folder_is_pasted_with_everything_in_it() {
        let trip = "FOLDER::com.apple.CloudDocs::trip";
        let days = "FOLDER::com.apple.CloudDocs::days";
        let drive = FakeDrive::new(&[
            (trip, ROOT, "Trip", None, "FOLDER"),
            (
                "FILE::com.apple.CloudDocs::plan",
                trip,
                "Plan",
                Some("pdf"),
                "FILE",
            ),
            (
                "FILE::com.apple.CloudDocs::plan2",
                trip,
                "Plan 2",
                Some("pdf"),
                "FILE",
            ),
            (days, trip, "Days", None, "FOLDER"),
            (
                "FILE::com.apple.CloudDocs::d1",
                days,
                "Day one",
                Some("txt"),
                "FILE",
            ),
            (WORK, ROOT, "Work", None, "FOLDER"),
        ]);
        let server = serve(&drive).await;
        let made = paste(&server, &drive.item(trip), WORK).await.unwrap();
        assert_eq!(made.name, "Trip");
        assert_eq!(drive.names(WORK), ["Trip"]);
        assert_eq!(
            drive.names(&made.drivewsid),
            ["Days", "Plan 2.pdf", "Plan.pdf"]
        );
        let inner = drive
            .0
            .lock()
            .unwrap()
            .items
            .iter()
            .find(|i| i.parent == made.drivewsid && i.name == "Days")
            .unwrap()
            .id
            .clone();
        assert_eq!(drive.names(&inner), ["Day one.txt"]);
        // The originals are untouched.
        assert_eq!(drive.names(trip), ["Days", "Plan 2.pdf", "Plan.pdf"]);
        assert_eq!(drive.names(days), ["Day one.txt"]);

        // Into the folder it is in: "Trip copy", and never into the copy being made.
        let again = paste(&server, &drive.item(trip), ROOT).await.unwrap();
        assert_eq!(again.name, "Trip copy");
        assert_eq!(drive.names(ROOT), ["Trip", "Trip copy", "Work"]);
        assert_eq!(
            drive.names(&again.drivewsid),
            ["Days", "Plan 2.pdf", "Plan.pdf"]
        );
        // Into itself, it never begins.
        assert!(paste(&server, &drive.item(trip), trip).await.is_err());
    }

    /// A paste that fails part-way leaves nothing of itself behind: the copy goes to Recently
    /// Deleted again, and Apple's refusal is the answer.
    #[tokio::test]
    async fn a_failed_paste_takes_its_copy_away_again() {
        let drive = FakeDrive::new(&[
            (REPORT, ROOT, "Report", Some("pdf"), "FILE"),
            (WORK, ROOT, "Work", None, "FOLDER"),
        ]);
        drive.0.lock().unwrap().refuse_moves = true;
        let server = serve(&drive).await;
        assert!(paste(&server, &drive.item(REPORT), WORK).await.is_err());
        assert_eq!(drive.names(ROOT), ["Report.pdf", "Work"]);
        assert!(drive.names(WORK).is_empty());
        assert_eq!(drive.writes().last().unwrap(), "trash Report 2.pdf");
    }

    /// Only files and folders: an app's own library is not copied.
    #[tokio::test]
    async fn an_app_library_is_not_copied() {
        let drive = FakeDrive::new(&[(
            "APPLIBRARY::com.apple.Keynote",
            ROOT,
            "Keynote",
            None,
            "APP_LIBRARY",
        )]);
        let server = serve(&drive).await;
        let keynote = drive.item("APPLIBRARY::com.apple.Keynote");
        assert!(paste(&server, &keynote, WORK).await.is_err());
        assert!(drive.writes().is_empty());
    }
}

#[cfg(test)]
mod delete_as_shown_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    const ROOT_ID: &str = "FOLDER::com.apple.CloudDocs::root";
    const WORK: &str = "FOLDER::com.apple.CloudDocs::work";

    /// Drive as Apple keeps it: every item with its etag, a folder's etag moving on whenever
    /// something lands in it, and a delete refused (ETAG_CONFLICT) unless it names the etag
    /// the item has now.
    #[derive(Clone, Default)]
    struct Apple(Arc<Mutex<Vec<Held>>>);

    /// An item as Apple keeps it: its id, its folder, its kind, its etag.
    type Held = (String, String, &'static str, String);

    impl Apple {
        fn new() -> Self {
            let a = Apple::default();
            a.put(WORK, ROOT_ID, "FOLDER", "w1");
            a.put("FILE::com.apple.CloudDocs::a", WORK, "FILE", "a1");
            a.put("FILE::com.apple.CloudDocs::b", WORK, "FILE", "b1");
            a.put("FILE::com.apple.CloudDocs::plan", ROOT_ID, "FILE", "p1");
            a
        }
        fn put(&self, id: &str, parent: &str, kind: &'static str, etag: &str) {
            self.0
                .lock()
                .unwrap()
                .push((id.into(), parent.into(), kind, etag.into()));
        }
        /// A file added on another device: it lands, and its folder's etag moves on.
        fn added_elsewhere(&self, id: &str) {
            self.put(id, WORK, "FILE", "n1");
            self.bump(WORK);
        }
        fn bump(&self, id: &str) {
            for i in self.0.lock().unwrap().iter_mut().filter(|i| i.0 == id) {
                i.3.push('+');
            }
        }
        fn has(&self, id: &str) -> bool {
            self.0.lock().unwrap().iter().any(|i| i.0 == id)
        }
        fn item(&self, id: &str) -> DriveItem {
            let all = self.0.lock().unwrap();
            let i = all.iter().find(|i| i.0 == id).unwrap();
            DriveItem {
                drivewsid: i.0.clone(),
                parent_id: Some(i.1.clone()),
                name: i.0.rsplit("::").next().unwrap().into(),
                extension: None,
                kind: i.2.into(),
                size: None,
                child_count: None,
                created_at: None,
                modified_at: None,
                etag: Some(i.3.clone()),
            }
        }
        fn children(&self, folder: &str) -> Vec<DriveItem> {
            let ids: Vec<String> = self
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|i| i.1 == folder)
                .map(|i| i.0.clone())
                .collect();
            ids.iter().map(|id| self.item(id)).collect()
        }
    }

    impl Respond for Apple {
        fn respond(&self, req: &Request) -> ResponseTemplate {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            if req.url.path().ends_with("/retrieveItemDetailsInFolders") {
                let folder = body[0]["drivewsid"].as_str().unwrap_or_default();
                let items: Vec<Value> = self
                    .children(folder)
                    .iter()
                    .map(|i| {
                        json!({ "drivewsid": i.drivewsid, "parentId": folder, "name": i.name,
                                     "type": i.kind, "etag": i.etag })
                    })
                    .collect();
                return ResponseTemplate::new(200)
                    .set_body_json(json!([{ "drivewsid": folder, "items": items }]));
            }
            if req.url.path().ends_with("/moveItemsToTrash") {
                let id = body["items"][0]["drivewsid"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let sent = body["items"][0]["etag"].as_str().map(str::to_string);
                let now = self.has(&id).then(|| self.item(&id).etag).flatten();
                if sent != now {
                    return ResponseTemplate::new(200).set_body_json(
                        json!({ "items": [{ "drivewsid": id, "status": "ETAG_CONFLICT", "etag": now }] }),
                    );
                }
                let mut all = self.0.lock().unwrap();
                all.retain(|i| i.0 != id && i.1 != id);
                return ResponseTemplate::new(200)
                    .set_body_json(json!({ "items": [{ "drivewsid": id, "status": "OK" }] }));
            }
            ResponseTemplate::new(404)
        }
    }

    async fn serve(apple: &Apple) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(apple.clone())
            .mount(&server)
            .await;
        server
    }

    /// Nothing changed since it was shown: deleted at once, no question.
    #[tokio::test]
    async fn an_unchanged_folder_is_deleted_at_once() {
        let apple = Apple::new();
        let (work, shown) = (apple.item(WORK), apple.children(WORK));
        let server = serve(&apple).await;
        let done = delete_as_shown(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &work,
            &shown,
            None,
        )
        .await
        .unwrap();
        assert_eq!(done, Removed::Done);
        assert!(!apple.has(WORK));
    }

    /// A file added on another device since the folder was shown: nothing is deleted, and the
    /// folder as it now stands comes back to ask about. Asked and answered for that version,
    /// it goes; but if it moved on again meanwhile, it is asked about again.
    #[tokio::test]
    async fn a_folder_with_a_file_added_elsewhere_is_asked_about() {
        let apple = Apple::new();
        let (work, shown) = (apple.item(WORK), apple.children(WORK));
        apple.added_elsewhere("FILE::com.apple.CloudDocs::new");
        let server = serve(&apple).await;
        let http = reqwest::Client::new();
        let asked = delete_as_shown(&http, &server.uri(), vec![], &work, &shown, None)
            .await
            .unwrap();
        let Removed::Changed(now) = asked else {
            panic!("deleted a folder holding a file never shown: {asked:?}")
        };
        assert_eq!((now.items, now.added, now.changed, now.gone), (3, 1, 0, 0));
        assert!(apple.has(WORK) && apple.has("FILE::com.apple.CloudDocs::new"));

        // Another lands before the answer: asked again, still nothing deleted.
        apple.added_elsewhere("FILE::com.apple.CloudDocs::newer");
        let again = delete_as_shown(
            &http,
            &server.uri(),
            vec![],
            &work,
            &shown,
            now.etag.as_deref(),
        )
        .await
        .unwrap();
        let Removed::Changed(now) = again else {
            panic!("deleted past a second change: {again:?}")
        };
        assert_eq!((now.items, now.added), (4, 2));
        assert!(apple.has(WORK));

        // "Delete anyway", for the version just shown: gone.
        let done = delete_as_shown(
            &http,
            &server.uri(),
            vec![],
            &work,
            &shown,
            now.etag.as_deref(),
        )
        .await
        .unwrap();
        assert_eq!(done, Removed::Done);
        assert!(!apple.has(WORK));
    }

    /// A file changed on another device since it was shown: Apple refuses the delete against
    /// the version shown, and that is said — the newer file stays.
    #[tokio::test]
    async fn a_file_changed_elsewhere_is_not_deleted() {
        let apple = Apple::new();
        let plan = apple.item("FILE::com.apple.CloudDocs::plan");
        apple.bump("FILE::com.apple.CloudDocs::plan");
        let server = serve(&apple).await;
        let refused = delete_as_shown(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &plan,
            &[],
            None,
        )
        .await;
        match refused {
            Err(Error::Refused(words)) => {
                assert!(words.contains("changed on another device"), "{words}")
            }
            other => panic!("deleted a newer file: {other:?}"),
        }
        assert!(apple.has("FILE::com.apple.CloudDocs::plan"));
        // As shown, it goes.
        let now = apple.item("FILE::com.apple.CloudDocs::plan");
        let done = delete_as_shown(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &now,
            &[],
            None,
        )
        .await
        .unwrap();
        assert_eq!(done, Removed::Done);
    }

    #[test]
    fn a_folders_changes_are_counted_against_what_was_shown() {
        let apple = Apple::new();
        let shown = apple.children(WORK);
        apple.bump("FILE::com.apple.CloudDocs::a");
        apple.added_elsewhere("FILE::com.apple.CloudDocs::c");
        apple
            .0
            .lock()
            .unwrap()
            .retain(|i| i.0 != "FILE::com.apple.CloudDocs::b");
        let now = folder_now(&shown, &apple.children(WORK), None);
        assert_eq!((now.items, now.added, now.changed, now.gone), (2, 1, 1, 1));
    }
}
