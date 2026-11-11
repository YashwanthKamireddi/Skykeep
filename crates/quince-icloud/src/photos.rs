//! iCloud Photos: the library mirrored through CloudKit's change feed.
//!
//! Container `com.apple.photos.cloud`, zone `PrimarySync` (or, for some accounts, another
//! `PrimarySync*`: see [`library_zone`]). A photo is two records: a
//! `CPLAsset` (what the person sees: the date, the flags, edits) pointing by `masterRef` at a
//! `CPLMaster` (the original file and its renditions). Measured on his library: a full walk is
//! 273 records in eight seconds, and a pass from the token with nothing new is under half a
//! second. See docs/icloud-web/photos.md.

use crate::cloudkit::{field, Database, ZoneClient};
use crate::Result;
use base64::Engine;
use quince_core::photos::{Album, Kind, Photo};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const CONTAINER: &str = "com.apple.photos.cloud";
/// The zone most libraries live in, and the one used when Apple names no other.
pub const DEFAULT_ZONE: &str = "PrimarySync";

/// Zones already found, by [`ZoneClient::scope`]: asked of Apple once per account per run.
static ZONES: std::sync::Mutex<Option<HashMap<String, String>>> = std::sync::Mutex::new(None);

/// The zone among `names` that holds the library: `PrimarySync` where it is there, else
/// another `PrimarySync*` (Apple has moved some accounts' libraries to `PrimarySync1`,
/// icloud_photos_downloader#1367, and answers `ZONE_NOT_FOUND` for `PrimarySync`). `None` when
/// none is.
#[must_use]
pub fn pick_zone(names: &[String]) -> Option<&str> {
    names
        .iter()
        .find(|n| *n == DEFAULT_ZONE)
        .or_else(|| {
            let mut moved: Vec<&String> = names
                .iter()
                .filter(|n| n.starts_with(DEFAULT_ZONE))
                .collect();
            moved.sort();
            moved.into_iter().next()
        })
        .map(String::as_str)
}

/// The zone this account's library lives in, discovered from Apple's own list of zones
/// rather than assumed. Where the list cannot be had, or names no library zone,
/// [`DEFAULT_ZONE`]: what every account used until Apple began moving some. A failed list is
/// not remembered, so it is asked again next time; an answer is, for the rest of the run.
pub async fn library_zone(client: &ZoneClient<'_>) -> String {
    let scope = client.scope();
    if let Some(zone) = ZONES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(&scope).cloned())
    {
        return zone;
    }
    match client.zone_names().await {
        Ok(names) => {
            let zone = pick_zone(&names).unwrap_or(DEFAULT_ZONE).to_string();
            if zone != DEFAULT_ZONE {
                quiet!("photos: the library is in zone {zone}");
            }
            ZONES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_or_insert_with(HashMap::new)
                .insert(scope, zone.clone());
            zone
        }
        Err(e) => {
            quiet!("photos: couldn't list the zones ({e}); reading {DEFAULT_ZONE}");
            DEFAULT_ZONE.to_string()
        }
    }
}
/// The library's sync token, in the library's sync state.
pub const SERVICE: &str = "photos";

/// What one pass of the change feed brought.
pub struct PhotosFetch {
    pub records: Vec<Value>,
    /// Where the next pass carries on from. When the pass did not finish, this is a position
    /// partway through, and the next pass picks up from it rather than starting over.
    pub token: Option<String>,
    /// The position this pass began at, when it began at one Apple took.
    pub from: Option<String>,
    /// From a token: only what changed. Without one (none held, or the one held refused):
    /// a whole read from the beginning, and what it never sees is gone.
    pub incremental: bool,
    pub complete: bool,
    /// The library read: none for the personal one, else a Shared Library's zone.
    pub zone: Option<String>,
}

/// Where a library's progress is kept in the library's sync state: `photos` for the personal
/// library, `photos:<zone>` for a Shared Library.
#[must_use]
pub fn service_of(zone: Option<&str>) -> String {
    zone.map_or_else(|| SERVICE.to_string(), |z| format!("{SERVICE}:{z}"))
}

/// The zones Shared Libraries live in: `SharedSync-<id>`.
pub const SHARED_LIBRARY_PREFIX: &str = "SharedSync-";

/// Every Shared Library this account set up (its own database) or joined (the shared one,
/// with its owner): a `SharedSync-*` zone in either.
///
/// # Errors
/// When Apple cannot be reached or refuses the question.
pub async fn shared_libraries(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
) -> Result<Vec<crate::shared_collections::Place>> {
    let mut out = Vec::new();
    for database in [Database::Private, Database::Shared] {
        let ids = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params.clone())
            .in_database(database)
            .zone_ids()
            .await?;
        out.extend(
            ids.into_iter()
                .filter(|(name, _)| name.starts_with(SHARED_LIBRARY_PREFIX))
                .map(|(zone, owner)| crate::shared_collections::Place {
                    database,
                    zone,
                    owner,
                }),
        );
    }
    Ok(out)
}

/// How many pages of the change feed one fetch reads (500 records each). A library larger
/// than this is read over several, each carrying on where the last stopped and each stored
/// before the next is asked for: a hundred pages at once held some fifty thousand records in
/// memory together through a whole walk.
const PASS_PAGES: usize = 10;

/// How many fetches one refresh makes before it leaves the rest of a walk to the next: the
/// hundred pages a refresh has always gone through, ten at a time.
pub const PASS_ROUNDS: usize = 10;

/// A client for one library: the personal one (`place` none), or a Shared Library where
/// [`shared_libraries`] found it — in the shared database, under its owner, for one joined.
fn client_in<'a>(
    http: &'a reqwest::Client,
    ckdatabasews_root: &'a str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
) -> ZoneClient<'a> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    match place {
        None => client,
        Some(p) => client.in_database(p.database).owned_by(p.owner.clone()),
    }
}

/// The zone a library's records are in: a Shared Library's own, or the personal library's as
/// [`library_zone`] finds it.
async fn zone_in(
    client: &ZoneClient<'_>,
    place: Option<&crate::shared_collections::Place>,
) -> String {
    match place {
        Some(p) => p.zone.clone(),
        None => library_zone(client).await,
    }
}

/// One pass of the change feed from `since` (or from the start).
///
/// A `since` that Apple will not read any more (an expired or malformed change token) is
/// dropped and the pass reads from the start instead, and says so: that is a whole read,
/// not a pass over what changed.
pub async fn fetch(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    since: Option<&str>,
) -> Result<PhotosFetch> {
    fetch_in(http, ckdatabasews_root, params, None, since).await
}

/// [`fetch`] of one library: `place` none for the personal library, else a Shared Library
/// where [`shared_libraries`] found it.
///
/// # Errors
/// When Apple cannot be reached or refuses the read.
pub async fn fetch_in(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    since: Option<&str>,
) -> Result<PhotosFetch> {
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    let shared = place.map(|p| p.zone.clone());
    if let Some(from) = since.map(str::trim).filter(|t| !t.is_empty()) {
        match walk(&client, &zone, Some(from)).await {
            Ok((records, token, complete)) => {
                return Ok(PhotosFetch {
                    records,
                    token,
                    from: Some(from.to_string()),
                    incremental: true,
                    complete,
                    zone: shared,
                })
            }
            Err(e) if refused_token(&e) => {
                quiet!("photos: the change token was refused; reading from the start");
            }
            Err(e) => return Err(e),
        }
    }
    let (records, token, complete) = walk(&client, &zone, None).await?;
    Ok(PhotosFetch {
        records,
        token,
        from: None,
        incremental: false,
        complete,
        zone: shared,
    })
}

/// Whether Apple has the library ready to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// Apple is still preparing it — a library just turned on, or moved — and a read of it now
    /// would find it empty, or part of it. `progress` is Apple's percentage where it says.
    Preparing {
        progress: Option<u8>,
    },
}

/// What Apple's `CheckIndexingState` record says: `state` `FINISHED` is ready, anything else
/// (`RUNNING`, with a `progress` of 0 to 100) is still preparing (icloudpd #1387, #1388; the
/// same check icloud.com and pyicloud make before reading the library). No record, or none
/// with a state, says nothing: read as ready, as it always was.
#[must_use]
pub fn readiness_of(records: &[Value]) -> Readiness {
    let Some(fields) = records.iter().find_map(|r| r.get("fields")) else {
        return Readiness::Ready;
    };
    let Some(state) = fields.pointer("/state/value").and_then(Value::as_str) else {
        return Readiness::Ready;
    };
    if state.eq_ignore_ascii_case("FINISHED") {
        return Readiness::Ready;
    }
    let progress = fields
        .pointer("/progress/value")
        .and_then(|v| v.as_f64().or_else(|| v.as_str()?.trim().parse().ok()))
        .map(|p| p.clamp(0.0, 100.0) as u8);
    Readiness::Preparing { progress }
}

/// Ask Apple whether the library is ready to read ([`readiness_of`]).
///
/// # Errors
/// When Apple cannot be reached or refuses the question.
pub async fn readiness(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
) -> Result<Readiness> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    // Asked of the zone the library is in, as every read of it is (`library_zone`).
    let zone = library_zone(&client).await;
    let records = client
        .records_query(
            &zone,
            serde_json::json!({ "recordType": "CheckIndexingState" }),
        )
        .await?;
    Ok(readiness_of(&records))
}

/// Whether an error is Apple refusing the change token, rather than anything else.
///
/// The cloudkit layer names a refused continuation `StaleCursor`; an expired token
/// (`CHANGE_TOKEN_EXPIRED`) or a `BAD_REQUEST` about the token comes up as the zone refused,
/// with Apple's words, which speak of the token.
fn refused_token(e: &crate::Error) -> bool {
    match e {
        crate::Error::StaleCursor => true,
        crate::Error::Protocol(text) => {
            let text = text.to_lowercase();
            text.contains("token") || text.contains("continuation")
        }
        _ => false,
    }
}

/// Up to `PASS_PAGES` pages from `since`: the records, where to carry on from, and whether
/// the feed was read to its end.
async fn walk(
    client: &ZoneClient<'_>,
    zone: &str,
    since: Option<&str>,
) -> Result<(Vec<Value>, Option<String>, bool)> {
    let mut records = Vec::new();
    let mut token = since.map(str::to_string);
    for page_no in 0..PASS_PAGES {
        let page = client.changes(zone, token.as_deref()).await?;
        // A token answered with no position at all was not taken: an expired one can come
        // back as the zone's error code alone, which reads as an empty zone.
        if page_no == 0 && since.is_some() && page.sync_token.is_none() {
            return Err(crate::Error::StaleCursor);
        }
        records.extend(page.records);
        let (next, advanced) = crate::cloudkit::advance(token.take(), page.sync_token);
        token = next;
        if !page.more_coming {
            return Ok((records, token, true));
        }
        // "More coming" with no new position to carry on from would ask the same again.
        if !advanced {
            break;
        }
    }
    Ok((records, token, false))
}

/// Where each of a photo's pictures can be downloaded now. They expire: never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    pub thumb: Option<String>,
    pub medium: Option<String>,
    pub original: Option<String>,
    /// The whole picture as a JPEG, for zooming in: Apple's full-size rendition, or the
    /// original itself when that is already a JPEG or PNG (a HEIC original shows nowhere here).
    pub full: Option<String>,
    /// A video's playable rendition, or a Live Photo's moving part.
    pub video: Option<String>,
    /// A Live Photo's moving part as it was shot: the original `.MOV`.
    pub live: Option<String>,
}

