//! Reminder details beyond the reminder record's own fields: tags, a link, early alerts,
//! custom smart lists, and moving a reminder between a list's sections.
//!
//! What Apple's records look like, read from a real account (842 reminders) and from
//! icloud.com's own Reminders bundle:
//!
//! - **Tags.** A `Hashtag` record (`Hashtag/<UUID>`: `Name`, `Reminder` reference, `Deleted`),
//!   one per tag per reminder. The reminder lists the bare ids in `HashtagIDs`. There is no
//!   account-wide tag record (`HashtagLabel` is not a record type): the tags on the account are
//!   the names across its live `Hashtag` records. `Name` carries no `#`.
//! - **URL.** An `Attachment` record (`Attachment/<UUID>`: `Type` "URL", `UTI`, `URL` as
//!   base64 bytes, `Reminder` reference), listed by bare id in the reminder's `AttachmentIDs`.
//!   `File` and `Image` attachments are the same record with a file asset instead.
//! - **Early alert.** JSON in the reminder's own `DueDateDeltaAlertsData` (ENCRYPTED_BYTES):
//!   `{accountIdentifier, reminderIdentifier, minimumSupportedVersion: 20230430,
//!   dueDateDeltaAlerts: [{identifier, dueDateDeltaCount: -30, dueDateDeltaUnit: 0,
//!   creationDate, minimumSupportedAppVersion: 0}]}` — 49 reminders carry one, all "30 early".
//! - **Smart lists.** `SmartList` records: built-in ones carry only `SmartListType`
//!   (`com.apple.reminders.smartlist.today`, `.flagged`, `.assigned`…); a custom one also
//!   `Name`, `Color`, `BadgeEmblem` and `FilterData`, JSON such as `{"priorities":["medium"]}`.
//! - **Sections.** `ListSection` records (`DisplayName`, `List`, `CreationDate`); which reminder
//!   sits in which is the list's `MembershipsOfRemindersInSectionsAsData` file —
//!   `{minimumSupportedVersion, memberships: [{memberID, groupID?, modifiedOn}]}`, an entry with
//!   no `groupID` being a reminder taken out of every section — beside a
//!   `MembershipsOfRemindersInSectionsChecksum` whose token is what devices merge on.
//!
//! Every write here is behind [`REMINDER_DETAIL_WRITES`].

