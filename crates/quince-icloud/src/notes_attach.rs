//! A picture or file added to a note, as a phone adds one.
//!
//! Measured on his account (read-only, `probe/examples/notes_attach_locked_shape.rs`): a
//! picture added on his iPhone is three things written together.
//!
//! * A `Media` record holding the file: `Asset` (ASSETID — `fileChecksum`, `receipt`,
//!   `referenceChecksum`, `size`, `wrappingKey`), `FilenameEncrypted` (the name, plain UTF-8),
//!   `Deleted` 0 and `MinimumSupportedNotesVersion` — the note's own (14 on a new note, 0 on an
//!   old one: it matched the note every time). Its record `parent` is the note.
//! * An `Attachment` record: `UTI` (STRING) and `UTIEncrypted` (the same, plain bytes), `Note`
//!   and `Media` (REFERENCE, VALIDATE), `CreationDate` / `LastModificationDate`, `FileSize`,
//!   `Deleted` 0, the same minimum version, `Orientation` 0, `ImageFilterType` 0,
//!   `HandwritingSummaryVersion` 0 and the eight `CroppingQuad…` corners of the unit square;
//!   a picture also `Width` / `Height` (DOUBLE), a file its name in `TitleEncrypted`. Its
//!   record `parent` is the note too.
//! * The note's document, with one U+FFFC where the attachment sits and a run of its own for
//!   it (see [`crate::ckdocument::insert_attachment`]).
//!
//! What the phone adds that is left out here, on purpose: preview renderings
//! (`PreviewImages` and their sizes), OCR / image-classification summaries and their version
//! numbers, and `ReplicaIDToNotesVersionDataEncrypted`. All are worked out from the file by a
//! device — a version number written without its summary would tell the phone the work is done
//! when it is not — and the last is the server's to keep (see `document_like_apples`).
//!
//! The three go in one atomic `records/modify`, the note's update first (see where it is
//! sent), the note's part written only over the exact
//! version it was built from: if a phone edited the note meanwhile, nothing is written, the
//! note is read again, the attachment is placed again — moved along with the other device's
//! edit — and the three are sent again. The file is uploaded once, before any of that.
//!
//! Switched on by [`NOTE_ATTACH_WRITES`].

use crate::cloudkit::ZoneClient;
use crate::{Error, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::Serialize;
use serde_json::{json, Value};

/// Whether adding a file to a note may write to iCloud at all. On: a picture added here to a
/// throwaway note (2026-10-22, supervised, on his account) showed on his iPhone on a line of
/// its own, the note's other attachments and table untouched, with no conflict copy.
pub const NOTE_ATTACH_WRITES: bool = true;

const CONTAINER: &str = "com.apple.notes";
const ZONE: &str = "Notes";

/// The largest file put in a note here.
pub const MOST_BYTES: u64 = 100 * 1024 * 1024;

/// What was added, to show it at once.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attached {
    pub attachment: String,
    pub media: String,
    pub uti: String,
    /// Where it landed, in UTF-16 units of the text Apple now holds.
    pub offset: u64,
    /// The note's text as written, with the attachment's character in it.
    pub body: String,
    /// How that text is set, as written — Apple's runs, the attachment's among them — so the
    /// note can be kept as Apple now holds it without reading it back.
    pub runs: Vec<crate::ckdocument::Run>,
}

/// The type identifier Notes records for a file, from its name.
#[must_use]
pub fn uti_for(name: &str) -> &'static str {
    let ext = std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "public.jpeg",
        "png" => "public.png",
        "heic" => "public.heic",
        "heif" => "public.heif",
        "gif" => "com.compuserve.gif",
        "tif" | "tiff" => "public.tiff",
        "webp" => "org.webmproject.webp",
        "pdf" => "com.adobe.pdf",
        "txt" => "public.plain-text",
        "md" => "net.daringfireball.markdown",
        "csv" => "public.comma-separated-values-text",
        "json" => "public.json",
        "rtf" => "public.rtf",
        "html" | "htm" => "public.html",
        "zip" => "public.zip-archive",
        "mp3" => "public.mp3",
        "m4a" => "com.apple.m4a-audio",
        "wav" => "com.microsoft.waveform-audio",
        "mp4" => "public.mpeg-4",
        "mov" => "com.apple.quicktime-movie",
        "docx" => "org.openxmlformats.wordprocessingml.document",
        "xlsx" => "org.openxmlformats.spreadsheetml.sheet",
        "pptx" => "org.openxmlformats.presentationml.presentation",
        "pages" => "com.apple.iwork.pages.sffpages",
        "numbers" => "com.apple.iwork.numbers.sffnumbers",
        "key" => "com.apple.iwork.keynote.sffkey",
        _ => "public.data",
    }
}