fn link(fields: &Value, key: &str, name: &str) -> Option<String> {
    fields
        .pointer(&format!("/{key}/value/downloadURL"))
        .and_then(Value::as_str)
        .map(|u| u.replace("${f}", name))
}

/// A master's download links, as icloud.com takes them (`${f}` is the file name it asks for).
#[must_use]
pub fn links_of(master: &Value) -> Links {
    links_with_edit(None, master)
}

/// A photo's links, an edit's first: a photo cropped or filtered on an iPhone or a Mac keeps its
/// edited pictures on the asset (`resJPEG{Thumb,Med,Full}Res`), the master keeping the
/// original — as icloud.com shows and downloads it (`download_choice`). From the master alone,
/// every edited photo showed as shot, uncropped, in the grid, the viewer and the folder.
#[must_use]
pub fn links_with_edit(asset: Option<&Value>, master: &Value) -> Links {
    let mut l = master_links(master);
    let Some(a) = asset.map(|a| &a["fields"]) else {
        return l;
    };
    let edit = |key: &str, name: &str| link(a, key, name);
    if let Some(full) = edit("resJPEGFullRes", "full.jpg") {
        l.full = Some(full);
    }
    if let Some(med) =
        edit("resJPEGMedRes", "med.jpg").or_else(|| edit("resJPEGFullRes", "full.jpg"))
    {
        l.medium = Some(med);
    }
    if let Some(thumb) =
        edit("resJPEGThumbRes", "thumb.jpg").or_else(|| edit("resJPEGMedRes", "med.jpg"))
    {
        l.thumb = Some(thumb);
    }
    l
}

fn master_links(master: &Value) -> Links {
    let f = &master["fields"];
    Links {
        thumb: link(f, "resJPEGThumbRes", "thumb.jpg")
            .or_else(|| link(f, "resJPEGMedRes", "med.jpg")),
        medium: link(f, "resJPEGMedRes", "med.jpg")
            .or_else(|| link(f, "resJPEGFullRes", "full.jpg")),
        original: link(f, "resOriginalRes", "original"),
        full: link(f, "resJPEGFullRes", "full.jpg").or_else(|| {
            let kind = f.pointer("/itemType/value").and_then(Value::as_str);
            matches!(kind, Some("public.jpeg" | "public.png"))
                .then(|| link(f, "resOriginalRes", "original"))
                .flatten()
        }),
        video: link(f, "resVidMedRes", "video.mp4")
            .or_else(|| link(f, "resVidSmallRes", "video.mp4"))
            .or_else(|| link(f, "resOriginalVidComplRes", "live.mov")),
        live: link(f, "resOriginalVidComplRes", "live.mov"),
    }
}

fn decoded(record: &Value, name: &str) -> Option<String> {
    let v = record.pointer(&format!("/fields/{name}/value"))?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(v).ok()?;
    String::from_utf8(bytes).ok().filter(|s| !s.is_empty())
}

/// A field Photos keeps as a binary property list, base64'd: keywords, a location.
fn plist_field(record: &Value, name: &str) -> Option<plist::Value> {
    let v = record.pointer(&format!("/fields/{name}/value"))?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(v).ok()?;
    plist::from_bytes(&bytes).ok()
}