use crate::cloudkit::{field, ZoneClient};
use crate::{Error, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use quince_core::reminders::{
    EarlyAlert, ReminderAttachment, ReminderHashtag, ReminderSection, ReminderSmartList,
};
use serde_json::{json, Value};

const CONTAINER: &str = "com.apple.reminders";
const ZONE: &str = "Reminders";

/// Writes of tags, links, early alerts and sections. On: each was tried once, supervised, on
/// his account (2026-10-22) and seen on his iPhone as written — a tag added and removed, a
/// link changed, a 15-minute early alert, a section made, renamed, a reminder moved into it,
/// and the section removed. While off, every write function refuses before the network.
pub const REMINDER_DETAIL_WRITES: bool = true;

/// The version Apple writes into each of these JSON blobs, as measured.
const MINIMUM_SUPPORTED_VERSION: i64 = 20_230_430;

/// Seconds between the Unix epoch and Apple's.
const APPLE_EPOCH: f64 = 978_307_200.0;

fn gate(enabled: bool) -> Result<()> {
    if enabled {
        Ok(())
    } else {
        Err(Error::Refused(
            "Skykeep can't change this yet: writing it is still being checked against Apple's apps."
                .into(),
        ))
    }
}

fn writes() -> Result<()> {
    gate(REMINDER_DETAIL_WRITES)
}

fn apple_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
        - APPLE_EPOCH
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn bare(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

fn deleted(record: &Value) -> bool {
    field::flag(record, "Deleted")
        || record
            .get("deleted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// A string field that may be plain or base64 bytes (`URL` is bytes on icloud.com's model).
fn text_or_bytes(record: &Value, name: &str) -> Option<String> {
    let v = record.pointer(&format!("/fields/{name}"))?;
    let raw = v.get("value")?.as_str()?;
    let is_bytes = v
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|t| t.ends_with("BYTES"));
    if is_bytes {
        B64.decode(raw.trim())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    } else {
        Some(raw.to_string())
    }
}

/// A record's list of bare ids, such as `HashtagIDs`; `EMPTY_LIST` and absence are empty.
#[must_use]
pub fn id_list(record: &Value, name: &str) -> Vec<String> {
    record
        .pointer(&format!("/fields/{name}/value"))
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------

/// A live `Hashtag` record as a tag, or `None`.
#[must_use]
pub fn hashtag_from_record(record: &Value) -> Option<ReminderHashtag> {
    if deleted(record) {
        return None;
    }
    let name = field::string(record, "Name")?;
    let name = name.trim().trim_start_matches('#').trim().to_string();
    (!name.is_empty()).then_some(())?;
    Some(ReminderHashtag {
        guid: field::record_name(record)?,
        reminder_guid: field::reference(record, "Reminder")?,
        name,
    })
}

/// A live `Attachment` record, its link read where it is a URL one.
#[must_use]
pub fn attachment_from_record(record: &Value) -> Option<ReminderAttachment> {
    if deleted(record) {
        return None;
    }
    let kind = field::string(record, "Type").unwrap_or_else(|| "File".into());
    Some(ReminderAttachment {
        guid: field::record_name(record)?,
        reminder_guid: field::reference(record, "Reminder")?,
        url: (kind == "URL")
            .then(|| text_or_bytes(record, "URL"))
            .flatten()
            .map(|u| u.trim().to_string())
            .filter(|u| !u.is_empty()),
        uti: field::string(record, "UTI"),
        kind,
    })
}

/// A reminder's early alerts and the account id Apple writes beside them. A reminder with
/// none, or a blob that does not read, has none.
#[must_use]
pub fn early_alerts_from_record(record: &Value) -> (Option<String>, Vec<EarlyAlert>) {
    let Some(blob) = early_alert_blob(record) else {
        return (None, Vec::new());
    };
    let account = blob
        .get("accountIdentifier")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let alerts = blob
        .get("dueDateDeltaAlerts")
        .and_then(Value::as_array)
        .map(|alerts| {
            alerts
                .iter()
                .filter_map(|a| {
                    Some(EarlyAlert {
                        id: a.get("identifier")?.as_str()?.to_string(),
                        count: a.get("dueDateDeltaCount")?.as_i64()?.abs(),
                        unit: a.get("dueDateDeltaUnit")?.as_i64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (account, alerts)
}

fn early_alert_blob(record: &Value) -> Option<Value> {
    let raw = field::string(record, "DueDateDeltaAlertsData")?;
    let bytes = B64.decode(raw.trim()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A `SmartList` record, built-in or custom, or `None` once deleted.
#[must_use]
pub fn smart_list_from_record(record: &Value) -> Option<ReminderSmartList> {
    if deleted(record) {
        return None;
    }
    let kind = field::string(record, "SmartListType")?;
    let kind = kind.rsplit('.').next().unwrap_or(&kind).to_string();
    let filter = field::string(record, "FilterData")
        .and_then(|b| B64.decode(b.trim()).ok())
        .and_then(|b| String::from_utf8(b).ok());
    Some(ReminderSmartList {
        guid: field::record_name(record)?,
        kind,
        title: field::string(record, "Name"),
        colour: field::string(record, "Color").and_then(|c| {
            serde_json::from_str::<Value>(&c)
                .ok()
                .and_then(|v| v.get("daHexString")?.as_str().map(str::to_ascii_uppercase))
        }),
        emblem: field::string(record, "BadgeEmblem").filter(|e| !e.trim().is_empty()),
        pinned: record
            .pointer("/fields/PinnedDate/value")
            .and_then(Value::as_i64)
            .is_some_and(|ms| ms > 0),
        sorting: field::string(record, "SortingStyle"),
        filter,
    })
}

// ---------------------------------------------------------------------------------------
// Pure builders, for the writes below
// ---------------------------------------------------------------------------------------

/// A tag name as Apple stores it: no `#`, no spaces (the phone ends a tag at one).
pub fn tag_name(name: &str) -> Result<String> {
    let name = name.trim().trim_start_matches('#').trim();
    if name.is_empty() {
        return Err(Error::Refused("A tag needs a name.".into()));
    }
    if name.chars().any(char::is_whitespace) || name.contains('#') {
        return Err(Error::Refused(
            "A tag is one word: no spaces, and only the one # at the start.".into(),
        ));
    }
    if name.chars().count() > 100 {
        return Err(Error::Refused("That tag is too long.".into()));
    }
    Ok(name.to_string())
}

/// A link as it will be stored: trimmed, with a scheme, no spaces.
pub fn link(url: &str) -> Result<String> {
    let url = url.trim();
    let scheme_ok = url.split_once(':').is_some_and(|(scheme, rest)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
            && !rest.is_empty()
    });
    if !scheme_ok || url.chars().any(char::is_whitespace) || url.len() > 4096 {
        return Err(Error::Refused(
            "That isn't a link Reminders can open (it needs to start like https://).".into(),
        ));
    }
    Ok(url.to_string())
}

/// The new `DueDateDeltaAlertsData` JSON: what the reminder had, with its alerts replaced by
/// `alert` (or none). Keys Apple added that this does not know are kept as they were.
#[must_use]
pub fn early_alert_json(
    existing: Option<&Value>,
    account: &str,
    reminder_guid: &str,
    alert: Option<(i64, i64)>,
    now_apple: f64,
) -> Value {
    let mut blob = existing
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    blob["accountIdentifier"] = json!(account);
    blob["reminderIdentifier"] = json!(bare(reminder_guid));
    if blob.get("minimumSupportedVersion").is_none() {
        blob["minimumSupportedVersion"] = json!(MINIMUM_SUPPORTED_VERSION);
    }
    blob["dueDateDeltaAlerts"] = match alert {
        None => json!([]),
        Some((count, unit)) => json!([{
            "identifier": new_id(),
            "dueDateDeltaCount": -count.abs(),
            "dueDateDeltaUnit": unit,
            "creationDate": now_apple,
            "minimumSupportedAppVersion": 0,
        }]),
    };
    blob
}

/// The list's membership file with one reminder placed in `section` (a section record name),
/// or out of every one. Only that reminder's entry is written: every other entry, and every
/// other byte of the file Apple wrote — its key order, its numbers as Apple formatted them —
/// is kept exactly ([`splice_memberships`]). The entry changed carries `modified_on` (Apple's
/// seconds), which is what devices merge by.
pub fn place_in_section(
    file: Option<&[u8]>,
    reminder_guid: &str,
    section: Option<&str>,
    modified_on: f64,
) -> Result<Vec<u8>> {
    let refused = || {
        Error::Refused(
            "This list's sections are in a form Skykeep doesn't read, so nothing was moved.".into(),
        )
    };
    let original = file.filter(|b| !b.is_empty());
    let mut doc: Value = match original {
        None => json!({ "minimumSupportedVersion": MINIMUM_SUPPORTED_VERSION, "memberships": [] }),
        Some(bytes) => serde_json::from_slice(bytes).map_err(|_| refused())?,
    };
    if !doc.is_object() {
        return Err(refused());
    }
    if doc.get("memberships").is_some_and(|m| !m.is_array()) {
        return Err(refused());
    }
    if doc.get("memberships").is_none() {
        doc["memberships"] = json!([]);
    }
    let member = bare(reminder_guid).to_string();
    let entries = doc["memberships"]
        .as_array_mut()
        .expect("checked an array above");
    let at = entries.iter().position(|m| {
        m.get("memberID")
            .and_then(Value::as_str)
            .is_some_and(|id| id.eq_ignore_ascii_case(&member))
    });
    let mut entry = at.map_or_else(|| json!({ "memberID": member }), |i| entries[i].clone());
    if !entry.is_object() {
        return Err(refused());
    }
    match section {
        Some(s) => entry["groupID"] = json!(bare(s)),
        None => {
            if let Some(o) = entry.as_object_mut() {
                o.remove("groupID");
            }
        }
    }
    entry["modifiedOn"] = json!(modified_on);
    match at {
        Some(i) => entries[i] = entry.clone(),
        None => entries.push(entry.clone()),
    }
    let Some(bytes) = original else {
        return serde_json::to_vec(&doc).map_err(|e| Error::Protocol(e.to_string()));
    };
    let (changed, appended) = match at {
        Some(i) => (vec![(i, entry)], None),
        None => (Vec::new(), Some(entry)),
    };
    splice_memberships(bytes, &changed, appended.as_ref(), &doc).ok_or_else(refused)
}

/// Where a membership file's entries sit in its bytes: the `memberships` array's closing
/// bracket and each entry's span, or no array at all (`None` inside). `None` for a file this
/// cannot walk exactly.
struct MembershipSpans {
    /// Index of the array's `]`, or of the object's closing `}` when there is no array.
    close: usize,
    /// Whether the array (or, with no array, the object) already holds something.
    has_items: bool,
    array: bool,
    entries: Vec<(usize, usize)>,
}

fn json_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// Past the string starting at `i` (its opening quote).
fn json_string_end(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'"') {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// Past the value starting at `i`.
fn json_value_end(b: &[u8], i: usize) -> Option<usize> {
    match *b.get(i)? {
        b'"' => json_string_end(b, i),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut j = i;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = json_string_end(b, j)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            let mut j = i;
            while j < b.len() && !matches!(b[j], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
            {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

fn membership_spans(b: &[u8]) -> Option<MembershipSpans> {
    let mut i = json_ws(b, 0);
    if b.get(i) != Some(&b'{') {
        return None;
    }
    i = json_ws(b, i + 1);
    let mut members = 0usize;
    loop {
        if b.get(i) == Some(&b'}') {
            if json_ws(b, i + 1) != b.len() {
                return None;
            }
            return Some(MembershipSpans {
                close: i,
                has_items: members > 0,
                array: false,
                entries: Vec::new(),
            });
        }
        if members > 0 {
            if b.get(i) != Some(&b',') {
                return None;
            }
            i = json_ws(b, i + 1);
        }
        let key_end = json_string_end(b, i)?;
        let key: String = serde_json::from_slice(&b[i..key_end]).ok()?;
        i = json_ws(b, key_end);
        if b.get(i) != Some(&b':') {
            return None;
        }
        i = json_ws(b, i + 1);
        let value_end = json_value_end(b, i)?;
        if key == "memberships" {
            if b.get(i) != Some(&b'[') {
                return None;
            }
            let mut entries = Vec::new();
            let mut j = json_ws(b, i + 1);
            loop {
                if b.get(j) == Some(&b']') {
                    if j + 1 != value_end {
                        return None;
                    }
                    let has_items = !entries.is_empty();
                    return Some(MembershipSpans {
                        close: j,
                        has_items,
                        array: true,
                        entries,
                    });
                }
                if !entries.is_empty() {
                    if b.get(j) != Some(&b',') {
                        return None;
                    }
                    j = json_ws(b, j + 1);
                }
                let end = json_value_end(b, j)?;
                entries.push((j, end));
                j = json_ws(b, end);
            }
        }
        members += 1;
        i = json_ws(b, value_end);
    }
}

/// Apple's membership file with only the named entries rewritten (`changed`, by index) and
/// one appended, every other byte kept exactly. The result must read back as `expected` — the
/// whole edit made on the parsed file — or nothing is answered, so a file this walks wrongly
/// is refused rather than written.
fn splice_memberships(
    bytes: &[u8],
    changed: &[(usize, Value)],
    appended: Option<&Value>,
    expected: &Value,
) -> Option<Vec<u8>> {
    let spans = membership_spans(bytes)?;
    let parsed: Value = serde_json::from_slice(bytes).ok()?;
    let count = parsed
        .get("memberships")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if spans.array && spans.entries.len() != count {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() + 128);
    let mut from = 0usize;
    let mut changed: Vec<&(usize, Value)> = changed.iter().collect();
    changed.sort_by_key(|(i, _)| *i);
    for (i, entry) in changed {
        let (start, end) = *spans.entries.get(*i)?;
        out.extend_from_slice(&bytes[from..start]);
        out.extend(serde_json::to_vec(entry).ok()?);
        from = end;
    }
    if let Some(entry) = appended {
        let text = serde_json::to_vec(entry).ok()?;
        if spans.array {
            match spans.entries.last() {
                Some(&(_, end)) => {
                    out.extend_from_slice(&bytes[from..end]);
                    out.push(b',');
                    from = end;
                }
                None => {
                    out.extend_from_slice(&bytes[from..spans.close]);
                    from = spans.close;
                }
            }
            out.extend(text);
        } else {
            out.extend_from_slice(&bytes[from..spans.close]);
            from = spans.close;
            if spans.has_items {
                out.push(b',');
            }
            out.extend_from_slice(b"\"memberships\":[");
            out.extend(text);
            out.push(b']');
        }
    }
    out.extend_from_slice(&bytes[from..]);
    (serde_json::from_slice::<Value>(&out).ok()? == *expected).then_some(out)
}

/// The checksum written beside the membership file: SHA-512 of its bytes, in hex.
///
/// Apple's own value is 128 hex digits too, but it does not match SHA-512 (or SHA3-512 or
/// BLAKE2b) of the downloaded file under any arrangement tried — so what Apple hashes is not
/// known. Devices merge on the checksum field's resolution token, which this write advances;
/// whether a phone also re-derives the checksum is what the supervised live test has to show.
#[must_use]
pub fn membership_checksum(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha512};
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The record's `ResolutionTokenMap` with only `keys` advanced, as a field value.
///
/// `ZoneClient::update_record_signed` advances a token for every field it writes. A section
/// move writes the file and its checksum, but Apple keeps a token for the checksum alone
/// (`membershipsOfRemindersInSectionsChecksum`), so this stamps exactly that.
#[must_use]
pub fn stamp_only(current: &Value, keys: &[&str], now_apple: f64) -> Value {
    let mut map: Value = current
        .pointer("/fields/ResolutionTokenMap/value")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .filter(|v: &Value| v.get("map").is_some_and(Value::is_object))
        .unwrap_or_else(|| json!({ "map": {} }));
    let writer = crate::topotext::writer_id();
    for key in keys {
        let counter = map["map"][*key]["counter"].as_u64().unwrap_or(0) + 1;
        map["map"][*key] =
            json!({ "counter": counter, "modificationTime": now_apple, "replicaID": writer });
    }
    json!({ "type": "STRING", "value": map.to_string() })
}

// ---------------------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------------------

fn client<'a>(
    http: &'a reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> ZoneClient<'a> {
    ZoneClient::new(http, root, CONTAINER, params)
}

async fn live_reminder(zc: &ZoneClient<'_>, reminder_guid: &str) -> Result<Value> {
    match zc.lookup_record(ZONE, reminder_guid).await? {
        Some(r) if !deleted(&r) => Ok(r),
        _ => Err(Error::Refused("Apple no longer has that reminder.".into())),
    }
}

/// The live records a reminder lists under `ids_field`, each with its record name.
async fn listed(
    zc: &ZoneClient<'_>,
    reminder: &Value,
    ids_field: &str,
    prefix: &str,
) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for id in id_list(reminder, ids_field) {
        if let Some(r) = zc.lookup_record(ZONE, &format!("{prefix}/{id}")).await? {
            if !deleted(&r) {
                out.push(r);
            }
        }
    }
    Ok(out)
}

/// A field change adding `id` to a reminder's id list, or `None` if it is there already.
fn with_id(reminder: &Value, ids_field: &str, id: &str) -> Option<Value> {
    let mut ids = id_list(reminder, ids_field);
    if ids.iter().any(|x| x.eq_ignore_ascii_case(id)) {
        return None;
    }
    ids.push(id.to_string());
    Some(json!({
        ids_field: { "type": "STRING_LIST", "value": ids },
        "LastModifiedDate": { "type": "TIMESTAMP", "value": now_millis() },
    }))
}

/// A field change taking `ids` out of a reminder's id list, or `None` if none are there.
fn without_ids(reminder: &Value, ids_field: &str, ids: &[String]) -> Option<Value> {
    let before = id_list(reminder, ids_field);
    let after: Vec<String> = before
        .iter()
        .filter(|x| !ids.iter().any(|g| g.eq_ignore_ascii_case(x)))
        .cloned()
        .collect();
    (after.len() != before.len()).then(|| {
        json!({
            ids_field: { "type": "STRING_LIST", "value": after },
            "LastModifiedDate": { "type": "TIMESTAMP", "value": now_millis() },
        })
    })
}

/// Tag a reminder. Answers the `Hashtag` record's name — the one already there, if the
/// reminder carries this tag (however capitalised).
///
/// As a repeat is set: the tag record is made first, then the reminder pointed at it, so the
/// reminder never lists a tag that does not exist.
pub async fn add_tag_cloudkit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    reminder_guid: &str,
    name: &str,
) -> Result<String> {
    writes()?;
    let name = tag_name(name)?;
    let zc = client(http, root, params);
    let reminder = live_reminder(&zc, reminder_guid).await?;
    for tag in listed(&zc, &reminder, "HashtagIDs", "Hashtag").await? {
        if field::string(&tag, "Name")
            .is_some_and(|n| n.trim_start_matches('#').eq_ignore_ascii_case(&name))
        {
            if let Some(guid) = field::record_name(&tag) {
                return Ok(guid);
            }
        }
    }
    let id = new_id();
    let guid = format!("Hashtag/{id}");
    zc.create_record(
        ZONE,
        &guid,
        "Hashtag",
        // As a phone writes one (measured on his account, 2026-10-22): the name encrypted,
        // as every name a person types into Reminders is, `Type` 0 (a tag a person typed),
        // `CreationDate`, `Imported` 0. No `ResolutionTokenMap`: Apple's carry none.
        json!({
            "Name": { "type": "STRING", "value": name, "isEncrypted": true },
            "Reminder": zc.reference_to(ZONE, reminder_guid),
            "Type": { "type": "NUMBER_INT64", "value": 0 },
            "CreationDate": { "type": "TIMESTAMP", "value": now_millis() },
            "Deleted": { "type": "NUMBER_INT64", "value": 0 },
            "Imported": { "type": "NUMBER_INT64", "value": 0 },
        }),
    )
    .await?;
    zc.edit_record_signed(ZONE, reminder_guid, "Reminder", |r| {
        with_id(r, "HashtagIDs", &id)
    })
    .await?;
    Ok(guid)
}

/// Take a tag off a reminder: the reminder stops listing it, then the tag record is marked
/// deleted, as Apple's clients mark records.
pub async fn remove_tag_cloudkit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    reminder_guid: &str,
    name: &str,
) -> Result<()> {
    writes()?;
    let name = tag_name(name)?;
    let zc = client(http, root, params);
    let reminder = live_reminder(&zc, reminder_guid).await?;
    let matching: Vec<String> = listed(&zc, &reminder, "HashtagIDs", "Hashtag")
        .await?
        .iter()
        .filter(|t| {
            field::string(t, "Name")
                .is_some_and(|n| n.trim_start_matches('#').eq_ignore_ascii_case(&name))
        })
        .filter_map(field::record_name)
        .collect();
    if matching.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = matching.iter().map(|g| bare(g).to_string()).collect();
    zc.edit_record_signed(ZONE, reminder_guid, "Reminder", |r| {
        without_ids(r, "HashtagIDs", &ids)
    })
    .await?;
    // Removed as a phone removes one: the Hashtag record deleted outright. Measured on his
    // account (2026-10-22): 253 Hashtag tombstones in the zone and not one Hashtag an Apple
    // device left with `Deleted` 1; a tag only marked `Deleted` 1 still showed on his iPhone.
    for guid in &matching {
        zc.delete_record(ZONE, guid).await?;
    }
    Ok(())
}

/// Set or clear a reminder's link. An existing URL attachment is changed in place; a new one
/// is made and then listed on the reminder; clearing unlists it and marks it deleted.
pub async fn set_url_cloudkit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    reminder_guid: &str,
    url: Option<&str>,
) -> Result<()> {
    writes()?;
    let url = url.map(link).transpose()?;
    let zc = client(http, root, params);
    let reminder = live_reminder(&zc, reminder_guid).await?;
    let existing: Vec<String> = listed(&zc, &reminder, "AttachmentIDs", "Attachment")
        .await?
        .iter()
        .filter(|a| field::string(a, "Type").as_deref() == Some("URL"))
        .filter_map(field::record_name)
        .collect();
    // An encrypted STRING, as the phone writes it (measured on his account, 2026-10-22: a
    // URL attachment made on an iPhone). Bytes here would not be the field Apple reads.
    let url_field = |u: &str| json!({ "type": "STRING", "value": u, "isEncrypted": true });
    match (url, existing.first()) {
        (Some(u), Some(guid)) => {
            zc.update_record(
                ZONE,
                guid,
                "Attachment",
                None,
                json!({ "URL": url_field(&u) }),
            )
            .await?;
        }
        (Some(u), None) => {
            let id = new_id();
            zc.create_record(
                ZONE,
                &format!("Attachment/{id}"),
                "Attachment",
                json!({
                    "Type": { "type": "STRING", "value": "URL" },
                    "UTI": { "type": "STRING", "value": "public.url" },
                    "URL": url_field(&u),
                    "Reminder": zc.reference_to(ZONE, reminder_guid),
                    "Deleted": { "type": "NUMBER_INT64", "value": 0 },
                    "Imported": { "type": "NUMBER_INT64", "value": 0 },
                }),
            )
            .await?;
            zc.edit_record_signed(ZONE, reminder_guid, "Reminder", |r| {
                with_id(r, "AttachmentIDs", &id)
            })
            .await?;
        }
        (None, _) if existing.is_empty() => {}
        (None, _) => {
            let ids: Vec<String> = existing.iter().map(|g| bare(g).to_string()).collect();
            zc.edit_record_signed(ZONE, reminder_guid, "Reminder", |r| {
                without_ids(r, "AttachmentIDs", &ids)
            })
            .await?;
            for guid in &existing {
                zc.update_record(
                    ZONE,
                    guid,
                    "Attachment",
                    None,
                    json!({ "Deleted": { "type": "NUMBER_INT64", "value": 1 } }),
                )
                .await?;
            }
        }
    }
    Ok(())
}

/// Set a reminder's early alert — `count` units before it is due (unit 0 minutes, 1 hours,
/// 2 days, 3 weeks, 4 months; only minutes is measured) — or clear it with `None`.
///
/// Apple writes an account id inside the alert. It is taken from the reminder's own alert
/// if it has one, else `account` (from any other reminder's, see
/// `Library::early_alert_account`); with neither, nothing is written.
pub async fn set_early_alert_cloudkit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    reminder_guid: &str,
    alert: Option<(i64, i64)>,
    account: Option<&str>,
) -> Result<()> {
    writes()?;
    if let Some((count, unit)) = alert {
        if count == 0 || count.abs() > 10_000 || !(0..=4).contains(&unit) {
            return Err(Error::Refused(
                "That isn't an early reminder Apple offers.".into(),
            ));
        }
    }
    let zc = client(http, root, params);
    let reminder = live_reminder(&zc, reminder_guid).await?;
    if alert.is_some() && field::int(&reminder, "DueDate").is_none() {
        return Err(Error::Refused(
            "Give this a date before asking to be reminded early.".into(),
        ));
    }
    let account = early_alert_blob(&reminder)
        .and_then(|b| {
            b.get("accountIdentifier")?
                .as_str()
                .map(ToString::to_string)
        })
        .or_else(|| account.map(ToString::to_string))
        .ok_or_else(|| {
            Error::Refused(
                "Set one early reminder on an Apple device first; Skykeep needs what it writes."
                    .into(),
            )
        })?;
    let now = apple_now();
    zc.edit_record_signed(ZONE, reminder_guid, "Reminder", |r| {
        if alert.is_none() && early_alert_blob(r).is_none() {
            return None;
        }
        let blob = early_alert_json(
            early_alert_blob(r).as_ref(),
            &account,
            reminder_guid,
            alert,
            now,
        );
        Some(json!({
            "DueDateDeltaAlertsData": {
                "type": "ENCRYPTED_BYTES",
                "value": B64.encode(blob.to_string().as_bytes()),
            },
            "LastModifiedDate": { "type": "TIMESTAMP", "value": now_millis() },
        }))
    })
    .await?;
    Ok(())
}

/// Make a section in a list. Answers it, to be shown before the next pass.
pub async fn create_section_cloudkit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    list_guid: &str,
    title: &str,
) -> Result<ReminderSection> {
    writes()?;
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Refused("A section needs a name.".into()));
    }
    let zc = client(http, root, params);
    let guid = format!("ListSection/{}", new_id());
    let stamp = now_millis();
    // A phone's new section carries a token for exactly these four (measured on his account:
    // `Deleted` and `Imported` get none, `minimumSupportedVersion` one with no field of its
    // own), so they are stamped and nothing else.
    zc.create_record(
        ZONE,
        &guid,
        "ListSection",
        json!({
            "DisplayName": { "type": "STRING", "value": title, "isEncrypted": true },
            "List": zc.reference_to(ZONE, list_guid),
            "CreationDate": { "type": "TIMESTAMP", "value": stamp },
            "Deleted": { "type": "NUMBER_INT64", "value": 0 },
            "Imported": { "type": "NUMBER_INT64", "value": 0 },
            "ResolutionTokenMap": stamp_only(
                &json!({}),
                &["minimumSupportedVersion", "list", "creationDate", "displayName"],
                apple_now(),
            ),
        }),
    )
    .await?;
    Ok(ReminderSection {
        guid,
        list_guid: list_guid.to_string(),
        title: title.to_string(),
        created_at: Some(stamp / 1000),
    })
}

/// Rename a section.
pub async fn rename_section_cloudkit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    section_guid: &str,
    title: &str,
) -> Result<()> {
    writes()?;
    let title = title.trim();
    if title.is_empty() {
        return Err(Error::Refused("A section needs a name.".into()));
    }
    client(http, root, params)
        .update_record_signed(
            ZONE,
            section_guid,
            "ListSection",
            None,
            json!({ "DisplayName": { "type": "STRING", "value": title, "isEncrypted": true } }),
        )
        .await?;
    Ok(())
}

/// The query string every CloudKit request carries, as `ZoneClient` sends it.
fn query(params: &[(String, String)]) -> Vec<(String, String)> {
    let mut q = params.to_vec();
    q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
    q.push(("ckjsVersion".into(), "2.6.1".into()));
    q
}

/// Upload one file for a record's asset field, as CloudKit JS does: ask for an address
/// (`assets/upload`), post the bytes there, and answer the receipt Apple gives — the value
/// the field is then written with.
async fn upload_asset(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    record_type: &str,
    record_name: &str,
    field_name: &str,
    bytes: Vec<u8>,
) -> Result<Value> {
    let url = format!(
        "{}/database/1/{CONTAINER}/production/private/assets/upload",
        root.trim_end_matches('/')
    );
    let asked = http
        .post(&url)
        .query(&query(params))
        .json(&json!({
            "zoneID": { "zoneName": ZONE, "zoneType": "REGULAR_CUSTOM_ZONE" },
            "tokens": [{ "recordType": record_type, "recordName": record_name, "fieldName": field_name }],
        }))
        .send()
        .await?;
    let status = asked.status();
    if crate::session_lapsed(status) {
        return Err(Error::ReauthRequired);
    }
    let answer: Value =
        serde_json::from_str(&crate::bounded::reply(asked).await?).unwrap_or(Value::Null);
    let to = answer
        .pointer("/tokens/0/url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Protocol(format!("CloudKit gave nowhere to upload to ({status})")))?;
    let size = bytes.len();
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name("file")
        .mime_str("application/octet-stream")
        .map_err(|e| Error::Protocol(e.to_string()))?;
    let sent = http
        .post(to)
        .multipart(reqwest::multipart::Form::new().part("files", part))
        .send()
        .await?;
    let status = sent.status();
    let text = crate::bounded::reply(sent).await?;
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "Apple would not take the file ({status})"
        )));
    }
    let receipt: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("Apple's upload reply was not JSON: {e}")))?;
    let single = receipt
        .get("singleFile")
        .filter(|s| s.get("receipt").is_some())
        .cloned()
        .ok_or_else(|| Error::Protocol("Apple took the bytes but gave no receipt".into()))?;
    if single
        .get("size")
        .and_then(Value::as_u64)
        .is_some_and(|s| s != size as u64)
    {
        return Err(Error::Protocol(
            "Apple received a different file than was sent".into(),
        ));
    }
    Ok(single)
}

const MEMBERSHIP_FILE: &str = "MembershipsOfRemindersInSectionsAsData";
const MEMBERSHIP_SUM: &str = "MembershipsOfRemindersInSectionsChecksum";

/// Move a reminder into a section of its list, or out of every one (`section` `None`).
///
/// The placement is the list's membership file: read as Apple holds it now, this one entry
/// changed, uploaded, and written onto the list over exactly the version read — another
/// device's placement made meanwhile is read again and kept, never overwritten.
pub async fn move_to_section_cloudkit(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    list_guid: &str,
    reminder_guid: &str,
    section: Option<&str>,
) -> Result<()> {
    writes()?;
    let zc = client(http, root, params.clone());
    let reminder = live_reminder(&zc, reminder_guid).await?;
    if field::reference(&reminder, "List").as_deref() != Some(list_guid) {
        return Err(Error::Refused("That reminder isn't in this list.".into()));
    }
    if let Some(section) = section {
        match zc.lookup_record(ZONE, section).await? {
            Some(s)
                if !deleted(&s) && field::reference(&s, "List").as_deref() == Some(list_guid) => {}
            _ => return Err(Error::Refused("That section isn't in this list.".into())),
        }
    }
    edit_memberships(http, root, &params, list_guid, |current, now| {
        place_in_section(current, reminder_guid, section, now).map(Some)
    })
    .await
}

/// The list's membership file with every reminder in `section` taken out of it (its entry
/// kept, without a `groupID`, as Apple keeps a reminder in no section), or `None` when none
/// is in it. Every other entry and key is kept as it was.
pub fn out_of_section(
    file: Option<&[u8]>,
    section: &str,
    modified_on: f64,
) -> Result<Option<Vec<u8>>> {
    let Some(bytes) = file.filter(|b| !b.is_empty()) else {
        return Ok(None);
    };
    let unreadable = || {
        Error::Refused(
            "This list's sections are in a form Skykeep doesn't read, so nothing was changed."
                .into(),
        )
    };
    let mut doc: Value = serde_json::from_slice(bytes).map_err(|_| unreadable())?;
    let group = bare(section);
    let mut changed = Vec::new();
    for (i, entry) in doc
        .get_mut("memberships")
        .and_then(Value::as_array_mut)
        .ok_or_else(unreadable)?
        .iter_mut()
        .enumerate()
    {
        let here = entry
            .get("groupID")
            .and_then(Value::as_str)
            .is_some_and(|g| g.eq_ignore_ascii_case(group));
        if here {
            if let Some(o) = entry.as_object_mut() {
                o.remove("groupID");
                o.insert("modifiedOn".into(), json!(modified_on));
                changed.push((i, entry.clone()));
            }
        }
    }
    if changed.is_empty() {
        return Ok(None);
    }
    splice_memberships(bytes, &changed, None, &doc)
        .map(Some)
        .ok_or_else(unreadable)
}

/// Rewrite a list's membership file: `edit` gets the file as Apple holds it now (or none)
/// and answers the new one, or `None` for no change. Uploaded and written onto the list over
/// exactly the version read; another device's write meanwhile is read again and kept.
async fn edit_memberships(
    http: &reqwest::Client,
    root: &str,
    params: &[(String, String)],
    list_guid: &str,
    edit: impl Fn(Option<&[u8]>, f64) -> Result<Option<Vec<u8>>>,
) -> Result<()> {
    let zc = client(http, root, params.to_vec());
    for _ in 0..5 {
        let Some(list) = zc.lookup_record(ZONE, list_guid).await? else {
            return Err(Error::Refused("That list is no longer in iCloud.".into()));
        };
        let Some(tag) = list.get("recordChangeTag").and_then(Value::as_str) else {
            return Err(Error::Conflict);
        };
        let current = match list
            .pointer(&format!("/fields/{MEMBERSHIP_FILE}/value/downloadURL"))
            .and_then(Value::as_str)
        {
            None => None,
            Some(url) => {
                let got = http.get(url.replace("${f}", "memberships")).send().await?;
                if !got.status().is_success() {
                    return Err(Error::Protocol(format!(
                        "the list's sections did not download ({})",
                        got.status()
                    )));
                }
                Some(got.bytes().await?.to_vec())
            }
        };
        let now = apple_now();
        let Some(file) = edit(current.as_deref(), now)? else {
            return Ok(());
        };
        let sum = membership_checksum(&file);
        let receipt =
            upload_asset(http, root, params, "List", list_guid, MEMBERSHIP_FILE, file).await?;
        let changes = json!({
            MEMBERSHIP_FILE: { "type": "ASSETID", "value": receipt },
            MEMBERSHIP_SUM: { "type": "STRING", "value": sum, "isEncrypted": true },
            "ResolutionTokenMap": stamp_only(&list, &["membershipsOfRemindersInSectionsChecksum"], now),
        });
        match zc
            .update_record_exact(ZONE, list_guid, "List", tag, changes)
            .await
        {
            Err(Error::Conflict) => continue,
            other => return other.map(|_| ()),
        }
    }
    Err(Error::Conflict)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        B64.encode(s.as_bytes())
    }

    /// Apple's own file shape: keys in either order, numbers in 17 significant digits that
    /// a shortest-form writer would shorten.
    const APPLE_FILE: &[u8] = br#"{"minimumSupportedVersion":20230430,"memberships":[{"modifiedOn":776521004.53464794,"memberID":"A4722061-AEE0-4B0A-AC0E-595C5FE268BC"},{"memberID":"BA60D351-AF45-451B-AC56-84D5F30BA1BB","modifiedOn":777393833.63318706},{"memberID":"C0FFEE00-0000-0000-0000-000000000001","groupID":"S1","modifiedOn":785142271.29329395}]}"#;

    /// A no-op edit gives back Apple's bytes exactly.
    #[test]
    fn a_no_op_splice_is_byte_identical() {
        let doc: Value = serde_json::from_slice(APPLE_FILE).unwrap();
        assert_eq!(
            splice_memberships(APPLE_FILE, &[], None, &doc).unwrap(),
            APPLE_FILE
        );
        // Spaced and newlined, too.
        let spaced = b"{ \"memberships\" : [ {\"memberID\":\"A\", \"modifiedOn\":1.10000000000000009} ,\n {\"memberID\":\"B\"} ] , \"x\":\"]}\\\"\" }";
        let doc: Value = serde_json::from_slice(spaced).unwrap();
        assert_eq!(splice_memberships(spaced, &[], None, &doc).unwrap(), spaced);
        // Nothing to take out: nothing to write.
        assert!(out_of_section(Some(APPLE_FILE), "ListSection/S9", 9.0)
            .unwrap()
            .is_none());
    }

    /// Moving one reminder changes only its entry's span: every byte before and after it is
    /// Apple's. A new member is appended after the last entry and nothing else moves.
    #[test]
    fn a_move_changes_only_that_entrys_span() {
        let second = br#"{"memberID":"BA60D351-AF45-451B-AC56-84D5F30BA1BB","modifiedOn":777393833.63318706}"#;
        let start = APPLE_FILE
            .windows(second.len())
            .position(|w| w == second)
            .unwrap();
        let end = start + second.len();
        let out = place_in_section(
            Some(APPLE_FILE),
            "Reminder/BA60D351-AF45-451B-AC56-84D5F30BA1BB",
            Some("ListSection/S2"),
            9.5,
        )
        .unwrap();
        assert_eq!(out[..start], APPLE_FILE[..start]);
        let tail = &APPLE_FILE[end..];
        assert_eq!(out[out.len() - tail.len()..], *tail);
        let entry: Value = serde_json::from_slice(&out[start..out.len() - tail.len()]).unwrap();
        assert_eq!(
            entry,
            json!({"memberID":"BA60D351-AF45-451B-AC56-84D5F30BA1BB","groupID":"S2","modifiedOn":9.5})
        );

        // A new member: Apple's text up to the array's end, then the one entry.
        let close = APPLE_FILE.len() - 2;
        let added = place_in_section(
            Some(APPLE_FILE),
            "Reminder/NEW",
            Some("ListSection/S2"),
            9.5,
        )
        .unwrap();
        assert_eq!(added[..close], APPLE_FILE[..close]);
        assert_eq!(
            &added[close..],
            br#",{"groupID":"S2","memberID":"NEW","modifiedOn":9.5}]}"#
        );

        // Out of a section: only the entries in it change.
        let s1 = br#"{"memberID":"C0FFEE00-0000-0000-0000-000000000001","groupID":"S1","modifiedOn":785142271.29329395}"#;
        let at = APPLE_FILE.windows(s1.len()).position(|w| w == s1).unwrap();
        let out = out_of_section(Some(APPLE_FILE), "ListSection/S1", 10.0)
            .unwrap()
            .unwrap();
        assert_eq!(out[..at], APPLE_FILE[..at]);
        assert_eq!(
            &out[at..],
            br#"{"memberID":"C0FFEE00-0000-0000-0000-000000000001","modifiedOn":10.0}]}"#
        );
    }

    /// Removing a section takes every reminder in it out of it — the entry kept, without a
    /// `groupID`, as Apple keeps a reminder in no section — and touches nothing else.
    #[test]
    fn removing_a_section_takes_its_reminders_out_of_it() {
        let file = br#"{"minimumSupportedVersion":20230430,"memberships":[
            {"memberID":"A","groupID":"S1","modifiedOn":1.5},
            {"memberID":"B","groupID":"S2","modifiedOn":2.5},
            {"memberID":"C","modifiedOn":3.5}]}"#;
        let out = out_of_section(Some(file), "ListSection/S1", 9.0)
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["minimumSupportedVersion"], 20_230_430);
        let m = v["memberships"].as_array().unwrap();
        assert_eq!(m[0], json!({ "memberID": "A", "modifiedOn": 9.0 }));
        assert_eq!(
            m[1],
            json!({ "memberID": "B", "groupID": "S2", "modifiedOn": 2.5 })
        );
        assert_eq!(m[2], json!({ "memberID": "C", "modifiedOn": 3.5 }));
        // Nobody in it: nothing to write.
        assert!(out_of_section(Some(file), "ListSection/S9", 9.0)
            .unwrap()
            .is_none());
        assert!(out_of_section(None, "ListSection/S1", 9.0)
            .unwrap()
            .is_none());
    }

    /// The gate refuses when off; it is on since the supervised live test (2026-10-22).
    #[test]
    fn the_gate_refuses_when_off() {
        const { assert!(REMINDER_DETAIL_WRITES) };
        assert!(matches!(gate(false), Err(Error::Refused(_))));
        assert!(gate(true).is_ok());
    }

    #[test]
    fn a_hashtag_record_reads_as_a_tag() {
        let r = json!({ "recordName": "Hashtag/H1", "recordType": "Hashtag", "fields": {
            "Name": { "type": "STRING", "value": "#Errands", "isEncrypted": true },
            "Reminder": { "type": "REFERENCE", "value": { "recordName": "Reminder/R1" } },
            "Deleted": { "type": "NUMBER_INT64", "value": 0 } } });
        assert_eq!(
            hashtag_from_record(&r),
            Some(ReminderHashtag {
                guid: "Hashtag/H1".into(),
                reminder_guid: "Reminder/R1".into(),
                name: "Errands".into()
            })
        );
        let mut gone = r.clone();
        gone["fields"]["Deleted"]["value"] = json!(1);
        assert_eq!(hashtag_from_record(&gone), None);
    }

    /// The link is bytes on icloud.com's model; a plain string reads too.
    #[test]
    fn a_url_attachment_reads_its_link() {
        let r = json!({ "recordName": "Attachment/A", "recordType": "Attachment", "fields": {
            "Type": { "type": "STRING", "value": "URL" },
            "UTI": { "type": "STRING", "value": "public.url" },
            "URL": { "type": "ENCRYPTED_BYTES", "value": b64("https://apple.com/x?y=1") },
            "Reminder": { "type": "REFERENCE", "value": { "recordName": "Reminder/R1" } } } });
        let a = attachment_from_record(&r).unwrap();
        assert_eq!(a.url.as_deref(), Some("https://apple.com/x?y=1"));
        assert_eq!(a.kind, "URL");
        let mut plain = r.clone();
        plain["fields"]["URL"] = json!({ "type": "STRING", "value": "https://a.b" });
        assert_eq!(
            attachment_from_record(&plain).unwrap().url.as_deref(),
            Some("https://a.b")
        );
        let mut image = r;
        image["fields"]["Type"]["value"] = json!("Image");
        assert_eq!(attachment_from_record(&image).unwrap().url, None);
    }

    /// The shape measured on 49 reminders.
    #[test]
    fn the_measured_early_alert_reads() {
        let blob = r#"{"accountIdentifier":"6D05-ACCT","minimumSupportedVersion":20230430,"reminderIdentifier":"9C68","dueDateDeltaAlerts":[{"identifier":"10A8","dueDateDeltaCount":-30,"creationDate":803667432.61867201,"dueDateDeltaUnit":0,"minimumSupportedAppVersion":0}]}"#;
        let r = json!({ "fields": { "DueDateDeltaAlertsData": { "type": "ENCRYPTED_BYTES", "value": b64(blob) } } });
        let (account, alerts) = early_alerts_from_record(&r);
        assert_eq!(account.as_deref(), Some("6D05-ACCT"));
        assert_eq!(
            alerts,
            vec![EarlyAlert {
                id: "10A8".into(),
                count: 30,
                unit: 0
            }]
        );
        assert_eq!(
            early_alerts_from_record(&json!({ "fields": {} })),
            (None, vec![])
        );
    }

    #[test]
    fn an_early_alert_is_written_in_apples_shape_keeping_unknown_keys() {
        let existing = json!({ "accountIdentifier": "OLD", "minimumSupportedVersion": 20230430,
            "somethingNew": 7, "dueDateDeltaAlerts": [{ "identifier": "X" }] });
        let blob = early_alert_json(Some(&existing), "ACCT", "Reminder/R1", Some((15, 0)), 800.5);
        assert_eq!(blob["accountIdentifier"], "ACCT");
        assert_eq!(blob["reminderIdentifier"], "R1");
        assert_eq!(blob["somethingNew"], 7);
        let a = &blob["dueDateDeltaAlerts"][0];
        assert_eq!(a["dueDateDeltaCount"], -15);
        assert_eq!(a["dueDateDeltaUnit"], 0);
        assert_eq!(a["creationDate"], 800.5);
        assert_eq!(a["minimumSupportedAppVersion"], 0);
        assert_eq!(blob["dueDateDeltaAlerts"].as_array().unwrap().len(), 1);
        let cleared = early_alert_json(None, "ACCT", "Reminder/R1", None, 1.0);
        assert_eq!(cleared["dueDateDeltaAlerts"], json!([]));
        assert_eq!(cleared["minimumSupportedVersion"], 20230430);
    }

    #[test]
    fn smart_lists_read_built_in_and_custom() {
        let custom = json!({ "recordName": "SmartList/C", "recordType": "SmartList", "fields": {
            "SmartListType": { "type": "STRING", "value": "com.apple.reminders.smartlist.custom" },
            "Name": { "type": "STRING", "value": "Medium", "isEncrypted": true },
            "Color": { "type": "STRING", "value": "{\"daHexString\":\"#ff9500\"}" },
            "FilterData": { "type": "ENCRYPTED_BYTES", "value": b64(r#"{"priorities":["medium"]}"#) },
            "Deleted": { "type": "NUMBER_INT64", "value": 0 } } });
        let s = smart_list_from_record(&custom).unwrap();
        assert_eq!(s.kind, "custom");
        assert_eq!(s.title.as_deref(), Some("Medium"));
        assert_eq!(s.colour.as_deref(), Some("#FF9500"));
        assert_eq!(s.filter.as_deref(), Some(r#"{"priorities":["medium"]}"#));
        let today = json!({ "recordName": "SmartList/T", "recordType": "SmartList", "fields": {
            "SmartListType": { "type": "STRING", "value": "com.apple.reminders.smartlist.today" },
            "PinnedDate": { "type": "TIMESTAMP", "value": 1_700_000_000_000_i64 } } });
        let t = smart_list_from_record(&today).unwrap();
        assert_eq!((t.kind.as_str(), t.pinned, t.filter), ("today", true, None));
    }

    #[test]
    fn tag_names_and_links_are_checked() {
        assert_eq!(tag_name(" #Errands ").unwrap(), "Errands");
        assert!(tag_name("#").is_err());
        assert!(tag_name("two words").is_err());
        assert!(tag_name("a#b").is_err());
        assert_eq!(link(" https://apple.com ").unwrap(), "https://apple.com");
        assert!(link("mailto:x@y.z").is_ok());
        assert!(link("apple.com").is_err());
        assert!(link("https://a b").is_err());
    }

    /// The measured file: a placement changed in place, another kept, a new one added, and
    /// one taken out keeps its entry without a groupID — as Apple's own files do.
    #[test]
    fn a_placement_changes_one_entry_and_keeps_the_rest() {
        let file = br#"{"minimumSupportedVersion":20230430,"future":true,"memberships":[
            {"memberID":"AAA","modifiedOn":1.5},
            {"memberID":"BBB","modifiedOn":2.5,"groupID":"S1"}]}"#;
        let moved: Value = serde_json::from_slice(
            &place_in_section(Some(file), "Reminder/aaa", Some("ListSection/S2"), 9.0).unwrap(),
        )
        .unwrap();
        assert_eq!(moved["future"], true);
        let m = moved["memberships"].as_array().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0]["groupID"], "S2");
        assert_eq!(m[0]["modifiedOn"], 9.0);
        assert_eq!(
            m[1],
            json!({"memberID":"BBB","modifiedOn":2.5,"groupID":"S1"})
        );

        let out: Value = serde_json::from_slice(
            &place_in_section(Some(file), "Reminder/BBB", None, 10.0).unwrap(),
        )
        .unwrap();
        assert_eq!(
            out["memberships"][1],
            json!({"memberID":"BBB","modifiedOn":10.0})
        );

        let fresh: Value = serde_json::from_slice(
            &place_in_section(None, "Reminder/CCC", Some("ListSection/S1"), 3.0).unwrap(),
        )
        .unwrap();
        assert_eq!(fresh["minimumSupportedVersion"], 20230430);
        assert_eq!(fresh["memberships"][0]["memberID"], "CCC");

        assert!(place_in_section(Some(b"not json"), "Reminder/A", None, 1.0).is_err());
    }

    #[test]
    fn only_the_checksum_token_is_advanced() {
        let list = json!({ "fields": { "ResolutionTokenMap": { "type": "STRING", "value":
            r#"{"map":{"membershipsOfRemindersInSectionsChecksum":{"counter":3,"modificationTime":1,"replicaID":"P"},"name":{"counter":2,"modificationTime":1,"replicaID":"P"}}}"# } } });
        let stamped = stamp_only(&list, &["membershipsOfRemindersInSectionsChecksum"], 5.0);
        let map: Value = serde_json::from_str(stamped["value"].as_str().unwrap()).unwrap();
        assert_eq!(
            map["map"]["membershipsOfRemindersInSectionsChecksum"]["counter"],
            4
        );
        assert_eq!(map["map"]["name"]["counter"], 2);
        assert_eq!(map["map"].as_object().unwrap().len(), 2);
        assert_eq!(membership_checksum(b"").len(), 128);
    }

    /// A sync pass carrying every kind lands in the library; a later pass's tombstones and a
    /// reminder sent again without its alert take them away.
    #[test]
    fn a_pass_stores_the_details_and_a_later_one_removes_them() {
        use crate::reminders::{apply_cloudkit, RemindersFetch};
        let alert = r#"{"accountIdentifier":"ACCT","dueDateDeltaAlerts":[{"identifier":"E","dueDateDeltaCount":-30,"dueDateDeltaUnit":0}]}"#;
        let reminder = |alerts: bool| {
            let mut r = json!({ "recordName": "Reminder/R1", "recordType": "Reminder", "fields": {
                "List": { "type": "REFERENCE", "value": { "recordName": "List/L" } },
                "TitleDocument": { "type": "ENCRYPTED_BYTES", "value": crate::ckdocument::text_to_base64("x") },
                "HashtagIDs": { "type": "STRING_LIST", "value": ["H"] } } });
            if alerts {
                r["fields"]["DueDateDeltaAlertsData"] =
                    json!({ "type": "ENCRYPTED_BYTES", "value": b64(alert) });
            }
            r
        };
        let records = vec![
            json!({ "recordName": "List/L", "recordType": "List", "fields": {
                "Name": { "type": "STRING", "value": "L" } } }),
            reminder(true),
            json!({ "recordName": "Hashtag/H", "recordType": "Hashtag", "fields": {
                "Name": { "type": "STRING", "value": "work" },
                "Reminder": { "type": "REFERENCE", "value": { "recordName": "Reminder/R1" } } } }),
            json!({ "recordName": "Attachment/A", "recordType": "Attachment", "fields": {
                "Type": { "type": "STRING", "value": "URL" },
                "URL": { "type": "ENCRYPTED_BYTES", "value": b64("https://apple.com") },
                "Reminder": { "type": "REFERENCE", "value": { "recordName": "Reminder/R1" } } } }),
            json!({ "recordName": "SmartList/S", "recordType": "SmartList", "fields": {
                "SmartListType": { "type": "STRING", "value": "com.apple.reminders.smartlist.custom" },
                "FilterData": { "type": "ENCRYPTED_BYTES", "value": b64(r#"{"hashtags":["work"]}"#) } } }),
        ];
        let mut lib = quince_core::Library::in_memory().unwrap();
        let pass = |records: Vec<Value>, incremental: bool| RemindersFetch {
            records,
            token: None,
            incremental,
            memberships: Default::default(),
        };
        apply_cloudkit(&mut lib, pass(records, false)).unwrap();
        let d = lib.reminder_detail("Reminder/R1").unwrap();
        assert_eq!(d.tags, vec!["work".to_string()]);
        assert_eq!(d.url.as_deref(), Some("https://apple.com"));
        assert_eq!(d.early_alerts.len(), 1);
        let in_list = lib.reminders_in_smart_list("SmartList/S", 0).unwrap();
        assert_eq!(in_list.reminders.len(), 1);
        assert_eq!(lib.reminder_tags().unwrap().len(), 1);

        apply_cloudkit(
            &mut lib,
            pass(
                vec![
                    reminder(false),
                    json!({ "recordName": "Hashtag/H", "deleted": true }),
                    json!({ "recordName": "Attachment/A", "deleted": true }),
                ],
                true,
            ),
        )
        .unwrap();
        let d = lib.reminder_detail("Reminder/R1").unwrap();
        assert!(d.tags.is_empty() && d.url.is_none() && d.early_alerts.is_empty());
        assert_eq!(
            lib.reminder_smart_lists().unwrap().len(),
            1,
            "unchanged, kept"
        );
    }

    mod live_shape {
        use super::*;
        use wiremock::matchers::{body_string_contains, method, path_regex};
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};

        fn reminder() -> Value {
            json!({ "recordName": "Reminder/R1", "recordType": "Reminder", "recordChangeTag": "t1",
                "fields": {
                    "List": { "type": "REFERENCE", "value": { "recordName": "List/L" } },
                    "HashtagIDs": { "type": "EMPTY_LIST", "value": [] },
                    "AttachmentIDs": { "type": "EMPTY_LIST", "value": [] },
                    "DueDate": { "type": "TIMESTAMP", "value": 1_800_000_000_000_i64 },
                    "Deleted": { "type": "NUMBER_INT64", "value": 0 } } })
        }

        async fn lookup(server: &MockServer, name: &str, record: Value) {
            Mock::given(method("POST"))
                .and(path_regex(r".*/records/lookup$"))
                .and(body_string_contains(name))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({ "records": [record] })),
                )
                .mount(server)
                .await;
        }

        async fn modify_ok(server: &MockServer) {
            Mock::given(method("POST"))
                .and(path_regex(r".*/records/modify$"))
                .respond_with(|req: &Request| {
                    let body: Value = serde_json::from_slice(&req.body).unwrap();
                    let name = body["operations"][0]["record"]["recordName"].clone();
                    ResponseTemplate::new(200).set_body_json(
                        json!({ "records": [{ "recordName": name, "recordChangeTag": "t2" }] }),
                    )
                })
                .mount(server)
                .await;
        }

        fn modifies(requests: &[Request]) -> Vec<Value> {
            requests
                .iter()
                .filter(|r| r.url.path().ends_with("/records/modify"))
                .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap()["operations"][0].clone())
                .collect()
        }

        /// The tag record first, then the reminder pointed at its bare id.
        #[tokio::test]
        async fn adding_a_tag_makes_the_record_then_lists_it() {
            let server = MockServer::start().await;
            lookup(&server, "Reminder/R1", reminder()).await;
            modify_ok(&server).await;
            let http = reqwest::Client::new();
            let guid = add_tag_cloudkit(&http, &server.uri(), vec![], "Reminder/R1", "#Errands")
                .await
                .unwrap();
            let ops = modifies(&server.received_requests().await.unwrap());
            assert_eq!(ops[0]["operationType"], "create");
            assert_eq!(ops[0]["record"]["recordType"], "Hashtag");
            assert_eq!(ops[0]["record"]["recordName"], guid);
            assert_eq!(ops[0]["record"]["fields"]["Name"]["value"], "Errands");
            assert_eq!(
                ops[0]["record"]["fields"]["Reminder"]["value"]["recordName"],
                "Reminder/R1"
            );
            assert_eq!(ops[1]["operationType"], "update");
            assert_eq!(
                ops[1]["record"]["recordChangeTag"], "t1",
                "over the version read"
            );
            assert_eq!(
                ops[1]["record"]["fields"]["HashtagIDs"],
                json!({ "type": "STRING_LIST", "value": [bare(&guid)] })
            );
            assert!(
                ops[1]["record"]["fields"]["ResolutionTokenMap"].is_object(),
                "signed"
            );
        }

        /// Removing a tag unlists it from the reminder and deletes its Hashtag record outright,
        /// as a phone does: a tag only marked `Deleted` still shows on an iPhone.
        #[tokio::test]
        async fn removing_a_tag_deletes_its_record() {
            let server = MockServer::start().await;
            let mut r = reminder();
            r["fields"]["HashtagIDs"] = json!({ "type": "STRING_LIST", "value": ["T1", "T2"] });
            lookup(&server, "Reminder/R1", r).await;
            lookup(&server, "Hashtag/T1", json!({ "recordName": "Hashtag/T1", "recordType": "Hashtag",
                "recordChangeTag": "h1", "fields": { "Name": { "type": "STRING", "value": "Errands" },
                "Deleted": { "type": "NUMBER_INT64", "value": 0 } } })).await;
            lookup(
                &server,
                "Hashtag/T2",
                json!({ "recordName": "Hashtag/T2", "recordType": "Hashtag",
                "recordChangeTag": "h2", "fields": { "Name": { "type": "STRING", "value": "Home" },
                "Deleted": { "type": "NUMBER_INT64", "value": 0 } } }),
            )
            .await;
            modify_ok(&server).await;
            let http = reqwest::Client::new();
            remove_tag_cloudkit(&http, &server.uri(), vec![], "Reminder/R1", "#errands")
                .await
                .unwrap();
            let ops = modifies(&server.received_requests().await.unwrap());
            assert_eq!(ops.len(), 2, "{ops:?}");
            assert_eq!(ops[0]["operationType"], "update");
            assert_eq!(
                ops[0]["record"]["fields"]["HashtagIDs"]["value"],
                json!(["T2"])
            );
            assert_eq!(ops[1]["operationType"], "forceDelete");
            assert_eq!(ops[1]["record"]["recordName"], "Hashtag/T1");
            assert!(ops
                .iter()
                .all(|o| o["record"]["recordName"] != "Hashtag/T2"));
        }

        #[tokio::test]
        async fn a_link_is_a_new_url_attachment() {
            let server = MockServer::start().await;
            lookup(&server, "Reminder/R1", reminder()).await;
            modify_ok(&server).await;
            let http = reqwest::Client::new();
            set_url_cloudkit(
                &http,
                &server.uri(),
                vec![],
                "Reminder/R1",
                Some("https://apple.com"),
            )
            .await
            .unwrap();
            let ops = modifies(&server.received_requests().await.unwrap());
            let made = &ops[0]["record"];
            assert_eq!(made["recordType"], "Attachment");
            assert_eq!(made["fields"]["Type"]["value"], "URL");
            // An encrypted string, as a phone writes it, not bytes.
            assert_eq!(
                made["fields"]["URL"],
                json!({ "type": "STRING", "value": "https://apple.com", "isEncrypted": true })
            );
            let id = bare(made["recordName"].as_str().unwrap()).to_string();
            assert_eq!(
                ops[1]["record"]["fields"]["AttachmentIDs"]["value"],
                json!([id])
            );
        }

        #[tokio::test]
        async fn an_early_alert_needs_an_account_id_from_somewhere() {
            let server = MockServer::start().await;
            lookup(&server, "Reminder/R1", reminder()).await;
            modify_ok(&server).await;
            let http = reqwest::Client::new();
            let refused = set_early_alert_cloudkit(
                &http,
                &server.uri(),
                vec![],
                "Reminder/R1",
                Some((30, 0)),
                None,
            )
            .await;
            assert!(matches!(refused, Err(Error::Refused(_))));
            assert!(modifies(&server.received_requests().await.unwrap()).is_empty());

            set_early_alert_cloudkit(
                &http,
                &server.uri(),
                vec![],
                "Reminder/R1",
                Some((30, 0)),
                Some("ACCT"),
            )
            .await
            .unwrap();
            let ops = modifies(&server.received_requests().await.unwrap());
            let sent = ops[0]["record"]["fields"]["DueDateDeltaAlertsData"]["value"]
                .as_str()
                .unwrap();
            let blob: Value = serde_json::from_slice(&B64.decode(sent).unwrap()).unwrap();
            assert_eq!(blob["accountIdentifier"], "ACCT");
            assert_eq!(blob["dueDateDeltaAlerts"][0]["dueDateDeltaCount"], -30);
        }

        /// The list's file downloaded, changed, uploaded, and the list written with the
        /// receipt and its checksum, only the checksum's token advanced.
        #[tokio::test]
        async fn moving_into_a_section_rewrites_the_lists_file() {
            let server = MockServer::start().await;
            lookup(&server, "Reminder/R1", reminder()).await;
            lookup(&server, "ListSection/S1", json!({ "recordName": "ListSection/S1", "recordChangeTag": "s",
                "fields": { "List": { "type": "REFERENCE", "value": { "recordName": "List/L" } } } })).await;
            lookup(
                &server,
                "\"List/L\"",
                json!({ "recordName": "List/L", "recordChangeTag": "l1",
                "fields": {
                    MEMBERSHIP_FILE: { "type": "ASSETID", "value": {
                        "downloadURL": format!("{}/file/${{f}}", server.uri()) } },
                    "ResolutionTokenMap": { "type": "STRING", "value": "{\"map\":{}}" } } }),
            )
            .await;
            Mock::given(method("GET"))
                .and(path_regex(r"^/file/"))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    r#"{"minimumSupportedVersion":20230430,"memberships":[{"memberID":"X","modifiedOn":1.0,"groupID":"S1"}]}"#,
                ))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/assets/upload$"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "tokens": [{ "url": format!("{}/put", server.uri()) }] })))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r"^/put$"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "singleFile": { "receipt": "RCPT", "fileChecksum": "c", "wrappingKey": "w", "referenceChecksum": "r" } })))
                .mount(&server)
                .await;
            modify_ok(&server).await;
            let http = reqwest::Client::new();
            move_to_section_cloudkit(
                &http,
                &server.uri(),
                vec![],
                "List/L",
                "Reminder/R1",
                Some("ListSection/S1"),
            )
            .await
            .unwrap();
            let requests = server.received_requests().await.unwrap();
            let uploaded = requests.iter().find(|r| r.url.path() == "/put").unwrap();
            let body = String::from_utf8_lossy(&uploaded.body);
            assert!(body.contains(r#""memberID":"R1""#) && body.contains(r#""groupID":"S1""#));
            assert!(
                body.contains(r#""memberID":"X""#),
                "the other placement kept"
            );
            let ops = modifies(&requests);
            let fields = &ops[0]["record"]["fields"];
            assert_eq!(ops[0]["record"]["recordChangeTag"], "l1");
            assert_eq!(fields[MEMBERSHIP_FILE]["value"]["receipt"], "RCPT");
            assert_eq!(fields[MEMBERSHIP_SUM]["value"].as_str().unwrap().len(), 128);
            let map: Value =
                serde_json::from_str(fields["ResolutionTokenMap"]["value"].as_str().unwrap())
                    .unwrap();
            assert_eq!(map["map"].as_object().unwrap().len(), 1);
        }

        #[tokio::test]
        async fn a_section_of_another_list_is_refused() {
            let server = MockServer::start().await;
            lookup(&server, "Reminder/R1", reminder()).await;
            lookup(&server, "ListSection/S9", json!({ "recordName": "ListSection/S9", "recordChangeTag": "s",
                "fields": { "List": { "type": "REFERENCE", "value": { "recordName": "List/OTHER" } } } })).await;
            let http = reqwest::Client::new();
            let r = move_to_section_cloudkit(
                &http,
                &server.uri(),
                vec![],
                "List/L",
                "Reminder/R1",
                Some("ListSection/S9"),
            )
            .await;
            assert!(matches!(r, Err(Error::Refused(_))));
        }
    }
}