/// A picture's pixel size, read from its header: PNG, JPEG, GIF, WebP and HEIC.
#[must_use]
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let be16 = |i: usize| {
        Some(u32::from(u16::from_be_bytes(
            b.get(i..i + 2)?.try_into().ok()?,
        )))
    };
    let le16 = |i: usize| {
        Some(u32::from(u16::from_le_bytes(
            b.get(i..i + 2)?.try_into().ok()?,
        )))
    };
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some((be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF8") {
        return Some((le16(6)?, le16(8)?));
    }
    if b.starts_with(b"\xff\xd8") {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xFF {
                return None;
            }
            let marker = b[i + 1];
            if marker == 0xFF {
                i += 1;
                continue;
            }
            // Start-of-frame markers, not DHT (C4), JPG (C8) or DAC (CC).
            if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
                return Some((be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + usize::try_from(be16(i + 2)?).ok()?;
        }
        return None;
    }
    if b.len() > 30 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        let le24 = |i: usize| -> Option<u32> {
            let s = b.get(i..i + 3)?;
            Some(u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16)
        };
        return match &b[12..16] {
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
            b"VP8L" => {
                let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
            }
            _ => None,
        };
    }
    if b.get(4..8) == Some(b"ftyp") {
        // HEIF: the largest `ispe` (image spatial extent) is the picture; the rest are
        // thumbnails and tiles.
        let mut best: Option<(u32, u32)> = None;
        let mut i = 0;
        while let Some(at) = b[i..].windows(4).position(|w| w == b"ispe") {
            let p = i + at + 4;
            if let (Some(w), Some(h)) = (be32(p + 4), be32(p + 8)) {
                if best.is_none_or(|(bw, bh)| {
                    u64::from(w) * u64::from(h) > u64::from(bw) * u64::from(bh)
                }) {
                    best = Some((w, h));
                }
            }
            i = p;
        }
        return best;
    }
    None
}

fn int(value: i64) -> Value {
    json!({ "type": "INT64", "value": value })
}

fn double(value: f64) -> Value {
    json!({ "type": "DOUBLE", "value": value })
}

fn bare(text: &str) -> Value {
    json!({ "type": "ENCRYPTED_BYTES", "value": B64.encode(text.as_bytes()) })
}

/// The `Media` record's fields, as a phone writes them.
#[must_use]
pub fn media_fields(asset: &Value, filename: &str, min_version: i64) -> Value {
    json!({
        "Asset": { "type": "ASSETID", "value": asset },
        "FilenameEncrypted": bare(filename),
        "Deleted": int(0),
        "MinimumSupportedNotesVersion": int(min_version),
    })
}

/// What an attachment's record says about the file.
pub struct Described<'a> {
    pub uti: &'a str,
    pub filename: &'a str,
    pub size: u64,
    pub pixels: Option<(u32, u32)>,
    pub min_version: i64,
    pub stamp_ms: i64,
}

/// The `Attachment` record's fields, as a phone writes them (see the module notes for what is
/// left out and why).
#[must_use]
pub fn attachment_fields(note: Value, media: Value, d: &Described<'_>) -> Value {
    let mut f = json!({
        "UTI": { "type": "STRING", "value": d.uti },
        "UTIEncrypted": bare(d.uti),
        "Note": note,
        "Media": media,
        "CreationDate": { "type": "TIMESTAMP", "value": d.stamp_ms },
        "LastModificationDate": { "type": "TIMESTAMP", "value": d.stamp_ms },
        "FileSize": int(i64::try_from(d.size).unwrap_or(i64::MAX)),
        "Deleted": int(0),
        "MinimumSupportedNotesVersion": int(d.min_version),
        "Orientation": int(0),
        "ImageFilterType": int(0),
        "HandwritingSummaryVersion": int(0),
        "CroppingQuadBottomLeftX": double(0.0),
        "CroppingQuadBottomLeftY": double(0.0),
        "CroppingQuadBottomRightX": double(1.0),
        "CroppingQuadBottomRightY": double(0.0),
        "CroppingQuadTopLeftX": double(0.0),
        "CroppingQuadTopLeftY": double(1.0),
        "CroppingQuadTopRightX": double(1.0),
        "CroppingQuadTopRightY": double(1.0),
    });
    match d.pixels {
        Some((w, h)) => {
            f["Width"] = double(f64::from(w));
            f["Height"] = double(f64::from(h));
        }
        None => f["TitleEncrypted"] = bare(d.filename),
    }
    f
}