/// The words a photo is filed under: `keywordsEnc`, a list of strings.
fn keywords_of(asset: &Value) -> Vec<String> {
    plist_field(asset, "keywordsEnc")
        .and_then(|v| v.into_array())
        .map(|a| {
            a.into_iter()
                .filter_map(plist::Value::into_string)
                .filter(|k| !k.trim().is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Where a photo was taken: `locationEnc` (or `locationV2Enc`), `lat` and `lon` in degrees.
/// Apple marks no fix with values out of range; those are not a place.
fn location_of(asset: &Value) -> Option<(f64, f64)> {
    let d = plist_field(asset, "locationV2Enc")
        .or_else(|| plist_field(asset, "locationEnc"))?
        .into_dictionary()?;
    let deg = |k: &str| {
        d.get(k).and_then(|v| {
            v.as_real()
                .or_else(|| v.as_signed_integer().map(|i| i as f64))
        })
    };
    let (lat, lon) = (deg("lat")?, deg("lon")?);
    ((-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat == 0.0 && lon == 0.0))
        .then_some((lat, lon))
}

fn millis(record: &Value, name: &str) -> Option<i64> {
    record
        .pointer(&format!("/fields/{name}/value"))
        .and_then(Value::as_i64)
        .map(|ms| ms / 1000)
}

/// A photo from its asset and the master it shows.
#[must_use]
/// The burst a photo was shot in (`burstId`).
fn burst_of(asset: &Value) -> Option<String> {
    asset
        .pointer("/fields/burstId/value")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Whether it is its burst's chosen photo, the one Photos shows: read on his library, each
/// burst has exactly one with `burstFlags` 20 (16 set) and `burstFlagsExt` 32; the rest are 2
/// or 4 — and Apple's own count of photos kept out of the library (67) is the bursts' 70 less
/// those three.
fn burst_pick_of(asset: &Value) -> bool {
    field::int(asset, "burstFlags").is_some_and(|f| f & 16 != 0)
        || field::int(asset, "burstFlagsExt").is_some_and(|f| f & 32 != 0)
}

pub fn photo_of(asset: &Value, master: &Value) -> Option<Photo> {
    let id = field::record_name(asset)?;
    let master_id = field::record_name(master)?;
    let m = &master["fields"];
    let uti = m
        .pointer("/itemType/value")
        .and_then(Value::as_str)
        .map(str::to_string);
    let video = uti
        .as_deref()
        .is_some_and(|t| t.contains("movie") || t.contains("mpeg") || t.contains("video"));
    let kind = if video {
        Kind::Video
    } else if m.get("resOriginalVidComplRes").is_some() {
        Kind::Live
    } else {
        Kind::Image
    };
    let size = |k: &str| m.pointer(&format!("/{k}/value")).and_then(Value::as_i64);
    let (mut width, mut height) = (
        size("resOriginalWidth")
            .or(size("resJPEGMedWidth"))
            .unwrap_or(0),
        size("resOriginalHeight")
            .or(size("resJPEGMedHeight"))
            .unwrap_or(0),
    );
    // EXIF orientations 5 to 8 are turned a quarter: the picture is as tall as it is wide.
    let orientation =
        field::int(asset, "orientation").or(field::int(master, "originalOrientation"));
    if matches!(orientation, Some(5..=8)) {
        std::mem::swap(&mut width, &mut height);
    }
    Some(Photo {
        id,
        master: master_id,
        taken_at: millis(asset, "assetDate")
            .or(millis(master, "originalCreationDate"))
            .unwrap_or(0),
        added_at: millis(asset, "addedDate"),
        kind,
        width,
        height,
        // Apple keeps a video's length in milliseconds (12917 for a 12.917 s clip, checked
        // against the files themselves); the library keeps seconds.
        duration: asset
            .pointer("/fields/duration/value")
            .and_then(Value::as_f64)
            .filter(|d| *d > 0.0)
            .map(|ms| ms / 1000.0),
        filename: decoded(master, "filenameEnc"),
        size: size("resOriginalFileSize"),
        uti,
        favorite: field::int(asset, "isFavorite") == Some(1),
        hidden: field::int(asset, "isHidden") == Some(1),
        deleted: field::int(asset, "isDeleted") == Some(1),
        deleted_at: field::int(asset, "isDeleted")
            .filter(|&d| d == 1)
            .and(millis(asset, "recordModificationDate")),
        subtype: field::int(asset, "assetSubtypeV2").unwrap_or(0),
        title: decoded(asset, "captionEnc"),
        caption: decoded(asset, "extendedDescEnc"),
        rating: field::int(asset, "rating").unwrap_or(0).clamp(0, 5),
        keywords: keywords_of(asset),
        rendered: field::int(asset, "customRenderedValue").unwrap_or(0),
        burst: burst_of(asset),
        burst_pick: burst_pick_of(asset),
        burst_count: 0,
        live_size: (kind == Kind::Live)
            .then(|| size("resOriginalVidComplFileSize"))
            .flatten(),
        latitude: location_of(asset).map(|l| l.0),
        longitude: location_of(asset).map(|l| l.1),
        expunge_at: (field::int(asset, "isDeleted") == Some(1))
            .then(|| millis(asset, "dateExpunged"))
            .flatten(),
        change_tag: asset
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// An asset's own fields laid over the photo it already is: what an asset-only change (a
/// favourite, a delete) brings, when its master did not change.
fn with_asset(mut p: Photo, asset: &Value) -> Photo {
    if let Some(t) = millis(asset, "assetDate") {
        p.taken_at = t;
    }
    p.added_at = millis(asset, "addedDate").or(p.added_at);
    p.favorite = field::int(asset, "isFavorite") == Some(1);
    p.hidden = field::int(asset, "isHidden") == Some(1);
    p.deleted = field::int(asset, "isDeleted") == Some(1);
    p.deleted_at = if p.deleted {
        millis(asset, "recordModificationDate").or(p.deleted_at)
    } else {
        None
    };
    p.title = decoded(asset, "captionEnc");
    p.caption = decoded(asset, "extendedDescEnc");
    p.rating = field::int(asset, "rating").unwrap_or(0).clamp(0, 5);
    p.keywords = keywords_of(asset);
    p.rendered = field::int(asset, "customRenderedValue").unwrap_or(p.rendered);
    if asset.pointer("/fields/burstId").is_some() {
        p.burst = burst_of(asset);
        p.burst_pick = burst_pick_of(asset);
    }
    // The asset is where a place is kept (as the full read takes it), and a changed asset
    // comes whole: none there is none at all — removed on the phone, it goes here too.
    let place = location_of(asset);
    (p.latitude, p.longitude) = (place.map(|l| l.0), place.map(|l| l.1));
    p.expunge_at = if p.deleted {
        millis(asset, "dateExpunged").or(p.expunge_at)
    } else {
        None
    };
    p.change_tag = asset
        .get("recordChangeTag")
        .and_then(Value::as_str)
        .map(str::to_string);
    p
}

/// A new asset `id` of the master `other` is a photo of: what the master gives (the file,
/// its size and kind) kept, and everything of the other asset's own let go, for
/// [`with_asset`] to lay this one's over.
fn another_asset_of(other: &Photo, id: &str, asset: &Value) -> Photo {
    let mut p = other.clone();
    p.id = id.to_string();
    p.subtype = field::int(asset, "assetSubtypeV2").unwrap_or(0);
    p.burst = None;
    p.burst_pick = false;
    p.burst_count = 0;
    p.deleted_at = None;
    p.expunge_at = None;
    p.change_tag = None;
    p
}

/// A pass applied to the library, merged in as it comes, so a large library shows as it fills.
///
/// A pass over what changed is only that. A whole read (one from the start, or one carrying
/// on from exactly where the last pass of it stopped) also notes what it saw, and when it
/// reaches the end lets go of whatever it never saw: nothing is pruned on a read that has not
/// finished. Answers how many photos changed, and every master's links seen, for the app to
/// keep.
pub fn apply(
    library: &mut quince_core::Library,
    fetched: PhotosFetch,
) -> Result<(usize, HashMap<String, Links>)> {
    // One pass is one change: a failure or crash part way leaves the library as it was.
    library.atomically(|library| apply_whole(library, fetched))
}

fn apply_whole(
    library: &mut quince_core::Library,
    fetched: PhotosFetch,
) -> Result<(usize, HashMap<String, Links>)> {
    let PhotosFetch {
        records: fresh,
        token,
        from,
        incremental,
        complete,
        zone,
    } = fetched;
    let zone = zone.as_deref();
    let walking = if incremental {
        let carrying_on = from.is_some() && library.photo_walk_cursor(zone)? == from;
        if !carrying_on {
            library.forget_photo_walk(zone)?;
        }
        carrying_on
    } else {
        library.begin_photo_walk(zone)?;
        library.mark_full_sync(&service_of(zone))?;
        true
    };
    // What the last pass held back comes first, so this pass's own records win over it.
    let records: Vec<Value> = library
        .photo_walk_held(zone)?
        .iter()
        .filter_map(|r| serde_json::from_str(r).ok())
        .chain(fresh)
        .collect();
    let mut masters: HashMap<String, &Value> = HashMap::new();
    let mut assets: Vec<&Value> = Vec::new();
    let mut gone: Vec<String> = Vec::new();
    let mut albums: Vec<Album> = Vec::new();
    let mut albums_seen = false;
    // Every album this pass speaks of: one deleted elsewhere arrives marked deleted, and has
    // to leave the list rather than stay as it was.
    let mut albums_touched: Vec<String> = Vec::new();
    // Which photo is in which album: a relation record each, named `<asset>-IN-<album>`.
    let mut in_albums: Vec<(String, String, i64)> = Vec::new();
    let mut out_of_albums: Vec<(String, String)> = Vec::new();
    for r in &records {
        if r.get("deleted").and_then(Value::as_bool) == Some(true) {
            if let Some(name) = field::record_name(r) {
                match name.split_once("-IN-") {
                    Some((asset, album)) => {
                        out_of_albums.push((album.to_string(), asset.to_string()))
                    }
                    None => gone.push(name),
                }
            }
            continue;
        }
        match field::record_type(r).as_deref() {
            Some("CPLMaster") => {
                if let Some(name) = field::record_name(r) {
                    masters.insert(name, r);
                }
            }
            // Deleted for good stays a record at Apple, marked expunged: it is gone here.
            Some("CPLAsset") if field::int(r, "isExpunged") == Some(1) => {
                if let Some(name) = field::record_name(r) {
                    gone.push(name);
                }
            }
            Some("CPLAsset") => assets.push(r),
            Some("CPLContainerRelation") => {
                let album = field::string(r, "containerId");
                let asset = field::string(r, "itemId");
                if let (Some(album), Some(asset)) = (album, asset) {
                    if field::int(r, "isDeleted") == Some(1) {
                        out_of_albums.push((album, asset));
                    } else {
                        in_albums.push((album, asset, field::int(r, "position").unwrap_or(0)));
                    }
                }
            }
            Some("CPLAlbum") => {
                albums_seen = true;
                if let Some(id) = field::record_name(r) {
                    albums_touched.push(id);
                }
                // Albums (type 0) and folders of them (type 3); smart albums and the rest are
                // not the person's own. The root folder is the top, not a folder to show.
                let kind = field::int(r, "albumType").unwrap_or(0);
                if field::int(r, "isDeleted") != Some(1) && (kind == 0 || kind == 3) {
                    if let (Some(id), Some(name)) =
                        (field::record_name(r), decoded(r, "albumNameEnc"))
                    {
                        if !id.starts_with("----") {
                            albums.push(Album {
                                id,
                                name,
                                position: field::int(r, "position").unwrap_or(0),
                                change_tag: r
                                    .get("recordChangeTag")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                folder: kind == 3,
                                parent: field::string(r, "parentId")
                                    .filter(|p| !p.is_empty() && !p.starts_with("----")),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let mut links: HashMap<String, Links> = masters
        .iter()
        .map(|(k, m)| (k.clone(), links_of(m)))
        .collect();
    // An asset's edit, laid over its master's links.
    for a in &assets {
        if let Some((name, m)) = a
            .pointer("/fields/masterRef/value/recordName")
            .and_then(Value::as_str)
            .and_then(|n| masters.get(n).map(|m| (n, m)))
        {
            links.insert(name.to_string(), links_with_edit(Some(*a), m));
        }
    }
    let mut photos = Vec::new();
    // Masters an asset in this pass was built from, and assets whose master is not here yet.
    let mut used: HashSet<&str> = HashSet::new();
    let mut waiting: Vec<&Value> = Vec::new();
    // A new asset of a master already kept: a photo duplicated in Photos is a second asset
    // made from the same master, and a pass over what changed brings the new asset alone —
    // the master did not change (icloud_photos_downloader#1394, timlaing/pyicloud#395). It
    // is built from the master's other photo here; held for a master that never comes, it
    // was dropped when the pass ended, and the duplicate never showed until a whole read.
    let elsewhere: HashMap<String, Photo> = {
        let wanted: Vec<String> = assets
            .iter()
            .filter_map(|a| {
                a.pointer("/fields/masterRef/value/recordName")
                    .and_then(Value::as_str)
            })
            .filter(|m| !masters.contains_key(*m))
            .map(str::to_string)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if wanted.is_empty() {
            HashMap::new()
        } else {
            library.photos_of_masters(&wanted)?
        }
    };
    for a in assets {
        let master = a
            .pointer("/fields/masterRef/value/recordName")
            .and_then(Value::as_str);
        let built = match master.and_then(|m| masters.get(m).map(|r| (m, r))) {
            Some((name, m)) => {
                used.insert(name);
                photo_of(a, m)
            }
            // Only the asset changed: its flags laid over the photo already known — or,
            // new, over its master's other photo.
            None => field::record_name(a).and_then(|id| {
                library
                    .photo(&id)
                    .ok()
                    .flatten()
                    .or_else(|| {
                        master
                            .and_then(|m| elsewhere.get(m))
                            .map(|p| another_asset_of(p, &id, a))
                    })
                    .map(|p| with_asset(p, a))
            }),
        };
        match built {
            Some(p) => photos.push(p),
            None if master.is_some() => waiting.push(a),
            None => {}
        }
    }
    let changed = photos.len() + gone.len();
    library.put_photos_in(zone, &photos)?;
    library.forget_photos(&gone)?;
    library.put_album_items(&in_albums)?;
    library.forget_album_items(&out_of_albums)?;
    let album_ids: Vec<String> = albums.iter().map(|a| a.id.clone()).collect();
    let mut all = library.albums()?;
    let known: HashSet<String> = all.iter().map(|a| a.id.clone()).collect();
    let gone_albums: HashSet<&String> = gone.iter().filter(|g| known.contains(*g)).collect();
    if albums_seen || !gone_albums.is_empty() {
        all.retain(|a| !albums_touched.contains(&a.id) && !gone_albums.contains(&a.id));
        for id in albums_touched.iter().chain(gone_albums.iter().copied()) {
            if !album_ids.contains(id) {
                library.forget_album(id)?;
            }
        }
        all.extend(albums);
        library.replace_albums(&all)?;
        library.forget_orphan_albums()?;
    }
    if walking {
        let seen: Vec<String> = photos.iter().map(|p| p.id.clone()).collect();
        let items: Vec<(String, String)> = in_albums
            .iter()
            .map(|(album, asset, _)| (album.clone(), asset.clone()))
            .collect();
        let carry_on_from = if complete { None } else { token.as_deref() };
        library.note_photo_walk(zone, &seen, &album_ids, &items, carry_on_from)?;
        if complete {
            library.finish_photo_walk(zone)?;
            library.forget_orphan_albums()?;
        }
    }
    // An asset and its master split across the end of an unfinished pass: the half that came
    // is held for the next pass, where the other half is.
    let mut held: Vec<(String, String)> = Vec::new();
    if !complete {
        let unused = masters
            .iter()
            .filter(|(name, _)| !used.contains(name.as_str()))
            .map(|(_, r)| *r);
        for r in unused.chain(waiting) {
            if let Some(name) = field::record_name(r) {
                held.push((name, r.to_string()));
            }
        }
    }
    library.hold_for_photo_walk(zone, &held)?;
    // Unfinished, this is where the next pass carries on from, so each pass gets further.
    library.mark_synced(&service_of(zone), token.as_deref())?;
    Ok((changed, links))
}

/// A photo's flag set, as icloud.com sets it: its `CPLAsset` record updated with the one
/// field, against the asset's change tag. Favorite is `isFavorite`; Delete (to Recently
/// Deleted, never for good) is `isDeleted: 1`; Recover is `isDeleted: 0`; Hide is `isHidden`.
/// Answers the asset as Apple now holds it.
#[allow(clippy::too_many_arguments)]
pub async fn set_flag(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    asset: &str,
    change_tag: Option<&str>,
    flag: Flag,
    on: bool,
) -> Result<Value> {
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    if flag == Flag::Expunged && on {
        return expunge_checked(&client, &zone, asset).await;
    }
    let name = match flag {
        Flag::Favorite => "isFavorite",
        Flag::Deleted => "isDeleted",
        Flag::Hidden => "isHidden",
        Flag::Expunged => "isExpunged",
    };
    client
        .update_record(
            &zone,
            asset,
            "CPLAsset",
            change_tag,
            serde_json::json!({ name: { "value": i64::from(on) } }),
        )
        .await
}

/// A photo deleted for good — which nothing takes back — only if Apple still holds it in
/// Recently Deleted, checked against Apple's record now, not this computer's copy, and written
/// over exactly that version. Recovered on another device since this window last looked, it
/// is kept: the plain update resent the delete over that recovery when its change tag moved.
/// Apple holding no such photo at all is [`crate::Error::NotFound`]: gone already, as asked
/// (by an earlier send of this same delete, or on another device).
async fn expunge_checked(client: &ZoneClient<'_>, zone: &str, asset: &str) -> Result<Value> {
    let zone = zone.to_string();
    let current = client
        .lookup_record(&zone, asset)
        .await?
        .ok_or(crate::Error::NotFound)?;
    if field::int(&current, "isDeleted") != Some(1) {
        return Err(crate::Error::Refused(
            "This photo was recovered on another device, so it was kept.".into(),
        ));
    }
    let tag = current
        .get("recordChangeTag")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            crate::Error::Protocol("Apple did not say which version of the photo it holds.".into())
        })?;
    client
        .update_record_exact(
            &zone,
            asset,
            "CPLAsset",
            tag,
            serde_json::json!({ "isExpunged": { "value": 1 } }),
        )
        .await
        .map_err(|e| match e {
            crate::Error::Conflict => crate::Error::Refused(
                "This photo changed on another device just now, so it was kept.".into(),
            ),
            other => other,
        })
}

/// One flag on many photos at once, as icloud.com sends a selection: batched, each photo
/// standing alone. `assets` are `(record name, change tag)`. Answers each photo's outcome in
/// the order given: the asset as it now is, or why not, as the error it was, so one Apple could
/// not be asked about is never taken for one it refused.
pub async fn set_flags(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    assets: &[(String, Option<String>)],
    flag: Flag,
    on: bool,
) -> Result<Vec<Result<Value>>> {
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    if flag == Flag::Expunged && on {
        // Eight at a time: a full Recently Deleted empties quickly, each photo still checked.
        let mut out = Vec::with_capacity(assets.len());
        for batch in assets.chunks(8) {
            let mut pending = Vec::with_capacity(batch.len());
            for (id, _) in batch {
                pending.push(expunge_checked(&client, &zone, id));
            }
            out.extend(futures_util::future::join_all(pending).await);
        }
        return Ok(out);
    }
    let name = match flag {
        Flag::Favorite => "isFavorite",
        Flag::Deleted => "isDeleted",
        Flag::Hidden => "isHidden",
        Flag::Expunged => "isExpunged",
    };
    let updates: Vec<(String, Option<String>, Value)> = assets
        .iter()
        .map(|(id, tag)| {
            (
                id.clone(),
                tag.clone(),
                serde_json::json!({ name: { "value": i64::from(on) } }),
            )
        })
        .collect();
    client.update_records(&zone, "CPLAsset", &updates).await
}

/// A photo's title and caption set, as icloud.com's Info sets them: the title is `captionEnc`
/// and the caption `extendedDescEnc` (Apple's names), each base64 of its UTF-8 bytes, against
/// the asset's change tag. Empty clears one. Answers the asset as it now is.
pub async fn set_title_and_caption(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    asset: &str,
    title: Option<&str>,
    caption: Option<&str>,
) -> Result<Option<Value>> {
    // Only what was changed here is written, over the photo as Apple holds it now: sending
    // both, and sending them again over a version another device had just written, put back
    // an old caption over one added on the phone.
    let mut fields = serde_json::Map::new();
    if let Some(t) = title {
        fields.insert(
            "captionEnc".into(),
            serde_json::json!({ "value": album_name(t) }),
        );
    }
    if let Some(c) = caption {
        fields.insert(
            "extendedDescEnc".into(),
            serde_json::json!({ "value": album_name(c) }),
        );
    }
    if fields.is_empty() {
        return Ok(None);
    }
    let fields = Value::Object(fields);
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    client
        .edit_record(&zone, asset, "CPLAsset", |_| Some(fields.clone()))
        .await
}

/// A photo's star rating set, as icloud.com's Info sets it: `rating`, 0 (none) to 5, against
/// the asset's change tag. Answers the asset as it now is.
pub async fn set_rating(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    asset: &str,
    change_tag: Option<&str>,
    rating: i64,
) -> Result<Value> {
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    client
        .update_record(
            &zone,
            asset,
            "CPLAsset",
            change_tag,
            serde_json::json!({ "rating": { "value": rating.clamp(0, 5) } }),
        )
        .await
}

/// A photo's keywords set, as icloud.com's Info sets them: `keywordsEnc`, a binary property
/// list of the words, base64'd, against the asset's change tag. Answers the asset as it now is.
pub async fn set_keywords(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    asset: &str,
    base: &[String],
    wanted: &[String],
) -> Result<(Value, Vec<String>)> {
    // The change made here — words added, words taken away from `base`, the list as it was
    // read — laid over the list Apple holds now, so a word added on another device meanwhile
    // stays. (The whole list sent, and sent again on a conflict, took it away.)
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let added: Vec<String> = wanted
        .iter()
        .filter(|w| !base.iter().any(|b| same(b, w)))
        .cloned()
        .collect();
    let removed: Vec<String> = base
        .iter()
        .filter(|b| !wanted.iter().any(|w| same(w, b)))
        .cloned()
        .collect();
    let result = std::sync::Mutex::new(Vec::new());
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    let written = client
        .edit_record(&zone, asset, "CPLAsset", |current| {
            let mut words: Vec<String> = keywords_of(current)
                .into_iter()
                .filter(|w| !removed.iter().any(|r| same(r, w)))
                .collect();
            for a in &added {
                if !words.iter().any(|w| same(w, a)) {
                    words.push(a.clone());
                }
            }
            let list =
                plist::Value::Array(words.iter().cloned().map(plist::Value::String).collect());
            let mut bytes = Vec::new();
            plist::to_writer_binary(&mut bytes, &list).ok()?;
            *result
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = words;
            Some(serde_json::json!({
                "keywordsEnc": { "value": base64::engine::general_purpose::STANDARD.encode(bytes) }
            }))
        })
        .await?
        .ok_or_else(|| crate::Error::Protocol("keywords could not be written".into()))?;
    let words = std::mem::take(
        &mut *result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    Ok((written, words))
}

/// When a photo was taken, changed, as icloud.com's Adjust Date and Time changes it: `assetDate`,
/// the moment in milliseconds, against the asset's change tag. (With Advanced Data Protection
/// icloud.com writes `assetDateEnc` too; his account does not use it.) Answers the asset.
pub async fn set_date(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    asset: &str,
    change_tag: Option<&str>,
    millis: i64,
) -> Result<Value> {
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    client
        .update_record(
            &zone,
            asset,
            "CPLAsset",
            change_tag,
            serde_json::json!({ "assetDate": { "value": millis } }),
        )
        .await
}

/// An album's name as Photos writes it: base64 of its UTF-8 bytes, untyped.
fn album_name(name: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(name.as_bytes())
}

/// Highest position an album or an album's item may take (`MAX_SAFE_INTEGER - 1024`).
pub const MAX_POSITION: i64 = 9_007_199_254_740_991 - 1024;
/// Gap between items added to an album, and the first item's position.
const ITEM_SPACING: i64 = 1024;

/// A new album — or folder — made as icloud.com makes one: CloudKit names it, it sorts first
/// (two below the first there is), its photos sort by date; a folder is albumType 3, and either
/// may sit in a folder (`parentId`). Answers the album.
pub async fn create_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    name: &str,
    first_position: Option<i64>,
    parent: Option<&str>,
    folder: bool,
) -> Result<Album> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let zone = library_zone(&client).await;
    let position = first_position.map_or(MAX_POSITION, |p| (p - 2).max(0));
    let made = client
        .create_record_unnamed(
            &zone,
            "CPLAlbum",
            serde_json::json!({
                "albumNameEnc": { "value": album_name(name) },
                "albumType": { "value": if folder { 3 } else { 0 } },
                "isDeleted": { "value": 0 },
                "isExpunged": { "value": 0 },
                "position": { "value": position },
                "sortAscending": { "value": 1 },
                "sortType": { "value": 1 },
            })
            .as_object()
            .cloned()
            .map(|mut f| {
                if let Some(p) = parent {
                    f.insert("parentId".into(), serde_json::json!({ "value": p }));
                }
                Value::Object(f)
            })
            .unwrap_or_default(),
        )
        .await?;
    Ok(Album {
        id: field::record_name(&made).ok_or_else(|| {
            crate::Error::Protocol("CloudKit made the album without a name".into())
        })?,
        name: name.to_string(),
        position,
        change_tag: made
            .get("recordChangeTag")
            .and_then(Value::as_str)
            .map(str::to_string),
        folder,
        parent: parent.map(str::to_string),
    })
}

/// An album renamed, against its change tag. Answers the new change tag.
pub async fn rename_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    change_tag: Option<&str>,
    name: &str,
) -> Result<Option<String>> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let zone = library_zone(&client).await;
    let now = jiff::Timestamp::now().as_millisecond();
    let answer = client
        .update_record(
            &zone,
            album,
            "CPLAlbum",
            change_tag,
            serde_json::json!({
                "albumNameEnc": { "value": album_name(name) },
                "userModificationDate": { "value": now },
            }),
        )
        .await?;
    Ok(answer
        .get("recordChangeTag")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// An album deleted, as Photos deletes one: marked deleted. Its photos stay in the library.
pub async fn delete_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    change_tag: Option<&str>,
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let zone = library_zone(&client).await;
    client
        .update_record(
            &zone,
            album,
            "CPLAlbum",
            change_tag,
            serde_json::json!({ "isDeleted": { "value": 1 } }),
        )
        .await?;
    Ok(())
}

/// Photos put in an album, after what is there: a `CPLContainerRelation` each, named
/// `<asset>-IN-<album>`, 1024 apart. Answers each photo with the position it took.
pub async fn add_to_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    assets: &[String],
    last_position: Option<i64>,
) -> (Vec<(String, i64)>, Result<()>) {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let zone = library_zone(&client).await;
    let mut at = last_position.unwrap_or(ITEM_SPACING);
    // What went in, even when a later one fails: the library records those, so they are not
    // shown as missing and asked for again.
    let mut added = Vec::new();
    for asset in assets {
        at = (at + ITEM_SPACING).min(MAX_POSITION);
        let name = format!("{asset}-IN-{album}");
        let made = client
            .create_record(
                &zone,
                &name,
                "CPLContainerRelation",
                serde_json::json!({
                    "itemId": { "value": asset },
                    "position": { "value": at },
                    "containerId": { "value": album },
                }),
            )
            .await;
        if let Err(e) = made {
            // Already there — added before a failure, or taken out on another device (marked,
            // not removed): in the album now, revived if it was taken out.
            match client.lookup_record(&zone, &name).await {
                Ok(Some(existing)) => {
                    if field::int(&existing, "isDeleted") == Some(1) {
                        let revived = client
                            .edit_record(&zone, &name, "CPLContainerRelation", |_| {
                                Some(serde_json::json!({
                                    "isDeleted": { "value": 0 },
                                    "position": { "value": at },
                                }))
                            })
                            .await;
                        if let Err(e) = revived {
                            return (added, Err(e));
                        }
                    } else if let Some(p) = field::int(&existing, "position") {
                        at = p;
                    }
                }
                _ => return (added, Err(e)),
            }
        }
        added.push((asset.clone(), at));
    }
    (added, Ok(()))
}

/// Photos taken out of an album, as icloud.com takes them: their relation records removed
/// outright. The photos stay in the library.
pub async fn remove_from_album(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    album: &str,
    assets: &[String],
) -> Result<()> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let zone = library_zone(&client).await;
    for asset in assets {
        client
            .delete_record(&zone, &format!("{asset}-IN-{album}"))
            .await?;
    }
    Ok(())
}

/// A file fetched from `url` to `path`, streamed: written to a part file of its own and moved
/// into place only once whole, so a large video is never held in memory and a half-written file
/// is never where a whole one is looked for. A refused fetch leaves nothing behind. Answers false
/// when Apple refused the link (so the caller may ask for a fresh one), true when it is there.
pub async fn fetch_to_file(
    http: &reqwest::Client,
    url: &str,
    path: &std::path::Path,
) -> Result<bool> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let response =
        http.get(url).send().await.map_err(|e| {
            crate::Error::Protocol(format!("the picture could not be fetched: {e}"))
        })?;
    if !response.status().is_success() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    let partial = path.with_extension(format!("{}.part", uuid::Uuid::new_v4().simple()));
    let mut out = tokio::fs::File::create(&partial)
        .await
        .map_err(|e| crate::Error::Protocol(e.to_string()))?;
    let mut body = response.bytes_stream();
    let written: std::result::Result<(), String> = async {
        while let Some(chunk) = body.next().await {
            out.write_all(&chunk.map_err(|e| e.to_string())?)
                .await
                .map_err(|e| e.to_string())?;
        }
        out.flush().await.map_err(|e| e.to_string())
    }
    .await;
    drop(out);
    if let Err(e) = written {
        tokio::fs::remove_file(&partial).await.ok();
        return Err(crate::Error::Protocol(format!(
            "the picture broke off: {e}"
        )));
    }
    // Waiting while Windows holds the file just written (quince_core::files).
    let (from, to) = (partial.clone(), path.to_path_buf());
    tokio::task::spawn_blocking(move || quince_core::files::rename(&from, &to))
        .await
        .map_err(|e| crate::Error::Protocol(e.to_string()))?
        .map_err(|e| crate::Error::Protocol(e.to_string()))?;
    Ok(true)
}

/// Which file a download takes, as icloud.com's Download Options offer them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Want {
    /// The original, as captured or imported.
    Originals,
    /// The best there is, edits included: HEIC and HEVC allowed.
    HighestQuality,
    /// JPEG, PNG, GIF, MOV or MP4 where there is one.
    MostCompatible,
}

const PHOTO_SLOTS: [&str; 5] = ["JPEGThumb", "JPEGMed", "JPEGLarge", "JPEGFull", "Original"];
const VIDEO_SLOTS: [&str; 4] = ["VidSmall", "VidMed", "VidFull", "Original"];

/// One rendition of a record: its file type (a UTI, lower case) and where to fetch it. The
/// original is on the master only. Any rendition not the original that says QuickTime is an MP4,
/// as icloud.com treats it (main.js `fileDataForSlot`).
fn rendition(slot: &str, record: &Value) -> Option<(String, String)> {
    let master = field::record_type(record).as_deref() == Some("CPLMaster");
    let (kind, url) = if slot == "Original" {
        if !master {
            return None;
        }
        (
            field::string(record, "resOriginalFileType"),
            record.pointer("/fields/resOriginalRes/value/downloadURL"),
        )
    } else {
        (
            field::string(record, &format!("res{slot}FileType")),
            record.pointer(&format!("/fields/res{slot}Res/value/downloadURL")),
        )
    };
    let mut kind = kind?.to_lowercase();
    if slot != "Original" && kind == "com.apple.quicktime-movie" {
        kind = "public.mpeg-4".into();
    }
    Some((kind, url?.as_str()?.to_string()))
}

/// The extension a downloaded file is saved with, always one of a fixed list of media types.
/// The type and the name both come from whoever took the photo (a Shared Library's are other
/// people's), so neither is used as it is: a claimed type ending `.desktop` would otherwise
/// name a launcher. An original of a type not listed keeps its own extension only when that
/// is a known media one, and is otherwise saved as plain data.
fn media_extension(kind: &str, own: &str) -> &'static str {
    const KNOWN: [&str; 22] = [
        "JPG", "JPEG", "PNG", "GIF", "HEIC", "HEIF", "TIF", "TIFF", "WEBP", "DNG", "CR2", "CR3",
        "NEF", "ARW", "ORF", "RAF", "RW2", "MOV", "MP4", "M4V", "AVI", "PDF",
    ];
    match kind {
        "public.jpeg" => "JPG",
        "public.png" => "PNG",
        "com.compuserve.gif" => "GIF",
        "public.heic" => "HEIC",
        "public.heif" => "HEIF",
        "public.tiff" => "TIF",
        "org.webmproject.webp" => "WEBP",
        "com.adobe.raw-image" => "DNG",
        "com.apple.quicktime-movie" => "MOV",
        "public.mpeg-4" => "MP4",
        "com.apple.m4v-video" => "M4V",
        "public.avi" => "AVI",
        "com.adobe.pdf" => "PDF",
        _ => KNOWN
            .into_iter()
            .find(|k| k.eq_ignore_ascii_case(own))
            .unwrap_or("bin"),
    }
}

/// The file a download takes, as icloud.com picks it (ui.main.js `getDownloadURLInfo`): the
/// largest rendition of an allowed type, from the asset first — where an edit's renditions are —
/// then the master; for originals, the master's original whatever its type. Answers the URL with
/// `${f}` filled, and the file's name: the original's, with the chosen type's extension.
/// The name is the photo's owner's to choose and may hold anything, a path included: a caller
/// writing a file makes it one plain file name first (src-tauri's `export::file_name`).
#[must_use]
pub fn download_choice(
    asset: &Value,
    master: &Value,
    want: Want,
    video: bool,
) -> Option<(String, String)> {
    let mut allowed = vec![
        "public.jpeg",
        "public.png",
        "com.compuserve.gif",
        "com.apple.quicktime-movie",
        "public.mpeg-4",
    ];
    if want != Want::MostCompatible {
        allowed.extend(["public.heic", "com.apple.m4v-video"]);
    }
    let slots: &[&str] = if video { &VIDEO_SLOTS } else { &PHOTO_SLOTS };
    let pick = |record: &Value| {
        slots.iter().rev().find_map(|slot| {
            let (kind, url) = rendition(slot, record)?;
            ((*slot == "Original" && want == Want::Originals)
                || kind == "com.adobe.pdf"
                || allowed.contains(&kind.as_str()))
            .then_some((kind, url))
        })
    };
    let from_master = pick(master);
    let (kind, url) = if want == Want::Originals {
        from_master.or_else(|| pick(asset))?
    } else {
        pick(asset).or(from_master)?
    };
    let full = decoded(master, "filenameEnc").unwrap_or_else(|| "Photo".into());
    let (base, own_ext) = full.rsplit_once('.').unwrap_or((full.as_str(), ""));
    let name = format!("{base}.{}", media_extension(&kind, own_ext));
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    Some((url.replace("${f}", &encoded), name))
}

/// The flags a photo carries that this app sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Flag {
    Favorite,
    Deleted,
    Hidden,
    /// Deleted for good, from Recently Deleted: `isExpunged: 1`. It cannot be undone.
    Expunged,
}

/// One master's links, read from Apple now: for a picture whose cached link expired, or one
/// never seen in this run.
pub async fn links(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    place: Option<&crate::shared_collections::Place>,
    master: &str,
    asset: &str,
) -> Result<Option<Links>> {
    let client = client_in(http, ckdatabasews_root, params, place);
    let zone = zone_in(&client, place).await;
    let Some(m) = client.lookup_record(&zone, master).await? else {
        return Ok(None);
    };
    // The asset too, for an edit's pictures; without it, the master's alone.
    let a = client.lookup_record(&zone, asset).await.ok().flatten();
    Ok(Some(links_with_edit(a.as_ref(), &m)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn master() -> Value {
        json!({"recordName": "M1", "recordType": "CPLMaster", "fields": {
            "itemType": {"value": "public.jpeg"},
            "filenameEnc": {"value": base64::engine::general_purpose::STANDARD.encode("IMG_0001.JPG")},
            "resOriginalWidth": {"value": 4032}, "resOriginalHeight": {"value": 3024},
            "resOriginalFileSize": {"value": 2_400_000},
            "resJPEGThumbRes": {"value": {"downloadURL": "https://cvws.example/thumb/${f}"}},
            "resOriginalRes": {"value": {"downloadURL": "https://cvws.example/orig/${f}"}}
        }})
    }

    fn asset(flags: Value) -> Value {
        let mut fields = json!({
            "masterRef": {"value": {"recordName": "M1"}},
            "assetDate": {"value": 1_789_640_832_217_i64}, "addedDate": {"value": 1_792_337_452_485_i64},
            "orientation": {"value": 6}
        });
        for (k, v) in flags.as_object().unwrap() {
            fields[k] = json!({"value": v});
        }
        json!({"recordName": "A1", "recordType": "CPLAsset", "recordChangeTag": "t1", "fields": fields})
    }

    /// An asset and its master read as one photo: its date, size (turned for orientation 6),
    /// name, flags and download links.
    #[test]
    fn an_asset_and_its_master_are_one_photo() {
        let p = photo_of(&asset(json!({"isFavorite": 1})), &master()).unwrap();
        assert_eq!((p.id.as_str(), p.master.as_str()), ("A1", "M1"));
        assert_eq!(p.taken_at, 1_789_640_832);
        assert_eq!(
            (p.width, p.height),
            (3024, 4032),
            "orientation 6 is a portrait"
        );
        assert_eq!(p.filename.as_deref(), Some("IMG_0001.JPG"));
        assert!(p.favorite && !p.deleted);
        assert_eq!(p.kind, Kind::Image);
        let l = links_of(&master());
        assert_eq!(
            l.thumb.as_deref(),
            Some("https://cvws.example/thumb/thumb.jpg")
        );
        assert_eq!(
            l.original.as_deref(),
            Some("https://cvws.example/orig/original")
        );
    }

    /// A photo edited on an iPhone or a Mac shows edited: its asset's pictures over the
    /// master's, the original still the master's.
    #[test]
    fn an_edited_photo_shows_its_edit() {
        let edited = asset(json!({
            "resJPEGThumbRes": {"downloadURL": "https://cvws.example/edit-thumb/${f}"},
            "resJPEGFullRes": {"downloadURL": "https://cvws.example/edit-full/${f}"}
        }));
        let l = links_with_edit(Some(&edited), &master());
        assert_eq!(
            l.thumb.as_deref(),
            Some("https://cvws.example/edit-thumb/thumb.jpg")
        );
        assert_eq!(
            l.full.as_deref(),
            Some("https://cvws.example/edit-full/full.jpg")
        );
        assert_eq!(
            l.medium.as_deref(),
            Some("https://cvws.example/edit-full/full.jpg")
        );
        assert_eq!(
            l.original.as_deref(),
            Some("https://cvws.example/orig/original")
        );
        // Not edited: the master's, as before.
        let plain = links_with_edit(Some(&asset(json!({}))), &master());
        assert_eq!(plain, links_of(&master()));
    }

    /// A burst's photos know their burst; the chosen one (flags 20 / 32) says so.
    #[test]
    fn a_burst_and_its_chosen_photo() {
        let pick = photo_of(
            &asset(json!({"burstId": "B1", "burstFlags": 20, "burstFlagsExt": 32})),
            &master(),
        )
        .unwrap();
        let other = photo_of(&asset(json!({"burstId": "B1", "burstFlags": 2})), &master()).unwrap();
        let four = photo_of(&asset(json!({"burstId": "B1", "burstFlags": 4})), &master()).unwrap();
        assert_eq!(pick.burst.as_deref(), Some("B1"));
        assert!(pick.burst_pick && !other.burst_pick && !four.burst_pick);
        assert_eq!(photo_of(&asset(json!({})), &master()).unwrap().burst, None);
    }

    /// A video's length: Apple's milliseconds, kept as seconds (a 62.18 s clip, not 1036 minutes).
    #[test]
    fn a_video_length_is_seconds() {
        let p = photo_of(&asset(json!({"duration": 62184.0})), &master()).unwrap();
        assert_eq!(p.duration, Some(62.184));
        assert_eq!(
            photo_of(&asset(json!({})), &master()).unwrap().duration,
            None
        );
    }

    /// A Live Photo knows its moving part: the original .MOV's link and size, for the Photos
    /// folder to list it beside the still. A still photo has neither.
    #[test]
    fn a_live_photo_knows_its_moving_part() {
        let mut m = master();
        m["fields"]["resOriginalVidComplRes"] =
            json!({"value": {"downloadURL": "https://cvws.example/live/${f}"}});
        m["fields"]["resOriginalVidComplFileSize"] = json!({"value": 3_500_000});
        let p = photo_of(&asset(json!({})), &m).unwrap();
        assert_eq!((p.kind, p.live_size), (Kind::Live, Some(3_500_000)));
        assert_eq!(
            links_of(&m).live.as_deref(),
            Some("https://cvws.example/live/live.mov")
        );
        let still = photo_of(&asset(json!({})), &master()).unwrap();
        assert_eq!((still.live_size, links_of(&master()).live), (None, None));
    }

    /// A full pass replaces the library; a later asset-only change (deleted) is laid over the
    /// photo already known; an expunged asset goes.
    /// A pass that fails as it stores its position — its last write, standing in for a crash
    /// part way — stores none of the photos it had already put in.
    /// A Shared Library read whole stores its photos marked with its zone and keeps its own
    /// place; the personal library's whole read that follows, seeing none of them, lets go of
    /// none of them; and the Shared Library's photo is the same photo the personal parser makes.
    #[test]
    fn a_shared_library_pass_is_its_own() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let mut shared_master = master();
        shared_master["recordName"] = json!("M2");
        let mut shared_asset = asset(json!({}));
        shared_asset["recordName"] = json!("A2");
        shared_asset["fields"]["masterRef"] = json!({"value": {"recordName": "M2"}});
        apply(
            &mut lib,
            PhotosFetch {
                records: vec![shared_master, shared_asset],
                token: Some("S1".into()),
                from: None,
                incremental: false,
                complete: true,
                zone: Some("SharedSync-X".into()),
            },
        )
        .unwrap();
        apply(
            &mut lib,
            PhotosFetch {
                records: vec![master(), asset(json!({}))],
                token: Some("P1".into()),
                from: None,
                incremental: false,
                complete: true,
                zone: None,
            },
        )
        .unwrap();
        assert_eq!(
            lib.photo_zone("A2").unwrap().as_deref(),
            Some("SharedSync-X")
        );
        assert_eq!(lib.photo_zone("A1").unwrap(), None);
        assert_eq!(
            lib.sync_token("photos:SharedSync-X").unwrap().as_deref(),
            Some("S1")
        );
        assert_eq!(lib.sync_token(SERVICE).unwrap().as_deref(), Some("P1"));
        let a1 = lib.photo("A1").unwrap().unwrap();
        let a2 = lib.photo("A2").unwrap().unwrap();
        assert_eq!(
            (a1.taken_at, a1.kind, a1.width),
            (a2.taken_at, a2.kind, a2.width)
        );
    }

    #[test]
    fn a_pass_that_fails_part_way_stores_none_of_itself() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        lib.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER crash_i BEFORE INSERT ON main.sync_state
                   BEGIN SELECT RAISE(ABORT, 'crash mid-apply'); END;
                 CREATE TEMP TRIGGER crash_u BEFORE UPDATE ON main.sync_state
                   BEGIN SELECT RAISE(ABORT, 'crash mid-apply'); END;",
            )
            .unwrap();
        let pass = PhotosFetch {
            records: vec![master(), asset(json!({}))],
            token: Some("T1".into()),
            from: None,
            incremental: false,
            complete: true,
            zone: None,
        };
        assert!(apply(&mut lib, pass).is_err());
        assert!(lib
            .photos(quince_core::photos::View::Library)
            .unwrap()
            .is_empty());
        assert_eq!(lib.sync_token(SERVICE).unwrap(), None);
        assert!(lib.conn().is_autocommit());
    }

    #[test]
    fn passes_build_and_keep_the_library() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let full = PhotosFetch {
            records: vec![
                master(),
                asset(json!({})),
                json!({"recordName": "A1-IN-AL", "recordType": "CPLContainerRelation", "fields": {"containerId": {"value": "AL"}, "itemId": {"value": "A1"}, "position": {"value": 1024}}}),
            ],
            token: Some("T1".into()),
            from: None,
            incremental: false,
            complete: true,
            zone: None,
        };
        apply(&mut lib, full).unwrap();
        assert_eq!(
            lib.photos(quince_core::photos::View::Library)
                .unwrap()
                .len(),
            1
        );
        let change = PhotosFetch {
            records: vec![asset(json!({"isDeleted": 1}))],
            token: Some("T2".into()),
            from: None,
            incremental: true,
            complete: true,
            zone: None,
        };
        apply(&mut lib, change).unwrap();
        assert!(lib
            .photos(quince_core::photos::View::Library)
            .unwrap()
            .is_empty());
        let deleted = lib
            .photos(quince_core::photos::View::RecentlyDeleted)
            .unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].filename.as_deref(),
            Some("IMG_0001.JPG"),
            "the master's details are kept"
        );
        let expunged = PhotosFetch {
            records: vec![json!({"recordName": "A1", "deleted": true})],
            token: Some("T3".into()),
            from: None,
            incremental: true,
            complete: true,
            zone: None,
        };
        apply(&mut lib, expunged).unwrap();
        assert!(lib.photo("A1").unwrap().is_none());
    }

    /// An album deleted on another device arrives marked deleted; it leaves the list, and
    /// its photos leave it with it (Apple leaves the relations standing), while a rename
    /// elsewhere is taken as it comes.
    #[test]
    fn an_album_deleted_elsewhere_goes() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let album = |name: &str, deleted: i64| {
            json!({"recordName": "AL", "recordType": "CPLAlbum", "fields": {
                "albumNameEnc": {"value": base64::engine::general_purpose::STANDARD.encode(name)},
                "albumType": {"value": 0}, "position": {"value": 5}, "isDeleted": {"value": deleted}}})
        };
        let relation = json!({"recordName": "A1-IN-AL", "recordType": "CPLContainerRelation", "fields": {"containerId": {"value": "AL"}, "itemId": {"value": "A1"}, "position": {"value": 1024}}});
        let pass = |records, incremental| PhotosFetch {
            records,
            token: Some("T".into()),
            from: None,
            incremental,
            complete: true,
            zone: None,
        };
        apply(
            &mut lib,
            pass(
                vec![master(), asset(json!({})), album("Trips", 0), relation],
                false,
            ),
        )
        .unwrap();
        assert_eq!(lib.albums().unwrap()[0].name, "Trips");
        apply(&mut lib, pass(vec![album("Sea", 0)], true)).unwrap();
        assert_eq!(
            lib.albums().unwrap()[0].name,
            "Sea",
            "a rename elsewhere is taken"
        );
        apply(&mut lib, pass(vec![album("Sea", 1)], true)).unwrap();
        assert!(
            lib.albums().unwrap().is_empty(),
            "a deleted album leaves the list"
        );
        assert!(
            lib.album_photos("AL").unwrap().is_empty(),
            "and its photos with it"
        );
        assert_eq!(
            lib.photos(quince_core::photos::View::Library)
                .unwrap()
                .len(),
            1,
            "the photos stay in the library"
        );
    }

    /// A photo deleted for good stays a record at Apple, marked `isExpunged`; the library
    /// lets it go rather than keep it in Recently Deleted.
    #[test]
    fn an_expunged_photo_goes() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let pass = |records, incremental| PhotosFetch {
            records,
            token: Some("T".into()),
            from: None,
            incremental,
            complete: true,
            zone: None,
        };
        apply(
            &mut lib,
            pass(vec![master(), asset(json!({"isDeleted": 1}))], false),
        )
        .unwrap();
        assert_eq!(
            lib.photos(quince_core::photos::View::RecentlyDeleted)
                .unwrap()
                .len(),
            1
        );
        apply(
            &mut lib,
            pass(vec![asset(json!({"isDeleted": 1, "isExpunged": 1}))], true),
        )
        .unwrap();
        assert!(lib.photo("A1").unwrap().is_none());
    }

    /// A photo duplicated in Photos is a second asset of the same master. A pass over what
    /// changed brings the new asset alone, the master unchanged (icloud_photos_downloader#1394,
    /// timlaing/pyicloud#395): it is a photo of its own, built from the master already kept,
    /// with its own flags — not held for a master that never comes and dropped.
    #[test]
    fn a_new_asset_of_a_kept_master_is_a_photo_of_its_own() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let pass = |records, incremental| PhotosFetch {
            records,
            token: Some("T".into()),
            from: None,
            incremental,
            complete: true,
            zone: None,
        };
        apply(
            &mut lib,
            pass(vec![master(), asset(json!({"isFavorite": 1}))], false),
        )
        .unwrap();
        let mut duplicate = asset(json!({ "captionEnc":
            base64::engine::general_purpose::STANDARD.encode("the copy") }));
        duplicate["recordName"] = json!("A2");
        duplicate["recordChangeTag"] = json!("t9");
        let (changed, _) = apply(&mut lib, pass(vec![duplicate], true)).unwrap();
        assert_eq!(changed, 1);

        let copy = lib
            .photo("A2")
            .unwrap()
            .expect("the duplicate is in the library");
        assert_eq!(copy.master, "M1");
        assert_eq!(
            copy.filename.as_deref(),
            Some("IMG_0001.JPG"),
            "the master's file"
        );
        assert_eq!(copy.size, Some(2_400_000));
        assert!(!copy.favorite, "its own flags, not the first photo's");
        assert_eq!(copy.title.as_deref(), Some("the copy"));
        assert_eq!(copy.change_tag.as_deref(), Some("t9"));
        let first = lib.photo("A1").unwrap().expect("the first is still there");
        assert!(first.favorite);
        assert_eq!(
            lib.photos(quince_core::photos::View::Library)
                .unwrap()
                .len(),
            2,
            "two photos of one master"
        );
    }

    /// Keywords and a place, from the bytes Apple holds for his photo titled "loupe": a binary
    /// plist list of words, and one of where (`lat`, `lon`) with Apple's marks for no fix.
    #[test]
    fn keywords_and_place_are_read_as_apple_keeps_them() {
        let asset = json!({"fields": {
            "keywordsEnc": {"value": "YnBsaXN0MDChAVVsb3VwZQgKAAAAAAAAAQEAAAAAAAAAAgAAAAAAAAAAAAAAAAAAABA="},
            "locationEnc": {"value": "YnBsaXN0MDDYAQIDBAUGBwgJCgsMDQ4PEFNhbHRWY291cnNlV2hvcnpBY2NXdmVydEFjY1NsYXRTbG9uVXNwZWVkWXRpbWVzdGFtcBAAE///////////E///////////E///////////I0Axw++gAAAAI0BU1uMAAAAAE///////////XxAYMjAyNi0xMC0xOFQxNTo0MDo0NS41ODJaCBkdJCw0ODxCTE5XYGlye4QAAAAAAAABAQAAAAAAAAARAAAAAAAAAAAAAAAAAAAAnw=="}
        }});
        assert_eq!(keywords_of(&asset), ["loupe"]);
        let (lat, lon) = location_of(&asset).unwrap();
        assert!((lat - 17.765_375).abs() < 1e-5 && (lon - 83.357_605).abs() < 1e-5);
        assert!(
            location_of(&json!({"fields": {}})).is_none(),
            "no place is no place"
        );
    }

    /// Download Options pick as icloud.com does: an original as it is; Most Compatible turns
    /// a HEIC original into the full-size JPEG; an edit (renditions on the asset) wins for
    /// Highest Quality, never for originals.
    #[test]
    fn downloads_pick_the_file_icloud_picks() {
        let master = json!({"recordType": "CPLMaster", "fields": {
            "filenameEnc": {"value": base64::engine::general_purpose::STANDARD.encode("IMG_0001.HEIC")},
            "resOriginalFileType": {"value": "public.heic"},
            "resOriginalRes": {"value": {"downloadURL": "https://o/${f}"}},
            "resJPEGFullFileType": {"value": "public.jpeg"},
            "resJPEGFullRes": {"value": {"downloadURL": "https://full/${f}"}},
        }});
        let plain = json!({"recordType": "CPLAsset", "fields": {}});
        let edited = json!({"recordType": "CPLAsset", "fields": {
            "resJPEGFullFileType": {"value": "public.jpeg"},
            "resJPEGFullRes": {"value": {"downloadURL": "https://edit/${f}"}},
        }});
        assert_eq!(
            download_choice(&plain, &master, Want::Originals, false).unwrap(),
            ("https://o/IMG_0001.HEIC".into(), "IMG_0001.HEIC".into())
        );
        assert_eq!(
            download_choice(&plain, &master, Want::MostCompatible, false).unwrap(),
            ("https://full/IMG_0001.JPG".into(), "IMG_0001.JPG".into())
        );
        assert_eq!(
            download_choice(&plain, &master, Want::HighestQuality, false)
                .unwrap()
                .1,
            "IMG_0001.HEIC"
        );
        assert_eq!(
            download_choice(&edited, &master, Want::HighestQuality, false)
                .unwrap()
                .0,
            "https://edit/IMG_0001.JPG"
        );
        assert_eq!(
            download_choice(&edited, &master, Want::Originals, false)
                .unwrap()
                .0,
            "https://o/IMG_0001.HEIC"
        );
    }

    /// A type Apple's record claims never becomes the extension: a Shared Library photo of
    /// type `x.desktop` is saved as data, a raw original keeps its own known extension.
    #[test]
    fn a_downloads_extension_is_always_a_known_media_one() {
        let master = |name: &str, kind: &str| {
            json!({"recordType": "CPLMaster", "fields": {
                "filenameEnc": {"value": base64::engine::general_purpose::STANDARD.encode(name)},
                "resOriginalFileType": {"value": kind},
                "resOriginalRes": {"value": {"downloadURL": "https://o/${f}"}},
            }})
        };
        let plain = json!({"recordType": "CPLAsset", "fields": {}});
        let name = |name: &str, kind: &str| {
            download_choice(&plain, &master(name, kind), Want::Originals, false)
                .unwrap()
                .1
        };
        // Its own known media extension, never the claimed type's.
        assert_eq!(name("x.png", "com.evil.desktop"), "x.PNG");
        assert_eq!(name("x.desktop", "com.evil.desktop"), "x.bin");
        assert_eq!(name("IMG_2.cr2", "com.canon.cr2-raw-image"), "IMG_2.CR2");
        assert_eq!(name("IMG_3.DNG", "com.adobe.raw-image"), "IMG_3.DNG");
        assert_eq!(name("no extension", "public.tiff"), "no extension.TIF");
    }

    /// Folders come through as folders, an album in one knows it, and when the folder is
    /// deleted elsewhere the album inside goes with it — though Apple keeps that album.
    #[test]
    fn folders_and_what_they_hold() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let album = |id: &str, name: &str, kind: i64, parent: Option<&str>, deleted: i64| {
            let mut f = json!({
                "albumNameEnc": {"value": base64::engine::general_purpose::STANDARD.encode(name)},
                "albumType": {"value": kind}, "position": {"value": 1}, "isDeleted": {"value": deleted}});
            if let Some(p) = parent {
                f["parentId"] = json!({"value": p});
            }
            json!({"recordName": id, "recordType": "CPLAlbum", "fields": f})
        };
        let pass = |records, incremental| PhotosFetch {
            records,
            token: Some("T".into()),
            from: None,
            incremental,
            complete: true,
            zone: None,
        };
        apply(
            &mut lib,
            pass(
                vec![
                    album("F", "Trips", 3, Some("----Root-Folder----"), 0),
                    album("A", "Goa", 0, Some("F"), 0),
                ],
                false,
            ),
        )
        .unwrap();
        let all = lib.albums().unwrap();
        let f = all.iter().find(|a| a.id == "F").unwrap();
        let a = all.iter().find(|a| a.id == "A").unwrap();
        assert!(f.folder && f.parent.is_none(), "a folder at the top");
        assert!(
            !a.folder && a.parent.as_deref() == Some("F"),
            "an album in it"
        );
        apply(&mut lib, pass(vec![album("F", "Trips", 3, None, 1)], true)).unwrap();
        assert!(
            lib.albums().unwrap().is_empty(),
            "the folder deleted takes the album inside with it"
        );
    }

    /// A picture fetched to disk arrives whole under its name, through a part file; a refused
    /// link leaves nothing, and says so, so a fresh link can be asked for.
    #[tokio::test]
    async fn pictures_are_fetched_whole_to_disk() {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let body: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        Mock::given(path("/ok"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        Mock::given(path("/gone"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let dir =
            std::env::temp_dir().join(format!("quince-fetch-{}", uuid::Uuid::new_v4().simple()));
        let http = reqwest::Client::new();
        let target = dir.join("full").join("A1");
        assert!(
            fetch_to_file(&http, &format!("{}/ok", server.uri()), &target)
                .await
                .unwrap()
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            body,
            "every byte, in order"
        );
        let other = dir.join("full").join("A2");
        assert!(
            !fetch_to_file(&http, &format!("{}/gone", server.uri()), &other)
                .await
                .unwrap()
        );
        assert!(!other.exists(), "a refused link leaves nothing");
        let leftovers: Vec<_> = std::fs::read_dir(dir.join("full"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.to_string_lossy().ends_with("part"))
            })
            .collect();
        assert!(leftovers.is_empty(), "no part file left behind");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Deleting for good is checked against Apple, not this computer's copy: a photo recovered
    /// on the phone since is kept (no write is sent at all); one still in Recently Deleted is
    /// deleted on exactly the version read.
    #[tokio::test]
    async fn a_photo_recovered_elsewhere_is_never_deleted_for_good() {
        use wiremock::matchers::{body_string_contains, method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let record = |name: &str, deleted: i64, tag: &str| {
            serde_json::json!({ "records": [{ "recordName": name, "recordType": "CPLAsset",
                "recordChangeTag": tag, "fields": { "isDeleted": { "value": deleted } } }] })
        };
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .and(body_string_contains("\"KEPT\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(record("KEPT", 0, "k2")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .and(body_string_contains("\"GONE\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(record("GONE", 1, "g7")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .and(body_string_contains("\"g7\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(record("GONE", 1, "g8")))
            .expect(1)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let answers = set_flags(
            &http,
            &server.uri(),
            vec![],
            None,
            &[
                ("KEPT".into(), Some("k1".into())),
                ("GONE".into(), Some("g1".into())),
            ],
            Flag::Expunged,
            true,
        )
        .await
        .unwrap();
        assert!(
            matches!(&answers[0], Err(e) if e.to_string().contains("recovered on another device")),
            "{answers:?}"
        );
        assert!(answers[1].is_ok(), "{answers:?}");
        // The one modify is GONE's, on the tag just read; KEPT was never written.
        let modifies = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path().ends_with("/records/modify"))
            .collect::<Vec<_>>();
        assert_eq!(modifies.len(), 1);
        assert!(!String::from_utf8_lossy(&modifies[0].body).contains("KEPT"));
    }

    fn master_named(name: &str) -> Value {
        let mut m = master();
        m["recordName"] = json!(name);
        m
    }

    fn asset_named(name: &str, master: &str) -> Value {
        let mut a = asset(json!({}));
        a["recordName"] = json!(name);
        a["fields"]["masterRef"] = json!({"value": {"recordName": master}});
        a
    }

    fn ids(lib: &quince_core::Library) -> Vec<String> {
        let mut ids: Vec<String> = lib
            .photos(quince_core::photos::View::Library)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        ids.sort();
        ids
    }

    /// A library too large for one pass is read over several: each pass is stored as it
    /// comes and the next carries on from where it stopped, an asset whose master fell in
    /// the other pass is still made whole, and only once the read reaches the end is what it
    /// never saw let go.
    #[test]
    fn a_large_library_is_read_over_several_passes() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let before = |records| PhotosFetch {
            records,
            token: Some("T0".into()),
            from: None,
            incremental: false,
            complete: true,
            zone: None,
        };
        apply(
            &mut lib,
            before(vec![master_named("MX"), asset_named("AX", "MX")]),
        )
        .unwrap();
        assert_eq!(ids(&lib), ["AX"]);

        // The daily whole read begins, and stops partway with a master's asset still to come.
        let first = PhotosFetch {
            records: vec![
                master_named("M1"),
                asset_named("A1", "M1"),
                master_named("M2"),
            ],
            token: Some("C1".into()),
            from: None,
            incremental: false,
            complete: false,
            zone: None,
        };
        apply(&mut lib, first).unwrap();
        assert_eq!(
            ids(&lib),
            ["A1", "AX"],
            "seen so far shows; nothing pruned yet"
        );
        assert_eq!(
            lib.sync_token(SERVICE).unwrap().as_deref(),
            Some("C1"),
            "the next pass carries on from here"
        );

        let second = PhotosFetch {
            records: vec![asset_named("A2", "M2")],
            token: Some("T1".into()),
            from: Some("C1".into()),
            incremental: true,
            complete: true,
            zone: None,
        };
        apply(&mut lib, second).unwrap();
        assert_eq!(
            ids(&lib),
            ["A1", "A2"],
            "the split photo is whole; the one the read never saw is gone"
        );
        assert_eq!(lib.sync_token(SERVICE).unwrap().as_deref(), Some("T1"));
        assert_eq!(
            lib.photo("A2").unwrap().unwrap().filename.as_deref(),
            Some("IMG_0001.JPG")
        );

        // An ordinary pass afterwards prunes nothing.
        let after = PhotosFetch {
            records: vec![master_named("M3"), asset_named("A3", "M3")],
            token: Some("T2".into()),
            from: Some("T1".into()),
            incremental: true,
            complete: true,
            zone: None,
        };
        apply(&mut lib, after).unwrap();
        assert_eq!(ids(&lib), ["A1", "A2", "A3"]);
    }

    /// A pass over what changed that is too long for one pass keeps its place too, rather
    /// than reading the same pages again next time; and it is never taken for a whole read.
    #[test]
    fn a_long_pass_over_changes_keeps_its_place() {
        let mut lib = quince_core::Library::in_memory().unwrap();
        let whole = PhotosFetch {
            records: vec![master_named("M1"), asset_named("A1", "M1")],
            token: Some("T0".into()),
            from: None,
            incremental: false,
            complete: true,
            zone: None,
        };
        apply(&mut lib, whole).unwrap();
        let part = PhotosFetch {
            records: vec![master_named("M2"), asset_named("A2", "M2")],
            token: Some("C1".into()),
            from: Some("T0".into()),
            incremental: true,
            complete: false,
            zone: None,
        };
        apply(&mut lib, part).unwrap();
        assert_eq!(lib.sync_token(SERVICE).unwrap().as_deref(), Some("C1"));
        let rest = PhotosFetch {
            records: vec![],
            token: Some("T1".into()),
            from: Some("C1".into()),
            incremental: true,
            complete: true,
            zone: None,
        };
        apply(&mut lib, rest).unwrap();
        assert_eq!(ids(&lib), ["A1", "A2"], "nothing pruned");
    }

    fn zone_reply(zone: Value) -> wiremock::ResponseTemplate {
        wiremock::ResponseTemplate::new(200).set_body_json(json!({ "zones": [zone] }))
    }

    /// Apple's answer to `CheckIndexingState`, as icloud.com asks it before reading the
    /// library: still preparing (with its progress), finished, or nothing to say.
    #[tokio::test]
    async fn whether_apple_has_the_library_ready() {
        use wiremock::matchers::{body_string_contains, method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let ask = |answer: Value| async move {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path_regex(
                    r".*/com\.apple\.photos\.cloud/production/private/records/query$",
                ))
                .and(body_string_contains("CheckIndexingState"))
                .and(body_string_contains("PrimarySync"))
                .respond_with(ResponseTemplate::new(200).set_body_json(answer))
                .mount(&server)
                .await;
            readiness(&reqwest::Client::new(), &server.uri(), vec![])
                .await
                .unwrap()
        };
        let state = |state: &str, progress: Value| {
            json!({ "records": [{
                "recordName": "_9f4bd1c2", "recordType": "CheckIndexingState",
                "fields": { "state": { "value": state, "type": "STRING" },
                            "progress": { "value": progress, "type": "INT64" } }
            }] })
        };
        assert_eq!(
            ask(state("RUNNING", json!(0))).await,
            Readiness::Preparing { progress: Some(0) }
        );
        assert_eq!(
            ask(state("RUNNING", json!(42))).await,
            Readiness::Preparing { progress: Some(42) }
        );
        assert_eq!(ask(state("FINISHED", json!(100))).await, Readiness::Ready);
        assert_eq!(ask(json!({ "records": [] })).await, Readiness::Ready);
        assert_eq!(
            readiness_of(&[json!({ "fields": { "state": { "value": "QUEUED" } } })]),
            Readiness::Preparing { progress: None }
        );
    }

    /// The whole library as Apple answers a walk from the start.
    async fn library_server(refusal: Value) -> wiremock::MockServer {
        use wiremock::matchers::{body_string_contains, method, path_regex};
        use wiremock::{Mock, MockServer};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .and(body_string_contains("old-token"))
            .respond_with(zone_reply(refusal))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "records": [master_named("M1"), asset_named("A1", "M1"),
                            master_named("M2"), asset_named("A2", "M2")],
                "syncToken": "fresh",
                "moreComing": false
            })))
            .mount(&server)
            .await;
        server
    }

    /// A change token Apple will no longer read (expired, or refused as a bad request) is
    /// dropped and the library read whole: not taken as "nothing changed", and nothing it
    /// still has deleted.
    #[tokio::test]
    async fn a_refused_change_token_reads_the_library_whole() {
        let refusals = [
            json!({"serverErrorCode": "CHANGE_TOKEN_EXPIRED", "reason": "change token expired"}),
            json!({"serverErrorCode": "CHANGE_TOKEN_EXPIRED"}),
            json!({"serverErrorCode": "BAD_REQUEST", "reason": "Unknown sync continuation type"}),
            json!({"serverErrorCode": "BAD_REQUEST", "reason": "Invalid sync token"}),
        ];
        for refusal in refusals {
            let server = library_server(refusal.clone()).await;
            let http = reqwest::Client::new();
            let fetched = fetch(&http, &server.uri(), vec![], Some("old-token"))
                .await
                .unwrap_or_else(|e| panic!("{refusal}: a refused token is recovered from: {e}"));
            assert!(
                !fetched.incremental,
                "{refusal}: a whole read, not a pass over changes"
            );
            assert!(fetched.from.is_none() && fetched.complete);
            assert_eq!(fetched.token.as_deref(), Some("fresh"));

            let mut lib = quince_core::Library::in_memory().unwrap();
            let had = PhotosFetch {
                records: vec![
                    master_named("M1"),
                    asset_named("A1", "M1"),
                    master_named("MX"),
                    asset_named("AX", "MX"),
                ],
                token: Some("old-token".into()),
                from: None,
                incremental: false,
                complete: true,
                zone: None,
            };
            apply(&mut lib, had).unwrap();
            let (changed, _) = apply(&mut lib, fetched).unwrap();
            assert_eq!(changed, 2, "{refusal}: not reported as nothing changed");
            assert_eq!(
                ids(&lib),
                ["A1", "A2"],
                "{refusal}: kept, added, and only the gone one pruned"
            );
            assert_eq!(lib.sync_token(SERVICE).unwrap().as_deref(), Some("fresh"));
        }
    }

    /// Other trouble is not mistaken for a refused token.
    #[tokio::test]
    async fn other_trouble_is_not_a_refused_token() {
        let server = library_server(
            json!({"serverErrorCode": "QUOTA_EXCEEDED", "reason": "Quota exceeded"}),
        )
        .await;
        let http = reqwest::Client::new();
        assert!(fetch(&http, &server.uri(), vec![], Some("old-token"))
            .await
            .is_err());
    }

    /// One pass reads at most `PASS_PAGES` pages and says where it got to, so the next
    /// carries on from there instead of starting over.
    #[tokio::test]
    async fn a_pass_stops_at_its_budget_and_says_where() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer};
        let server = MockServer::start().await;
        // Each page a position further on, as Apple's are.
        let pages = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = pages.clone();
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(move |_: &wiremock::Request| {
                let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                zone_reply(json!({
                    "records": [master_named("M1")], "syncToken": format!("further-{n}"),
                    "moreComing": true
                }))
            })
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let fetched = fetch(&http, &server.uri(), vec![], None).await.unwrap();
        assert!(!fetched.complete);
        let reached = format!("further-{PASS_PAGES}");
        assert_eq!(fetched.token.as_deref(), Some(reached.as_str()));
        assert_eq!(fetched.records.len(), PASS_PAGES);
        let again = fetch(&http, &server.uri(), vec![], Some(&reached))
            .await
            .unwrap();
        assert!(again.incremental && again.from.as_deref() == Some(reached.as_str()));
    }

    /// A whole walk read the way a refresh reads it — a few pages at a time, each stored
    /// before the next is asked for — ends with every photo, as one long fetch did, and never
    /// holds more than a few pages of records at once.
    #[tokio::test]
    async fn a_walk_read_a_few_pages_at_a_time_ends_whole() {
        use wiremock::matchers::{body_string_contains, method, path_regex};
        use wiremock::{Mock, MockServer};
        const PAGES: usize = 25;
        let server = MockServer::start().await;
        // Page `k` is asked for with the token page `k - 1` gave; the first with none.
        for k in (0..PAGES).rev() {
            let reply = zone_reply(json!({
                "records": [
                    master_named(&format!("M{k}")),
                    asset_named(&format!("A{k}"), &format!("M{k}"))
                ],
                "syncToken": format!("page-{}", k + 1),
                "moreComing": k + 1 < PAGES
            }));
            let mock = Mock::given(method("POST")).and(path_regex(r".*/changes/zone"));
            if k == 0 {
                mock.respond_with(reply).mount(&server).await;
            } else {
                mock.and(body_string_contains(format!("\"page-{k}\"")))
                    .respond_with(reply)
                    .with_priority(1)
                    .mount(&server)
                    .await;
            }
        }
        let http = reqwest::Client::new();
        let mut lib = quince_core::Library::in_memory().unwrap();
        let mut rounds = 0;
        loop {
            rounds += 1;
            let since = lib.sync_token(SERVICE).unwrap();
            let fetched = fetch(&http, &server.uri(), vec![], since.as_deref())
                .await
                .unwrap();
            assert!(
                fetched.records.len() <= 2 * PASS_PAGES,
                "one fetch holds a few pages"
            );
            let complete = fetched.complete;
            apply(&mut lib, fetched).unwrap();
            if complete || rounds >= PASS_ROUNDS {
                break;
            }
        }
        assert_eq!(rounds, PAGES.div_ceil(PASS_PAGES));
        assert_eq!(ids(&lib).len(), PAGES, "every photo, from every page");
        assert_eq!(
            lib.sync_token(SERVICE).unwrap(),
            Some(format!("page-{PAGES}"))
        );
    }
}