/// Where the attachment goes in the note as Apple holds it now (`current`, the field's
/// base64), and the document with it there.
///
/// `at` is in UTF-16 units of `base` — the text the person was looking at — when given, and
/// of Apple's text otherwise. When Apple's text has moved on since `base`, the place moves with
/// it, by the same merge a save uses; a place that cannot be carried across is refused rather
/// than guessed.
pub fn place_in_document(
    current: &str,
    base: Option<&str>,
    at: u64,
    id: &str,
    uti: &str,
    stamp: u64,
) -> Result<(String, u64)> {
    let moved = || {
        Error::Refused(
            "This note changed on another device right where the file was going, so nothing was added. Try again."
                .into(),
        )
    };
    // `at` and `base` are the text as shown, its lines as the phone draws them.
    let current = crate::ckdocument::with_apple_line_breaks_in_base64(current);
    let current = current.as_ref();
    let base = base.map(crate::ckdocument::apple_line_breaks);
    let base = base.as_deref();
    let theirs = crate::ckdocument::stored_text_from_base64(current).ok_or_else(|| {
        Error::Refused("Skykeep couldn't read this note's text as Apple holds it.".into())
    })?;
    let place = match base {
        Some(base) if base != theirs => {
            let mut mine: Vec<u16> = base.encode_utf16().collect();
            let i = usize::try_from(at).map_err(|_| moved())?;
            if i > mine.len() {
                return Err(moved());
            }
            mine.insert(i, 0xFFFC);
            let mine = String::from_utf16(&mine).map_err(|_| moved())?;
            let merged = super::merge_edits(base, &mine, &theirs).ok_or_else(moved)?;
            let (t, m): (Vec<u16>, Vec<u16>) = (
                theirs.encode_utf16().collect(),
                merged.encode_utf16().collect(),
            );
            let p = t.iter().zip(&m).take_while(|(a, b)| a == b).count();
            // The merge must be Apple's text with exactly our one character added.
            if m.len() != t.len() + 1 || m[p] != 0xFFFC || m[p + 1..] != t[p..] {
                return Err(moved());
            }
            p as u64
        }
        _ => at,
    };
    // On a paragraph of its own, as a phone places one: `place` moves to where its character
    // landed, past the newline put before it when it went in mid-line.
    let (document, place) = crate::ckdocument::insert_attachment_in_base64(
        current, place, id, uti, stamp,
    )
    .ok_or_else(|| {
        Error::Refused(
            "Skykeep can’t safely add a file to this note yet, so nothing was changed.".into(),
        )
    })?;
    super::fits_inline(&document)?;
    if !crate::ckdocument::attachments_sound_adding_in_base64(current, &document, id) {
        return Err(Error::Refused(super::ATTACHMENTS_UNSOUND.into()));
    }
    Ok((document, place))
}

/// CloudKit's query parameters, as [`ZoneClient`] sends them.
fn ck_query(params: &[(String, String)]) -> Vec<(String, String)> {
    let mut q = params.to_vec();
    q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
    q.push(("ckjsVersion".into(), "2.6.1".into()));
    q
}