#[cfg(test)]
mod zone_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_library_zone_is_primary_sync_where_there_else_the_one_apple_moved_it_to() {
        assert_eq!(
            pick_zone(&names(&["SharedSync-1", "PrimarySync1", "PrimarySync"])),
            Some("PrimarySync")
        );
        assert_eq!(
            pick_zone(&names(&["SharedSync-1", "PrimarySync2", "PrimarySync1"])),
            Some("PrimarySync1")
        );
        assert_eq!(pick_zone(&names(&["SharedSync-1", "Other"])), None);
        assert_eq!(pick_zone(&[]), None);
    }

    fn params(dsid: &str) -> Vec<(String, String)> {
        vec![("dsid".into(), dsid.into())]
    }

    fn page() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({ "zones": [{
            "records": [], "syncToken": "T", "moreComing": false
        }] }))
    }

    /// icloud_photos_downloader#1367: an account whose library Apple moved to `PrimarySync1`
    /// answers ZONE_NOT_FOUND for `PrimarySync`, and the library looked empty. The zone is
    /// found from Apple's list (an entry Apple could not fetch passed over, even one named
    /// like the library), asked once for the account, and every read goes to it.
    #[tokio::test]
    async fn a_library_apple_moved_is_read_from_the_zone_it_names() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(
                r".*/com\.apple\.photos\.cloud/production/private/zones/list$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "zones": [
                { "zoneID": { "zoneName": "PrimarySync" },
                  "serverErrorCode": "ZONE_NOT_FOUND", "reason": "zone not found" },
                { "zoneID": { "zoneName": "PrimarySync1", "zoneType": "REGULAR_CUSTOM_ZONE" },
                  "syncToken": "Z" },
                { "zoneID": { "zoneName": "SharedSync-ABC", "zoneType": "REGULAR_CUSTOM_ZONE" } }
            ] })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .and(body_string_contains("\"zoneName\":\"PrimarySync1\""))
            .respond_with(page())
            .expect(2)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let root = server.uri();
        for _ in 0..2 {
            let got = fetch(&http, &root, params("moved"), None).await.unwrap();
            assert!(got.complete);
        }
        let client = ZoneClient::new(&http, &root, CONTAINER, params("moved"));
        assert_eq!(library_zone(&client).await, "PrimarySync1", "remembered");
    }

    /// Whether the library is ready is asked of the zone it is in: an account whose library
    /// Apple moved (`PrimarySync1`) is asked there, never of a `PrimarySync` it does not have.
    #[tokio::test]
    async fn readiness_is_asked_of_the_library_s_own_zone() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/zones/list$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "zones": [
                { "zoneID": { "zoneName": "PrimarySync1", "zoneType": "REGULAR_CUSTOM_ZONE" } }
            ] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/query$"))
            .and(body_string_contains("\"zoneName\":\"PrimarySync1\""))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "records": [{
                "recordType": "CheckIndexingState",
                "fields": { "state": { "value": "RUNNING" }, "progress": { "value": 7 } }
            }] })),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            readiness(&reqwest::Client::new(), &server.uri(), vec![])
                .await
                .unwrap(),
            Readiness::Preparing { progress: Some(7) }
        );
    }

    /// No list to be had: `PrimarySync`, as every library was until Apple began moving some,
    /// and the list is asked for again next time rather than the failure remembered. A list
    /// naming no library zone is remembered as `PrimarySync`.
    #[tokio::test]
    async fn without_a_list_the_library_is_read_from_primary_sync() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/zones/list$"))
            .respond_with(ResponseTemplate::new(500))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .and(body_string_contains("\"zoneName\":\"PrimarySync\""))
            .respond_with(page())
            .expect(2)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let root = server.uri();
        for _ in 0..2 {
            fetch(&http, &root, params("unlisted"), None).await.unwrap();
        }

        let other = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/zones/list$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "zones": [
                { "zoneID": { "zoneName": "SharedSync-ABC" } }
            ] })))
            .expect(1)
            .mount(&other)
            .await;
        let other_root = other.uri();
        for _ in 0..2 {
            let client = ZoneClient::new(&http, &other_root, CONTAINER, params("a"));
            assert_eq!(library_zone(&client).await, DEFAULT_ZONE);
        }
    }
}