async fn post(
    http: &reqwest::Client,
    url: &str,
    params: &[(String, String)],
    body: &Value,
) -> Result<Value> {
    for tries in 1..=3u64 {
        let response = http
            .post(url)
            .query(&ck_query(params))
            .json(body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        let busy = matches!(status.as_u16(), 429 | 503)
            || status.as_u16() == 409
                && (text.contains("\"ZONE_BUSY\"") || text.contains("\"TRY_AGAIN_LATER\""));
        if busy {
            tokio::time::sleep(std::time::Duration::from_secs(2 * tries)).await;
            continue;
        }
        if !status.is_success() {
            crate::approval_needed(status, &text)?;
            return Err(Error::Protocol(format!(
                "CloudKit refused it ({status}): {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        return serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("CloudKit returned invalid JSON: {e}")));
    }
    Err(Error::Busy)
}

fn database(root: &str, op: &str) -> String {
    format!(
        "{}/database/1/{CONTAINER}/production/private/{op}",
        root.trim_end_matches('/')
    )
}

/// The file's bytes into CloudKit for `Media/<media>.Asset`, checked: Apple's receipt must
/// name exactly the bytes sent. Answers the ASSETID value a record carries.
async fn upload_asset(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    media: &str,
    bytes: Vec<u8>,
) -> Result<Value> {
    let offered = post(
        http,
        &database(root, "assets/upload"),
        params,
        &json!({
            "zoneID": { "zoneName": ZONE, "zoneType": "REGULAR_CUSTOM_ZONE" },
            "tokens": [{ "recordType": "Media", "recordName": media, "fieldName": "Asset" }],
        }),
    )
    .await?;
    let url = offered
        .pointer("/tokens/0/url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol("CloudKit offered nowhere to put the file".into()))?
        .to_string();
    let size = bytes.len() as u64;
    let mut sum = crate::drive::AppleChecksum::default();
    sum.update(&bytes);
    let local = sum.finish();
    let sent = http
        .post(&url)
        .header("content-type", "application/octet-stream")
        .body(bytes)
        .send()
        .await?;
    let status = sent.status();
    let text = sent.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(Error::Protocol(format!("the upload answered {status}")));
    }
    let receipt: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("the upload's receipt was not JSON: {e}")))?;
    let single = receipt
        .get("singleFile")
        .filter(|s| s.get("receipt").is_some())
        .ok_or_else(|| Error::Protocol("the upload gave no receipt".into()))?;
    if single.get("size").and_then(Value::as_u64) != Some(size)
        || single.get("fileChecksum").and_then(Value::as_str) != Some(local.as_str())
    {
        return Err(Error::Protocol(
            "the bytes Apple received are not the file's, so nothing was added".into(),
        ));
    }
    let of = |k: &str| single.get(k).cloned().unwrap_or(Value::Null);
    Ok(json!({
        "fileChecksum": of("fileChecksum"),
        "receipt": of("receipt"),
        "referenceChecksum": of("referenceChecksum"),
        "size": size,
        "wrappingKey": of("wrappingKey"),
    }))
}

/// Add the file at `path` to note `guid` at UTF-16 offset `at` of `base` (or of Apple's text).
pub async fn attach_file_cloudkit(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    guid: &str,
    path: &std::path::Path,
    base: Option<&str>,
    at: u64,
) -> Result<Attached> {
    if !NOTE_ATTACH_WRITES {
        return Err(Error::Refused(
            "Adding files to notes isn’t switched on yet. Nothing was changed.".into(),
        ));
    }
    attach_file_unguarded(http, ckdatabasews_root, params, guid, path, base, at).await
}

async fn attach_file_unguarded(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    guid: &str,
    path: &std::path::Path,
    base: Option<&str>,
    at: u64,
) -> Result<Attached> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::Refused("That file has no name to give it.".into()))?
        .to_string();
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| Error::Refused(format!("Couldn’t read {filename}: {e}")))?;
    if meta.is_dir() {
        return Err(Error::Refused(format!(
            "{filename} is a folder. A note takes files, so add the files inside it."
        )));
    }
    let size = meta.len();
    if size == 0 || size > MOST_BYTES {
        return Err(Error::Refused(format!(
            "{filename} is {} — a note takes files up to {} MB.",
            if size == 0 {
                "empty".to_string()
            } else {
                format!("{} MB", size / 1_048_576)
            },
            MOST_BYTES / 1_048_576
        )));
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| Error::Refused(format!("Couldn’t read {filename}: {e}")))?;
    let uti = uti_for(&filename);
    let pixels = if crate::note_media::kind_of(uti) == "image" {
        image_size(&bytes)
    } else {
        None
    };

    let client = ZoneClient::new(http, root, CONTAINER, params.clone());
    let attachment = uuid::Uuid::new_v4().to_string().to_uppercase();
    let media = uuid::Uuid::new_v4().to_string().to_uppercase();
    // The note asked after while the file goes up, not before it: one round trip fewer on
    // every file added. A note that is gone still adds nothing — the upload alone is never
    // part of any note, and the loop below looks the note up again before writing.
    let (there, asset) = tokio::join!(
        client.lookup_record(ZONE, guid),
        upload_asset(http, root, &params, &media, bytes)
    );
    if there?.is_none() {
        return Err(Error::Refused("Apple no longer has that note.".into()));
    }
    let asset = asset?;

    for _ in 0..4 {
        let note = client
            .lookup_record(ZONE, guid)
            .await?
            .ok_or_else(|| Error::Refused("Apple no longer has that note.".into()))?;
        let current = note
            .pointer("/fields/TextDataEncrypted/value")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Refused("That note has no body Apple will let us read.".into())
            })?;
        let tag = note
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .ok_or(Error::Conflict)?;
        let now_ms = super::now_millis();
        let stamp = u64::try_from(now_ms / 1000).unwrap_or(0);
        let (document, offset) = place_in_document(current, base, at, &attachment, uti, stamp)?;
        let min_version = note
            .pointer("/fields/MinimumSupportedNotesVersion/value")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let described = Described {
            uti,
            filename: &filename,
            size,
            pixels,
            min_version,
            stamp_ms: now_ms,
        };
        let parent = json!({ "recordName": guid });
        // The note first: CloudKit takes a new record's `parent` as touching the note, and
        // refuses a batch that then updates it ("Record updated multiple times in one batch",
        // since November 2026). Written first, the same three still go atomically.
        let operations = json!([
            { "operationType": "update", "record": {
                "recordName": guid, "recordType": "Note", "recordChangeTag": tag,
                "fields": {
                    "TextDataEncrypted": { "type": "ENCRYPTED_BYTES", "value": document },
                    "ModificationDate": { "type": "TIMESTAMP", "value": now_ms },
                },
            } },
            { "operationType": "create", "record": {
                "recordName": media, "recordType": "Media", "parent": parent,
                "fields": media_fields(&asset, &filename, min_version),
            } },
            { "operationType": "create", "record": {
                "recordName": attachment, "recordType": "Attachment", "parent": parent,
                "fields": attachment_fields(
                    client.reference_to(ZONE, guid),
                    client.reference_to(ZONE, &media),
                    &described,
                ),
            } },
        ]);
        let answer = post(
            http,
            &database(root, "records/modify"),
            &params,
            &json!({
                "operations": operations,
                "zoneID": { "zoneName": ZONE, "zoneType": "REGULAR_CUSTOM_ZONE" },
                "atomic": true,
            }),
        )
        .await?;
        match outcome(&answer) {
            Ok(()) => {
                let body =
                    crate::ckdocument::stored_text_from_base64(&document).unwrap_or_default();
                let runs = crate::ckdocument::runs_from_base64(&document);
                return Ok(Attached {
                    attachment,
                    media,
                    uti: uti.to_string(),
                    offset,
                    body,
                    runs,
                });
            }
            // The note moved (or the zone was busy): nothing was written; place it again.
            Err(Error::Conflict | Error::Busy) => {}
            Err(e) => return Err(e),
        }
    }
    Err(Error::Refused(
        "This note keeps changing on another device, so the file wasn’t added yet. Try again in a moment.".into(),
    ))
}

/// What an atomic `records/modify` came to: every record written, the note moved under us
/// (Conflict), the zone busy, or refused.
fn outcome(answer: &Value) -> Result<()> {
    let records = answer
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Protocol("CloudKit didn’t say what it wrote".into()))?;
    if records.len() != 3 {
        return Err(Error::Protocol(
            "CloudKit didn’t answer for every record".into(),
        ));
    }
    let mut failure: Option<Error> = None;
    for r in records {
        let Some(reason) = r.get("reason").and_then(Value::as_str) else {
            continue;
        };
        let code = r
            .get("serverErrorCode")
            .and_then(Value::as_str)
            .unwrap_or("");
        let this = if code == "CONFLICT" || reason.to_ascii_lowercase().contains("oplock") {
            Error::Conflict
        } else if matches!(code, "ZONE_BUSY" | "TRY_AGAIN_LATER") {
            Error::Busy
        } else if code == "ATOMIC_ERROR" {
            // Failed only because another of the three did: that one says why.
            continue;
        } else {
            Error::Protocol(format!("CloudKit refused the attachment: {reason}"))
        };
        failure = Some(match (failure, this) {
            (Some(Error::Protocol(p)), _) | (_, Error::Protocol(p)) => Error::Protocol(p),
            (_, e) => e,
        });
    }
    match failure {
        None if records.iter().all(|r| r.get("recordChangeTag").is_some()) => Ok(()),
        None => Err(Error::Protocol(
            "CloudKit wrote only part of the attachment".into(),
        )),
        Some(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn type_identifiers_come_from_the_name() {
        assert_eq!(uti_for("IMG_1.JPG"), "public.jpeg");
        assert_eq!(uti_for("a.heic"), "public.heic");
        assert_eq!(uti_for("report.pdf"), "com.adobe.pdf");
        assert_eq!(
            uti_for("sheet.xlsx"),
            "org.openxmlformats.spreadsheetml.sheet"
        );
        assert_eq!(uti_for("noext"), "public.data");
        assert_eq!(crate::note_media::kind_of(uti_for("x.png")), "image");
    }

    #[test]
    fn picture_sizes_are_read_from_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((640, 480)));
        let gif = b"GIF89a\x20\x03\x58\x02";
        assert_eq!(image_size(gif), Some((800, 600)));
        // SOI, an APP0 of length 4, then SOF0: precision, height 300, width 400.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 17, 8, 0x01, 0x2C, 0x01, 0x90, 3, 0,
            0, 0, 0,
        ];
        assert_eq!(image_size(&jpeg), Some((400, 300)));
        let mut heic = b"\0\0\0\x18ftypheic".to_vec();
        for (w, h) in [(512u32, 512u32), (4032, 3024)] {
            heic.extend(b"\0\0\0\x14ispe\0\0\0\0");
            heic.extend(w.to_be_bytes());
            heic.extend(h.to_be_bytes());
        }
        assert_eq!(image_size(&heic), Some((4032, 3024)));
        assert_eq!(image_size(b"not a picture"), None);
    }

    /// The records carry the fields measured on a phone's picture, and only those.
    #[test]
    fn records_are_shaped_like_the_phones() {
        let d = Described {
            uti: "public.jpeg",
            filename: "cat.jpg",
            size: 1234,
            pixels: Some((585, 90)),
            min_version: 14,
            stamp_ms: 1_792_549_727_010,
        };
        let f = attachment_fields(json!("NOTE-REF"), json!("MEDIA-REF"), &d);
        let mut names: Vec<&String> = f.as_object().unwrap().keys().collect();
        names.sort();
        assert_eq!(
            names,
            [
                "CreationDate",
                "CroppingQuadBottomLeftX",
                "CroppingQuadBottomLeftY",
                "CroppingQuadBottomRightX",
                "CroppingQuadBottomRightY",
                "CroppingQuadTopLeftX",
                "CroppingQuadTopLeftY",
                "CroppingQuadTopRightX",
                "CroppingQuadTopRightY",
                "Deleted",
                "FileSize",
                "HandwritingSummaryVersion",
                "Height",
                "ImageFilterType",
                "LastModificationDate",
                "Media",
                "MinimumSupportedNotesVersion",
                "Note",
                "Orientation",
                "UTI",
                "UTIEncrypted",
                "Width",
            ]
        );
        assert_eq!(f["UTIEncrypted"]["value"], B64.encode("public.jpeg"));
        assert_eq!(f["Width"], json!({ "type": "DOUBLE", "value": 585.0 }));
        assert_eq!(f["MinimumSupportedNotesVersion"]["value"], 14);
        let file = attachment_fields(
            json!(0),
            json!(0),
            &Described {
                pixels: None,
                uti: "com.adobe.pdf",
                filename: "a.pdf",
                ..d
            },
        );
        assert!(file.get("Width").is_none());
        assert_eq!(file["TitleEncrypted"]["value"], B64.encode("a.pdf"));
        let m = media_fields(&json!({ "receipt": "r" }), "cat.jpg", 14);
        assert_eq!(m["FilenameEncrypted"]["value"], B64.encode("cat.jpg"));
        assert_eq!(m["Asset"]["type"], "ASSETID");
    }

    const ID: &str = "11111111-2222-3333-4444-555555555555";

    /// The document change on a synthetic note: one U+FFFC where asked, on a paragraph of its
    /// own as a phone puts one, a run of its own naming the attachment, the guard satisfied —
    /// and the place carried across a phone's edit made meanwhile.
    #[test]
    fn the_attachment_lands_where_asked_even_after_an_edit_elsewhere() {
        let original = crate::ckdocument::text_to_base64("Shopping\nmilk\n");
        let (doc, at) = place_in_document(&original, None, 9, ID, "public.jpeg", 1).unwrap();
        assert_eq!(at, 9);
        assert_eq!(
            crate::ckdocument::stored_text_from_base64(&doc).unwrap(),
            "Shopping\n\u{FFFC}\nmilk\n"
        );
        let runs = crate::ckdocument::runs_from_base64(&doc);
        let mut from = 0;
        let mine: Vec<_> = runs
            .iter()
            .filter_map(|r| {
                from += r.length;
                r.attachment.as_deref().map(|a| (a, from - 1, r.length))
            })
            .collect();
        assert_eq!(mine, [(ID, 9, 1)]);
        assert_eq!(runs.iter().map(|r| r.length).sum::<u64>(), 16);
        assert!(crate::ckdocument::attachments_sound_adding_in_base64(
            &original, &doc, ID
        ));
        // The ordinary guard, which allows no new attachment, refuses it — the new one is
        // only ever allowed by name.
        assert!(!crate::ckdocument::attachments_sound_in_base64(
            &original, &doc
        ));
        // Once written, editing that note again is an ordinary, sound edit.
        let edited =
            crate::ckdocument::replace_text_in_base64(&doc, "Shopping!\n\u{FFFC}\nmilk\n").unwrap();
        assert!(crate::ckdocument::attachments_sound_in_base64(
            &doc, &edited
        ));

        // The phone added a line at the top since the person looked: the picture still goes
        // before "milk".
        let theirs = crate::ckdocument::text_to_base64("Today\nShopping\nmilk\n");
        let (doc, at) =
            place_in_document(&theirs, Some("Shopping\nmilk\n"), 9, ID, "public.jpeg", 1).unwrap();
        assert_eq!(at, 15);
        assert_eq!(
            crate::ckdocument::stored_text_from_base64(&doc).unwrap(),
            "Today\nShopping\n\u{FFFC}\nmilk\n"
        );
    }

    #[test]
    fn a_bad_place_or_a_reused_id_is_refused() {
        let original = crate::ckdocument::text_to_base64("ab");
        assert!(place_in_document(&original, None, 3, ID, "public.png", 1).is_err());
        // "😀" is two UTF-16 units; between them is no place at all.
        let emoji = crate::ckdocument::text_to_base64("😀");
        assert!(place_in_document(&emoji, None, 1, ID, "public.png", 1).is_err());
        let (doc, _) = place_in_document(&original, None, 2, ID, "public.png", 1).unwrap();
        // The same id a second time is one attachment in two places.
        assert!(place_in_document(&doc, None, 0, ID, "public.png", 1).is_err());
    }

    /// A picture goes on a paragraph of its own, as a phone puts one: mid-line it splits the
    /// line, at a line's end it goes on the line below, and the line it came from keeps its
    /// setting — a checklist item stays one, and the picture is not made one.
    #[test]
    fn the_attachment_gets_a_paragraph_of_its_own() {
        use crate::ckdocument::ParagraphChange;
        let plain = crate::ckdocument::text_to_base64("todo\nbuy\n");
        let listed = crate::ckdocument::set_paragraph_in_base64(
            &plain,
            5,
            ParagraphChange::Style(103),
            Some("buy"),
        )
        .unwrap();
        // At the end of "buy": below it, as the note's last line.
        let (doc, at) = place_in_document(&listed, None, 8, ID, "public.png", 1).unwrap();
        assert_eq!(at, 9);
        assert_eq!(
            crate::ckdocument::stored_text_from_base64(&doc).unwrap(),
            "todo\nbuy\n\u{FFFC}"
        );
        let runs = crate::ckdocument::runs_from_base64(&doc);
        let pic = runs.iter().find(|r| r.attachment.is_some()).unwrap();
        assert_eq!(pic.style, None);
        assert_eq!(pic.checked, None);
        let mut from = 0;
        let buy: Vec<_> = runs
            .iter()
            .filter(|r| {
                from += r.length;
                from > 5 && from <= 9
            })
            .collect();
        assert!(buy.iter().all(|r| r.style == Some(103)));
        assert_eq!(runs.iter().map(|r| r.length).sum::<u64>(), 10);

        // Mid-line: the line is split around it.
        let (doc, at) = place_in_document(&listed, None, 2, ID, "public.png", 1).unwrap();
        assert_eq!(at, 3);
        assert_eq!(
            crate::ckdocument::stored_text_from_base64(&doc).unwrap(),
            "to\n\u{FFFC}\ndo\nbuy\n"
        );
        let runs = crate::ckdocument::runs_from_base64(&doc);
        assert_eq!(runs.iter().map(|r| r.length).sum::<u64>(), 12);
        assert!(crate::ckdocument::attachments_sound_adding_in_base64(
            &listed, &doc, ID
        ));
    }

    fn modify_ok() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({ "records": [
            { "recordName": "M", "recordChangeTag": "a" },
            { "recordName": "A", "recordChangeTag": "b" },
            { "recordName": "N", "recordChangeTag": "c" },
        ] }))
    }

    /// The whole write against a stand-in CloudKit: the file uploaded once, then a conflict on
    /// the note (a phone edited it) sends nothing twice by mistake — the note is read again and
    /// the three records go again, atomically, the note over its new version.
    #[tokio::test]
    async fn a_conflict_reads_the_note_again_and_places_it_again() {
        let server = MockServer::start().await;
        let dir = std::env::temp_dir().join(format!("quince-attach-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("pic.png");
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(2u32.to_be_bytes());
        png.extend(3u32.to_be_bytes());
        std::fs::write(&file, &png).unwrap();
        let mut sum = crate::drive::AppleChecksum::default();
        sum.update(&png);
        let checksum = sum.finish();

        let note = |tag: &str, text: &str| {
            json!({ "records": [{ "recordName": "N", "recordType": "Note", "recordChangeTag": tag,
                "fields": { "TextDataEncrypted": { "type": "ENCRYPTED_BYTES",
                    "value": crate::ckdocument::text_to_base64(text) },
                    "MinimumSupportedNotesVersion": { "type": "INT64", "value": 14 } } }] })
        };
        // Lookups: before the upload, then once per attempt — the second attempt sees the
        // phone's edit.
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(note("t1", "a\nb")))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(note("t2", "zz\na\nb")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/assets/upload$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "tokens": [{
                "recordName": "M", "fieldName": "Asset", "url": format!("{}/put", server.uri()) }] })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/put$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "singleFile": {
                "fileChecksum": checksum, "receipt": "R", "referenceChecksum": "RC",
                "size": png.len(), "wrappingKey": "WK" } })),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "records": [
                { "recordName": "M", "serverErrorCode": "ATOMIC_ERROR", "reason": "atomic" },
                { "recordName": "A", "serverErrorCode": "ATOMIC_ERROR", "reason": "atomic" },
                { "recordName": "N", "serverErrorCode": "CONFLICT", "reason": "oplock" },
            ] })),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(modify_ok())
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let done = attach_file_unguarded(&http, &server.uri(), vec![], "N", &file, Some("a\nb"), 2)
            .await
            .unwrap();
        assert_eq!(done.body, "zz\na\n\u{FFFC}\nb");
        assert_eq!(done.offset, 5);

        let sent = server.received_requests().await.unwrap();
        let writes: Vec<Value> = sent
            .iter()
            .filter(|r| r.url.path().ends_with("/records/modify"))
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect();
        assert_eq!(writes.len(), 2);
        let last = &writes[1];
        assert_eq!(last["atomic"], true);
        let ops = last["operations"].as_array().unwrap();
        // The note first: after a new record naming it as `parent`, CloudKit refuses it.
        assert_eq!(ops[0]["record"]["recordType"], "Note");
        assert_eq!(ops[0]["record"]["recordChangeTag"], "t2");
        assert_eq!(ops[1]["record"]["recordType"], "Media");
        assert_eq!(ops[1]["record"]["parent"]["recordName"], "N");
        assert_eq!(ops[1]["record"]["fields"]["Asset"]["value"]["receipt"], "R");
        assert_eq!(ops[2]["record"]["recordType"], "Attachment");
        assert_eq!(ops[2]["record"]["parent"]["recordName"], "N");
        assert_eq!(ops[2]["record"]["fields"]["Width"]["value"], 2.0);
        assert_eq!(
            ops[2]["record"]["fields"]["MinimumSupportedNotesVersion"]["value"],
            14
        );
        // The same records, by the same names, both times: nothing made twice.
        for i in 1..3 {
            assert_eq!(
                writes[0]["operations"][i]["record"]["recordName"],
                ops[i]["record"]["recordName"]
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Off, nothing is sent at all.
    #[tokio::test]
    async fn switched_off_it_sends_nothing() {
        let server = MockServer::start().await;
        let http = reqwest::Client::new();
        let refused = attach_file_cloudkit(
            &http,
            &server.uri(),
            vec![],
            "N",
            std::path::Path::new("/nonexistent/x.png"),
            None,
            0,
        )
        .await;
        assert!(matches!(refused, Err(Error::Refused(_))));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
