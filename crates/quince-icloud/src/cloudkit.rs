//! Reading a CloudKit private-database zone.
//!
//! Three things about this API bite immediately, and all three fail quietly:
//!
//!   * `records/query` is refused for types Apple has not marked indexable — `Reminder` is
//!     one — so `changes/zone` is the way in. It is also the incremental path.
//!   * `desiredKeys: []` asks for *no* fields rather than all of them, which returns records
//!     that look empty.
//!   * Responses page. Ignoring `moreComing` silently truncates someone's data.

use crate::{Error, Result};
use serde_json::{json, Value};

/// Records fetched in one request.
///
/// A real account's zone runs to thousands — mostly tombstones — so a small page turns one
/// refresh into dozens of round trips.
const PAGE_SIZE: u64 = 500;

/// A ceiling on paging, so a server that always says `moreComing` cannot spin forever.
const MAX_PAGES: usize = 200;

/// The most records one lookup asks for: CloudKit's own limit for records/lookup.
const LOOKUP_AT_ONCE: usize = 200;

pub struct ZoneClient<'a> {
    http: &'a reqwest::Client,
    base: String,
    container: String,
    params: Vec<(String, String)>,
    /// Which of the container's databases: the account's own, or what others share with it.
    database: Database,
    /// Whose zone it is, for a zone another account owns (one shared with this one): CloudKit
    /// finds a shared zone only by its name and its owner together.
    owner: Option<String>,
}

/// A CloudKit container's databases this client reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    /// The account's own records.
    Private,
    /// Zones other accounts have shared with this one (an album someone invited you to).
    Shared,
}

impl Database {
    fn path(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shared => "shared",
        }
    }
}

/// One page of zone changes.
pub struct Page {
    pub records: Vec<Value>,
    pub sync_token: Option<String>,
    pub more_coming: bool,
}

/// Why a zone came back with nothing, when it did.
///
/// CloudKit answers all of these with HTTP 200 and puts the trouble inside the zone's own
/// result, so a status check alone says everything went fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZoneTrouble {
    /// The account has no such zone. A new Apple ID that has never opened Notes has no Notes
    /// zone at all — which is an empty library, not a failure.
    Missing,
    /// The cursor was refused. CloudKit expires them, and a client that has been closed for a
    /// while comes back holding one Apple no longer knows. The answer is to start over.
    StaleCursor,
    /// "Not now": the zone throttled, busy with another write, or asking to be tried later,
    /// with the seconds it asked for when it said.
    Busy(Option<u64>),
    /// The session was not accepted for the zone: a fresh sign-in, not a fault in the zone.
    Unauthenticated,
    /// Advanced Data Protection keeps the zone from this session until it is approved on one
    /// of the person's devices (`ACCESS_DENIED`, "private db access disabled").
    Locked,
    /// Something else, which is worth showing to somebody.
    Other(&'static str),
}

fn trouble_with(zone_result: &Value) -> Option<ZoneTrouble> {
    let code = zone_result.get("serverErrorCode").and_then(Value::as_str);
    let reason = zone_result.get("reason").and_then(Value::as_str);
    match code {
        Some("ZONE_NOT_FOUND") => Some(ZoneTrouble::Missing),
        Some("ACCESS_DENIED")
            if reason.is_some_and(|r| {
                r.to_ascii_lowercase()
                    .contains("private db access disabled")
            }) =>
        {
            Some(ZoneTrouble::Locked)
        }
        // An expired change token, with or without a reason. Without this arm a bare
        // `CHANGE_TOKEN_EXPIRED` matched nothing below and read as an empty, finished page:
        // "nothing changed", on a token that would never be read again — every change made
        // elsewhere from then on never arrived.
        Some("CHANGE_TOKEN_EXPIRED") => Some(ZoneTrouble::StaleCursor),
        // Said inside a 200 for the one zone, not as the request's status (timlaing/pyicloud
        // #384): read as a refusal, a moment's throttling failed the whole pass in Apple's
        // words, and a lapsed session read as a broken zone rather than a sign-in.
        Some("THROTTLED" | "TRY_AGAIN_LATER" | "ZONE_BUSY") => Some(ZoneTrouble::Busy(
            zone_result.get("retryAfter").and_then(Value::as_u64),
        )),
        Some("AUTHENTICATION_FAILED" | "AUTHENTICATION_REQUIRED") => {
            Some(ZoneTrouble::Unauthenticated)
        }
        // Apple says BAD_REQUEST for a cursor it will not read, with the reason naming the
        // continuation: "Unknown sync continuation type", "Invalid continuation format".
        Some("BAD_REQUEST")
            if reason.is_some_and(|r| r.to_lowercase().contains("continuation")) =>
        {
            Some(ZoneTrouble::StaleCursor)
        }
        // A reason with no code at all still means the zone was refused; so does a code with
        // no reason — any code at all is Apple saying something went wrong with the zone.
        Some(_) => Some(ZoneTrouble::Other("refused")),
        None if reason.is_some() => Some(ZoneTrouble::Other("refused")),
        _ => None,
    }
}

/// How many times a busy CloudKit is asked before saying so.
const BUSY_TRIES: u32 = 3;

/// A record refused only because the zone was mid-write for someone else — Apple's
/// "Sync zone CAS Op-Lock failed. There was a concurrent write … Retry request", coded
/// ZONE_BUSY or TRY_AGAIN_LATER — measured deleting notes one after another. Nothing was
/// applied; the same change, sent again, goes through.
fn zone_was_busy(code: &str, reason: &str) -> bool {
    matches!(code, "ZONE_BUSY" | "TRY_AGAIN_LATER")
        || reason
            .to_ascii_lowercase()
            .contains("sync zone cas op-lock failed")
}
/// The longest single wait a busy CloudKit is given, whatever it asks for.
const LONGEST_WAIT: u64 = 30;

impl<'a> ZoneClient<'a> {
    pub fn new(
        http: &'a reqwest::Client,
        ckdatabasews_root: &str,
        container: &str,
        params: Vec<(String, String)>,
    ) -> Self {
        Self {
            http,
            base: ckdatabasews_root.trim_end_matches('/').to_string(),
            container: container.to_string(),
            params,
            database: Database::Private,
            owner: None,
        }
    }

    /// The same client, reading `database`.
    #[must_use]
    pub fn in_database(mut self, database: Database) -> Self {
        self.database = database;
        self
    }

    /// The same client, for zones `owner` owns (a zone shared with this account).
    #[must_use]
    pub fn owned_by(mut self, owner: Option<String>) -> Self {
        self.owner = owner;
        self
    }

    /// A zone as CloudKit names it in a request: its name, and its owner where it is someone
    /// else's.
    fn zone_id(&self, zone: &str) -> Value {
        let mut id = json!({ "zoneName": zone, "zoneType": "REGULAR_CUSTOM_ZONE" });
        if let Some(owner) = &self.owner {
            id["ownerRecordName"] = json!(owner);
        }
        id
    }

    /// The address of one of the database's operations (`records/lookup`, `zones/list` …).
    fn url(&self, operation: &str) -> String {
        format!(
            "{}/database/1/{}/production/{}/{operation}",
            self.base,
            self.container,
            self.database.path()
        )
    }

    fn query(&self) -> Vec<(String, String)> {
        let mut q = self.params.clone();
        q.push(("ckjsBuildVersion".into(), "17DProjectDev77".into()));
        q.push(("ckjsVersion".into(), "2.6.1".into()));
        q
    }

    /// One page of changes for `zone`.
    /// POST to CloudKit, waiting out a busy server the way it asks to be waited out.
    ///
    /// Measured on the account: an overloaded CloudKit answers 503 with
    /// `{"serverErrorCode":"SERVER_OVERLOADED","retryAfter":10}`. That went to the screen as
    /// raw JSON and the write was simply lost. It is a request to come back shortly, so it is
    /// honoured — up to `BUSY_TRIES` times, never waiting more than `LONGEST_WAIT` — and only
    /// a server still busy after that is reported, in words.
    async fn post_json(&self, url: &str, body: &Value) -> Result<(reqwest::StatusCode, String)> {
        let mut tries = 0;
        loop {
            tries += 1;
            let response = self
                .http
                .post(url)
                .query(&self.query())
                .json(body)
                .send()
                .await?;
            let status = response.status();
            let header_wait = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            let text = response.text().await.unwrap_or_default();
            // Advanced Data Protection keeping this session out until it is approved on a
            // device (403 ACCESS_DENIED, "private db access disabled"): said as that, for every
            // CloudKit read and write, rather than as Apple's JSON.
            if !status.is_success() {
                crate::approval_needed(status, &text)?;
            }
            // A zone busy with another write answers 409 ZONE_BUSY — "rejected, retry", the
            // whole request not applied — measured when a phone and this app wrote at once.
            // Two of this app's own writes to one zone close together (notes deleted one after
            // another in Recently Deleted) answer 409 TRY_AGAIN_LATER, "Sync zone CAS Op-Lock
            // failed. There was a concurrent write" — the same: nothing applied, ask again.
            // Each is a wait like 503's, not a conflict over the record.
            let zone_busy = status.as_u16() == 409
                && (text.contains("\"ZONE_BUSY\"") || text.contains("\"TRY_AGAIN_LATER\""));
            if !matches!(status.as_u16(), 429 | 503) && !zone_busy {
                return Ok((status, text));
            }
            if tries >= BUSY_TRIES {
                quiet!("cloudkit: still busy after {tries} tries ({status})");
                // Busy, not refused: nothing was applied, and a change waiting to be sent is
                // kept for later rather than dropped as if Apple had turned it down.
                return Err(Error::Busy);
            }
            let asked = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("retryAfter").and_then(Value::as_u64))
                .or(header_wait)
                .unwrap_or(2);
            tokio::time::sleep(std::time::Duration::from_secs(asked.clamp(1, LONGEST_WAIT))).await;
        }
    }

    pub async fn changes(&self, zone: &str, sync_token: Option<&str>) -> Result<Page> {
        let mut zone_body = json!({
            "zoneID": self.zone_id(zone),
            "maxChanges": PAGE_SIZE,
        });
        // An empty string is not a cursor — Apple answers one with a 500. Absent means
        // "from the beginning", which is what an empty one was trying to say anyway.
        if let Some(token) = sync_token.map(str::trim).filter(|t| !t.is_empty()) {
            zone_body["syncToken"] = Value::String(token.to_string());
        }
        // Note: no `desiredKeys`. Sending an empty list asks Apple for no fields at all.

        let url = self.url("changes/zone");
        let mut tries = 0;
        loop {
            tries += 1;
            match self.changes_once(&url, zone, &zone_body).await {
                Err(e) => return Err(e),
                Ok(Ok(page)) => return Ok(page),
                // Waited out as a busy request is (`post_json`), then said in words.
                Ok(Err(asked)) if tries < BUSY_TRIES => {
                    let wait = asked.unwrap_or(2).clamp(1, LONGEST_WAIT);
                    tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
                }
                Ok(Err(_)) => {
                    quiet!("cloudkit: zone {zone} still busy after {tries} tries");
                    return Err(Error::Busy);
                }
            }
        }
    }

    /// One `changes/zone` request: the page, or the wait a busy zone asked for.
    async fn changes_once(
        &self,
        url: &str,
        zone: &str,
        zone_body: &Value,
    ) -> Result<std::result::Result<Page, Option<u64>>> {
        let (status, text) = self
            .post_json(url, &json!({ "zones": [zone_body] }))
            .await?;
        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit zone {zone} returned {status}: {}",
                text.chars().take(200).collect::<String>()
            )));
        }

        let body: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("CloudKit returned invalid JSON: {e}")))?;
        let zone_result = body
            .get("zones")
            .and_then(Value::as_array)
            .and_then(|zones| zones.first())
            .ok_or_else(|| Error::Protocol("CloudKit returned no zone".into()))?;

        match trouble_with(zone_result) {
            // A zone that does not exist is an account that has never used the thing. Reading
            // it as an empty, finished walk is the truth; raising is not.
            Some(ZoneTrouble::Missing) => {
                return Ok(Ok(Page {
                    records: Vec::new(),
                    sync_token: None,
                    more_coming: false,
                }))
            }
            Some(ZoneTrouble::StaleCursor) => return Err(Error::StaleCursor),
            Some(ZoneTrouble::Busy(asked)) => return Ok(Err(asked)),
            Some(ZoneTrouble::Unauthenticated) => return Err(Error::ReauthRequired),
            Some(ZoneTrouble::Locked) => return Err(Error::NeedsDeviceApproval),
            Some(ZoneTrouble::Other(_)) => {
                let reason = zone_result
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("no reason given");
                return Err(Error::Protocol(format!(
                    "CloudKit refused zone {zone}: {reason}"
                )));
            }
            None => {}
        }

        Ok(Ok(Page {
            records: zone_result
                .get("records")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            sync_token: zone_result
                .get("syncToken")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            more_coming: zone_result
                .get("moreComing")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }))
    }

    /// Every record in a zone, following `moreComing`.
    ///
    /// Returns the records and the final sync token, which a later call can resume from so a
    /// refresh fetches only what changed.
    pub async fn all_records(&self, zone: &str, since: Option<&str>) -> Result<ZoneWalk> {
        match self.walk(zone, since).await {
            // CloudKit expires cursors, so an app that has been closed for a while comes back
            // holding one Apple will not read. Starting over is what it expects a client to
            // do, and it is invisible to the person: the library is already on disk, and a
            // full pass replaces it with the same thing plus whatever changed.
            Err(Error::StaleCursor) if since.is_some() => self.walk(zone, None).await,
            other => other,
        }
    }

    async fn walk(&self, zone: &str, since: Option<&str>) -> Result<ZoneWalk> {
        let since = since.map(str::trim).filter(|t| !t.is_empty());
        let mut records = Vec::new();
        let mut token = since.map(ToString::to_string);
        let mut complete = false;

        for page_no in 0..MAX_PAGES {
            let page = self.changes(zone, token.as_deref()).await?;
            // A cursor answered with no position at all was not taken: an expired one can
            // come back as an empty zone result, which read as "nothing changed" for good.
            if page_no == 0 && token.is_some() && page.sync_token.is_none() {
                return Err(Error::StaleCursor);
            }
            records.extend(page.records);
            let (next, advanced) = advance(token.take(), page.sync_token);
            token = next;
            if !page.more_coming {
                complete = true;
                break;
            }
            // "More coming" with no new position — none, or the one just sent — would ask
            // the same thing again, up to the page limit (coddingtonbear/icloud-md#48): it
            // stops there, unfinished, and the next pass asks again from where it is.
            if !advanced {
                break;
            }
        }
        Ok(ZoneWalk {
            records,
            from_start: since.is_none(),
            token,
            complete,
        })
    }
}

/// Where a walk carries on from after a page: the page's position, or — given none, or an empty
/// one — the one it was asked from, so the last page leaving it out never loses the place
/// (and the next pass never reads the whole zone again for it). Answers whether it moved.
pub(crate) fn advance(asked: Option<String>, given: Option<String>) -> (Option<String>, bool) {
    match given.filter(|t| !t.is_empty()) {
        Some(t) if asked.as_ref() != Some(&t) => (Some(t), true),
        Some(t) => (Some(t), false),
        None => (asked, false),
    }
}

/// What one walk of a zone returned.
///
/// `complete` is the part that matters. A walk stops at a page cap, and one that stopped
/// early has seen only part of the zone — treating that as the whole truth and pruning
/// everything it did not mention would delete records that are perfectly fine. Nothing
/// authoritative may be done with a walk that did not finish.
pub struct ZoneWalk {
    pub records: Vec<Value>,
    pub token: Option<String>,
    pub complete: bool,
    /// Read from the zone's beginning — no cursor asked for, or the cursor refused and the
    /// walk started over. A walk from a cursor carries only changes, where absence means
    /// unchanged; one started over after a refused cursor carries everything and is as
    /// authoritative as a first pass (`ZoneWalk::whole`).
    pub from_start: bool,
}

impl ZoneWalk {
    /// Whether this walk read the zone from its beginning — asked for no cursor, or its cursor
    /// refused and the walk started over — and to its end. Only such a walk is the whole truth
    /// that rows missing from it may be pruned by; any other carries changes, or part of the
    /// zone, where absence means nothing.
    #[must_use]
    pub fn whole(&self) -> bool {
        self.from_start && self.complete
    }
}

/// Field helpers. CloudKit wraps every value as `{"type": …, "value": …}`.
pub mod field {
    use serde_json::Value;

    #[must_use]
    pub fn string(record: &Value, name: &str) -> Option<String> {
        record
            .pointer(&format!("/fields/{name}/value"))?
            .as_str()
            .map(ToString::to_string)
    }

    #[must_use]
    pub fn int(record: &Value, name: &str) -> Option<i64> {
        record.pointer(&format!("/fields/{name}/value"))?.as_i64()
    }

    #[must_use]
    pub fn flag(record: &Value, name: &str) -> bool {
        int(record, name).unwrap_or(0) != 0
    }

    /// CloudKit timestamps are milliseconds; the library stores seconds.
    #[must_use]
    pub fn timestamp_seconds(record: &Value, name: &str) -> Option<i64> {
        int(record, name).map(|ms| ms / 1000)
    }

    /// The target of a REFERENCE field, e.g. `List/A233F124-…`.
    #[must_use]
    pub fn reference(record: &Value, name: &str) -> Option<String> {
        record
            .pointer(&format!("/fields/{name}/value/recordName"))?
            .as_str()
            .map(ToString::to_string)
    }

    /// Text out of a document field such as `TitleDocument`.
    #[must_use]
    pub fn document_text(record: &Value, name: &str) -> Option<String> {
        crate::ckdocument::text_from_base64(&string(record, name)?)
    }

    #[must_use]
    pub fn record_name(record: &Value) -> Option<String> {
        record.get("recordName")?.as_str().map(ToString::to_string)
    }

    #[must_use]
    pub fn record_type(record: &Value) -> Option<String> {
        record.get("recordType")?.as_str().map(ToString::to_string)
    }
}

/// Seconds between the Unix epoch and Apple's, the first moment of 2001 UTC.
const APPLE_EPOCH: f64 = 978_307_200.0;

/// The token key Apple uses for a field: its name with the first letter lowered, except where
/// Apple named it something else.
fn token_key(field: &str) -> String {
    match field {
        "ReminderIDs" => "reminderIDsMergeableOrdering".into(),
        _ => {
            let mut chars = field.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_lowercase().chain(chars).collect()
            })
        }
    }
}

/// Add a `ResolutionTokenMap` to `fields` advancing the token of every field in it.
fn stamp_tokens(fields: &mut Value, current: Option<&Value>) {
    let Some(written) = fields.as_object() else {
        return;
    };
    let mut map: Value = current
        .and_then(|c| c.pointer("/fields/ResolutionTokenMap/value"))
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .filter(|v: &Value| v.get("map").is_some_and(Value::is_object))
        .unwrap_or_else(|| json!({ "map": {} }));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
        - APPLE_EPOCH;
    let writer = crate::topotext::writer_id();
    let names: Vec<String> = written
        .keys()
        .filter(|k| k.as_str() != "ResolutionTokenMap")
        .cloned()
        .collect();
    for name in names {
        let key = token_key(&name);
        let counter = map["map"][&key]["counter"].as_u64().unwrap_or(0) + 1;
        map["map"][key] =
            json!({ "counter": counter, "modificationTime": now, "replicaID": writer });
    }
    fields["ResolutionTokenMap"] = json!({ "type": "STRING", "value": map.to_string() });
}

#[cfg(test)]
mod token_tests {
    use super::*;

    fn tokens(fields: &Value) -> Value {
        serde_json::from_str(fields["ResolutionTokenMap"]["value"].as_str().unwrap()).unwrap()
    }

    /// The shape measured on the phone's own records.
    #[test]
    fn each_field_written_gets_its_token_advanced() {
        let current = json!({ "fields": { "ResolutionTokenMap": { "type": "STRING", "value":
            r#"{"map":{"titleDocument":{"counter":1,"modificationTime":803701265.4,"replicaID":"A"},
                      "priority":{"counter":4,"modificationTime":803701265.4,"replicaID":"B"},
                      "reminderIDsMergeableOrdering":{"counter":123,"modificationTime":1.0,"replicaID":"C"}}}"# } } });
        let mut fields = json!({
            "TitleDocument": { "type": "ENCRYPTED_BYTES", "value": "x" },
            "ReminderIDs": { "type": "STRING", "value": "[]" },
            "LastModifiedDate": { "type": "TIMESTAMP", "value": 1 },
        });
        stamp_tokens(&mut fields, Some(&current));
        let map = &tokens(&fields)["map"];
        assert_eq!(map["titleDocument"]["counter"], 2);
        assert_eq!(map["reminderIDsMergeableOrdering"]["counter"], 124);
        assert_eq!(map["lastModifiedDate"]["counter"], 1);
        // What was not written is left exactly as it was.
        assert_eq!(map["priority"]["counter"], 4);
        assert_eq!(map["priority"]["replicaID"], "B");
        // Now, in Apple's seconds, by this installation.
        let t = map["titleDocument"]["modificationTime"].as_f64().unwrap();
        assert!(t > 812_000_000.0, "{t}");
        assert_eq!(
            map["titleDocument"]["replicaID"],
            crate::topotext::writer_id()
        );
    }

    #[test]
    fn a_new_record_starts_every_token_at_one() {
        let mut fields = json!({ "Name": { "type": "STRING", "value": "GYM" } });
        stamp_tokens(&mut fields, None);
        assert_eq!(tokens(&fields)["map"]["name"]["counter"], 1);
    }

    #[test]
    fn a_broken_map_is_started_again_rather_than_kept() {
        let current = json!({ "fields": { "ResolutionTokenMap": { "type": "STRING", "value": "not json" } } });
        let mut fields = json!({ "Flagged": { "type": "NUMBER_INT64", "value": 1 } });
        stamp_tokens(&mut fields, Some(&current));
        assert_eq!(tokens(&fields)["map"]["flagged"]["counter"], 1);
    }
}

#[cfg(test)]
mod tests {

    /// A zone another account owns is asked for in the shared database, by its name and its
    /// owner together; the account's own zones, in the private one, by name alone.
    #[test]
    fn a_shared_zone_is_named_with_its_owner_in_the_shared_database() {
        let http = reqwest::Client::new();
        let own = ZoneClient::new(&http, "https://ck.example/", "c", vec![]);
        assert_eq!(
            own.url("zones/list"),
            "https://ck.example/database/1/c/production/private/zones/list"
        );
        assert!(own.zone_id("Z").get("ownerRecordName").is_none());
        let theirs = ZoneClient::new(&http, "https://ck.example/", "c", vec![])
            .in_database(Database::Shared)
            .owned_by(Some("_owner".into()));
        assert_eq!(
            theirs.url("records/lookup"),
            "https://ck.example/database/1/c/production/shared/records/lookup"
        );
        assert_eq!(theirs.zone_id("Z")["ownerRecordName"], "_owner");
        assert_ne!(
            own.scope(),
            theirs.scope(),
            "what is learnt of one is never taken for the other"
        );
    }
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn record(name: &str, kind: &str) -> Value {
        json!({
            "recordName": name,
            "recordType": kind,
            "fields": { "Completed": { "type": "NUMBER_INT64", "value": 0 } }
        })
    }

    fn overloaded() -> ResponseTemplate {
        // What the account sent, verbatim but for the uuid.
        ResponseTemplate::new(503).set_body_json(json!({
            "uuid": "1", "retryAfter": 1, "serverErrorCode": "SERVER_OVERLOADED"
        }))
    }

    /// Many records are looked up 200 to a request, each found one kept by its name, and a
    /// name CloudKit has no record for (a tombstone without a change tag) left out.
    #[tokio::test]
    async fn records_are_looked_up_two_hundred_at_a_time() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(|req: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&req.body).unwrap();
                let asked = body["records"].as_array().unwrap();
                assert!(asked.len() <= 200, "{} in one lookup", asked.len());
                let records: Vec<Value> = asked
                    .iter()
                    .map(|r| {
                        let name = r["recordName"].as_str().unwrap();
                        if name == "gone" {
                            json!({ "recordName": name, "serverErrorCode": "NOT_FOUND" })
                        } else {
                            json!({ "recordName": name, "recordChangeTag": "t" })
                        }
                    })
                    .collect();
                ResponseTemplate::new(200).set_body_json(json!({ "records": records }))
            })
            .expect(3)
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.photos.cloud", vec![]);
        let mut names: Vec<String> = (0..449).map(|i| format!("A{i}")).collect();
        names.push("gone".into());
        let found = client.lookup_records("PrimarySync", &names).await.unwrap();
        assert_eq!(found.len(), 449);
        assert!(found.contains_key("A448") && !found.contains_key("gone"));
    }

    /// A busy CloudKit asks to be asked again shortly; asked again, the change goes through
    /// instead of being lost with raw JSON on the screen.
    #[tokio::test]
    async fn an_overloaded_cloudkit_is_waited_out() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(overloaded())
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "R", "recordChangeTag": "t2" }]
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        client
            .update_record("Reminders", "R", "Reminder", Some("t1"), json!({}))
            .await
            .unwrap();
    }

    /// A zone busy with another device's write answers 409 ZONE_BUSY and asks to be asked
    /// again — measured on the account — and asked again, the write goes through.
    #[tokio::test]
    async fn a_busy_zone_is_waited_out() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "uuid": "1", "serverErrorCode": "ZONE_BUSY",
                "reason": "Sync zone CAS Op-Lock failed. There was a concurrent write and this operation was rejected. Retry"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "R", "recordChangeTag": "t2" }]
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        client
            .update_record("Reminders", "R", "Reminder", Some("t1"), json!({}))
            .await
            .unwrap();
    }

    /// Two of this app's own writes close together: Apple's answer, word for word as he met it
    /// deleting notes in Recently Deleted, is waited out the same way.
    #[tokio::test]
    async fn a_concurrent_write_is_tried_again() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({
                "uuid": "d1fdc23f-b3c5-4537-8ab2-75e2ec1606f9", "serverErrorCode": "TRY_AGAIN_LATER",
                "reason": "Sync zone CAS Op-Lock failed. There was a concurrent write and this operation was rejected."
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "N", "recordChangeTag": "t2" }]
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.notes", vec![]);
        client
            .update_record("Notes", "N", "Note", Some("t1"), json!({}))
            .await
            .unwrap();
    }

    /// The same refusal carried per record in a 200, as Apple also sends it: tried again too.
    #[tokio::test]
    async fn a_record_refused_for_a_busy_zone_is_tried_again() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "N", "serverErrorCode": "TRY_AGAIN_LATER",
                  "reason": "Sync zone CAS Op-Lock failed. There was a concurrent write and this operation was rejected. Retry request..." }]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "N", "recordChangeTag": "t2" }]
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.notes", vec![]);
        client
            .update_record("Notes", "N", "Note", Some("t1"), json!({}))
            .await
            .unwrap();
    }

    /// And one still busy after that is said in a sentence, not Apple's JSON.
    #[tokio::test]
    async fn a_cloudkit_that_stays_busy_says_so_plainly() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(overloaded())
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let message = client
            .update_record("Reminders", "R", "Reminder", Some("t1"), json!({}))
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            message,
            "iCloud is busy right now. Nothing was changed — try again in a minute."
        );
    }

    /// Ignoring moreComing silently truncates someone's data, so paging is the thing to test.
    #[tokio::test]
    async fn paging_follows_more_coming_to_the_end() {
        let server = MockServer::start().await;
        let pages = vec![
            json!({ "zones": [{ "records": [record("a", "Reminder")],
                                "syncToken": "t1", "moreComing": true }] }),
            json!({ "zones": [{ "records": [record("b", "Reminder")],
                                "syncToken": "t2", "moreComing": true }] }),
            json!({ "zones": [{ "records": [record("c", "Reminder")],
                                "syncToken": "t3", "moreComing": false }] }),
        ];
        for page in pages {
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone$"))
                .respond_with(ResponseTemplate::new(200).set_body_json(page))
                .up_to_n_times(1)
                .mount(&server)
                .await;
        }

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let ZoneWalk { records, token, .. } = client.all_records("Reminders", None).await.unwrap();

        assert_eq!(records.len(), 3, "every page must be collected");
        assert_eq!(
            token.as_deref(),
            Some("t3"),
            "the final cursor is what resumes later"
        );
    }

    /// A server that says more-coming but hands back the position it was asked from would be
    /// asked the same thing two hundred times (coddingtonbear/icloud-md#48): the walk stops,
    /// unfinished, at the position it reached.
    #[tokio::test]
    async fn paging_stops_when_the_cursor_does_not_move() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{ "records": [record("a", "Reminder")], "syncToken": "STUCK", "moreComing": true }]
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let walk = client.all_records("Reminders", None).await.unwrap();
        assert!(!walk.complete, "not the whole zone");
        assert_eq!(walk.token.as_deref(), Some("STUCK"));
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "asked once more, then stopped"
        );
    }

    /// A last page that leaves its position out, or leaves it empty, keeps the one the walk
    /// was at: dropped, the next pass read the whole zone again.
    #[test]
    fn a_page_without_a_position_keeps_the_one_before() {
        assert_eq!(advance(Some("A".into()), None), (Some("A".into()), false));
        assert_eq!(
            advance(Some("A".into()), Some(String::new())),
            (Some("A".into()), false)
        );
        assert_eq!(
            advance(Some("A".into()), Some("A".into())),
            (Some("A".into()), false)
        );
        assert_eq!(
            advance(Some("A".into()), Some("B".into())),
            (Some("B".into()), true)
        );
        assert_eq!(advance(None, Some("B".into())), (Some("B".into()), true));
        assert_eq!(advance(None, None), (None, false));
    }

    /// A server that always says more-coming without a cursor must not spin.
    #[tokio::test]
    async fn paging_stops_when_there_is_no_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{ "records": [record("a", "Reminder")], "moreComing": true }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let ZoneWalk { records, .. } = client.all_records("Reminders", None).await.unwrap();
        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn a_refused_zone_is_an_error_not_an_empty_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{ "reason": "Zone does not exist" }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        assert!(client.all_records("Missing", None).await.is_err());
    }

    #[test]
    fn field_helpers_read_cloudkits_wrapping() {
        let rec = json!({
            "recordName": "Reminder/abc",
            "recordType": "Reminder",
            "fields": {
                "Completed": { "type": "NUMBER_INT64", "value": 1 },
                "CreationDate": { "type": "TIMESTAMP", "value": 1_753_808_338_928_i64 },
                "List": { "type": "REFERENCE", "value": { "recordName": "List/xyz" } }
            }
        });
        assert_eq!(field::record_name(&rec).as_deref(), Some("Reminder/abc"));
        assert_eq!(field::record_type(&rec).as_deref(), Some("Reminder"));
        assert!(field::flag(&rec, "Completed"));
        assert!(
            !field::flag(&rec, "Missing"),
            "an absent flag reads as false"
        );
        assert_eq!(field::reference(&rec, "List").as_deref(), Some("List/xyz"));
        assert_eq!(
            field::timestamp_seconds(&rec, "CreationDate"),
            Some(1_753_808_338),
            "milliseconds must become seconds"
        );
    }
}

impl ZoneClient<'_> {
    /// Update one record in the zone.
    ///
    /// `changes` are merged into the record's existing fields, so only what actually changed
    /// is sent. The change tag is CloudKit's optimistic-concurrency check: if the record
    /// moved since we read it, Apple refuses rather than silently overwriting whatever the
    /// phone did.
    /// [`Self::update_record`] with no retry: a conflict comes back as [`Error::Conflict`].
    ///
    /// For writes built from the record as it stood — a note's document, with its merge
    /// records — where resending the same fields under a newer tag would lay a stale document
    /// over whatever the other device just wrote.
    pub async fn update_record_exact(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        change_tag: &str,
        changes: Value,
    ) -> Result<Value> {
        self.modify(
            zone,
            json!({ "operationType": "update", "record": {
                "recordName": record_name,
                "recordType": record_type,
                "recordChangeTag": change_tag,
                "fields": changes,
            } }),
        )
        .await
    }

    pub async fn update_record(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        change_tag: Option<&str>,
        changes: Value,
    ) -> Result<Value> {
        let mut record = json!({
            "recordName": record_name,
            "recordType": record_type,
            "fields": changes,
        });

        // CloudKit rejects an update with no change tag outright — not as a conflict, as a
        // malformed request. The local copy legitimately may not have one yet: a record
        // created a moment ago has a name before it has a tag. Fetching the current tag is
        // what makes a write on a just-created record work instead of failing at 400.
        let tag = match change_tag {
            Some(tag) => Some(tag.to_string()),
            None => self
                .lookup_record(zone, record_name)
                .await?
                .and_then(|fresh| {
                    fresh
                        .get("recordChangeTag")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                }),
        };
        if let Some(tag) = tag {
            record["recordChangeTag"] = Value::String(tag);
        }

        match self
            .modify(
                zone,
                json!({ "operationType": "update", "record": record.clone() }),
            )
            .await
        {
            Err(Error::Conflict) => {
                // The record moved since we read it — usually the phone changed it, sometimes
                // our own previous write advanced the tag. Read the current tag and try once
                // more, rather than handing the user an error we can resolve ourselves.
                let Some(fresh) = self.lookup_record(zone, record_name).await? else {
                    return Err(Error::Protocol(
                        "that record no longer exists in iCloud".into(),
                    ));
                };
                let Some(tag) = fresh.get("recordChangeTag").and_then(Value::as_str) else {
                    return Err(Error::Conflict);
                };
                let mut retried = record;
                retried["recordChangeTag"] = Value::String(tag.to_string());
                self.modify(
                    zone,
                    json!({ "operationType": "update", "record": retried }),
                )
                .await
            }
            other => other,
        }
    }

    /// Send one operation to `records/modify` and surface what came back.
    /// A `records/query` in the zone: `query` is CloudKit's `{recordType, filterBy}`. For the
    /// server-side operations Apple runs as queries — completing a repeating reminder is one —
    /// as much as for reading. Answers with the records it returns.
    pub async fn records_query(&self, zone: &str, query: Value) -> Result<Vec<Value>> {
        let body = json!({
            "query": query,
            "zoneID": self.zone_id(zone),
        });
        let url = self.url("records/query");
        let (status, text) = self.post_json(&url, &body).await?;
        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit refused the query ({status}): {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("CloudKit returned invalid JSON: {e}")))?;
        Ok(parsed
            .get("records")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    async fn modify(&self, zone: &str, operation: Value) -> Result<Value> {
        // A record refused because the zone was busy with another write was not applied: the
        // same request goes again after a moment, as a busy server's is.
        let mut tries = 0;
        loop {
            tries += 1;
            match self.modify_once(zone, &operation).await {
                Err(Error::Busy) if tries < BUSY_TRIES => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        400 * u64::from(tries) * u64::from(tries),
                    ))
                    .await;
                }
                Err(Error::Busy) => return Err(Error::Busy),
                other => return other,
            }
        }
    }

    async fn modify_once(&self, zone: &str, operation: &Value) -> Result<Value> {
        let body = json!({
            "operations": [operation],
            "zoneID": self.zone_id(zone),
        });

        let url = self.url("records/modify");
        let (status, text) = self.post_json(&url, &body).await?;

        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit refused the change ({status}): {}",
                text.chars().take(200).collect::<String>()
            )));
        }

        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("CloudKit returned invalid JSON: {e}")))?;

        // A 200 can still carry a per-record failure, which is how a conflict arrives.
        if let Some(reason) = parsed.pointer("/records/0/reason").and_then(Value::as_str) {
            let code = parsed
                .pointer("/records/0/serverErrorCode")
                .and_then(Value::as_str)
                .unwrap_or("");
            if zone_was_busy(code, reason) {
                return Err(Error::Busy);
            }
            // Apple signals a stale change tag as CONFLICT, and sometimes as an "oplock"
            // message with a different code; both mean the same thing.
            let stale = code == "CONFLICT" || reason.to_ascii_lowercase().contains("oplock");
            return Err(if stale {
                Error::Conflict
            } else {
                Error::Protocol(format!("CloudKit refused the change: {reason}"))
            });
        }
        Ok(parsed)
    }
}

impl ZoneClient<'_> {
    /// Create a record in the zone.
    ///
    /// The caller supplies the record name so the same identifier can be stored locally
    /// without waiting for a round trip.
    /// [`Self::update_record`], with each changed field's resolution token advanced.
    ///
    /// Reminders and lists carry `ResolutionTokenMap`, a JSON map from each field to
    /// `{counter, modificationTime, replicaID}`, and a phone merges field by field on those
    /// tokens. A write that leaves them alone gives a phone no reason to prefer it — measured on
    /// the account, a reminder created here had every token at counter 0, and the phone's own
    /// edits carry counters of 1 and up. So each field written gets its counter raised, the
    /// time set to now, and this installation as the writer. Only for record types that carry
    /// the map: CloudKit refuses a field its schema does not already have.
    pub async fn update_record_signed(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        change_tag: Option<&str>,
        changes: Value,
    ) -> Result<Value> {
        // The token map is one field written whole, worked out from the record as read. The
        // plain retry resent it after a conflict: every counter a phone had raised in between
        // went back to what it was before, and the phone's next merge could prefer an older
        // value of a field this write never touched. So each try is stamped from a fresh read,
        // and written only over that version.
        let mut tag = change_tag.map(ToString::to_string);
        for _ in 0..5 {
            let current = self.lookup_record(zone, record_name).await?;
            let mut stamped = changes.clone();
            stamp_tokens(&mut stamped, current.as_ref());
            let Some(fresh) = current
                .as_ref()
                .and_then(|c| c.get("recordChangeTag"))
                .and_then(Value::as_str)
            else {
                // Nothing there to stamp against: Apple's own answer to the write says why.
                return self
                    .update_record(zone, record_name, record_type, tag.as_deref(), stamped)
                    .await;
            };
            // The caller's tag first, as before: a change made elsewhere since it was read is
            // still laid under this one, field by field, on the fresh read below.
            let first = tag.take().unwrap_or_else(|| fresh.to_string());
            match self
                .update_record_exact(zone, record_name, record_type, &first, stamped)
                .await
            {
                Err(Error::Conflict) => continue,
                other => return other,
            }
        }
        Err(Error::Conflict)
    }

    /// [`Self::update_record_signed`] with no retry: a conflict comes back as
    /// [`Error::Conflict`], for a caller whose fields were built from the record as it stood
    /// (a reminder's title document) and must be built again, not resent.
    pub async fn update_record_signed_exact(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        change_tag: &str,
        mut changes: Value,
    ) -> Result<Value> {
        let current = self.lookup_record(zone, record_name).await?;
        stamp_tokens(&mut changes, current.as_ref());
        self.update_record_exact(zone, record_name, record_type, change_tag, changes)
            .await
    }

    /// Change a record by working the change out from the record as Apple holds it, written
    /// only over that exact version — and, if another device wrote first, read again and
    /// worked out again. For fields written whole from what was read, such as a list's order:
    /// the plain retry resent the array built from the older read, and a reminder the phone
    /// had just added dropped out of the order. `change` answers `None` when nothing needs
    /// writing. Signed as [`Self::update_record_signed`] signs.
    pub async fn edit_record_signed(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        change: impl Fn(&Value) -> Option<Value>,
    ) -> Result<Option<Value>> {
        self.edit_record_inner(zone, record_name, record_type, change, true)
            .await
    }

    /// [`Self::edit_record_signed`] for records Apple does not sign — notes.
    pub async fn edit_record(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        change: impl Fn(&Value) -> Option<Value>,
    ) -> Result<Option<Value>> {
        self.edit_record_inner(zone, record_name, record_type, change, false)
            .await
    }

    async fn edit_record_inner(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        change: impl Fn(&Value) -> Option<Value>,
        signed: bool,
    ) -> Result<Option<Value>> {
        for _ in 0..5 {
            let Some(current) = self.lookup_record(zone, record_name).await? else {
                return Err(Error::Refused("That is no longer in iCloud.".into()));
            };
            let Some(mut changes) = change(&current) else {
                return Ok(None);
            };
            let Some(tag) = current.get("recordChangeTag").and_then(Value::as_str) else {
                return Err(Error::Conflict);
            };
            if signed {
                stamp_tokens(&mut changes, Some(&current));
            }
            match self
                .update_record_exact(zone, record_name, record_type, tag, changes)
                .await
            {
                Ok(v) => return Ok(Some(v)),
                Err(Error::Conflict) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(Error::Conflict)
    }

    /// [`Self::create_record`], with a first token for every field, as the phone makes one.
    pub async fn create_record_signed(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        mut fields: Value,
    ) -> Result<Value> {
        stamp_tokens(&mut fields, None);
        self.create_record(zone, record_name, record_type, fields)
            .await
    }

    pub async fn create_record(
        &self,
        zone: &str,
        record_name: &str,
        record_type: &str,
        fields: Value,
    ) -> Result<Value> {
        let made = self
            .modify(
                zone,
                json!({
                    "operationType": "create",
                    "record": {
                        "recordName": record_name,
                        "recordType": record_type,
                        "fields": fields,
                    },
                }),
            )
            .await;
        match made {
            // The connection dropped with the request out: Apple may have made it and the
            // answer been lost. Said to have failed, it was made again under a new name — a
            // second reminder, a second note. The name is ours, so Apple is asked.
            Err(Error::Network(e)) => match self.lookup_record(zone, record_name).await {
                Ok(Some(record)) => Ok(json!({ "records": [record] })),
                _ => Err(Error::Network(e)),
            },
            other => other,
        }
    }

    /// Create a record CloudKit names itself, as icloud.com creates a Photos album: no
    /// `recordName` in the body. Answers the record made, its name with it.
    pub async fn create_record_unnamed(
        &self,
        zone: &str,
        record_type: &str,
        fields: Value,
    ) -> Result<Value> {
        let answer = self
            .modify(
                zone,
                json!({
                    "operationType": "create",
                    "record": { "recordType": record_type, "fields": fields },
                }),
            )
            .await?;
        answer.pointer("/records/0").cloned().ok_or_else(|| {
            Error::Protocol("CloudKit made the record but did not say what it is".into())
        })
    }

    /// Remove a record outright.
    ///
    /// `forceDelete` is CloudKit's delete-regardless-of-change-tag. The alternative is to
    /// pass the tag and have Apple refuse if it moved, which is right when a person is
    /// editing and wrong when they have said to delete the thing.
    pub async fn delete_record(&self, zone: &str, record_name: &str) -> Result<()> {
        self.modify(
            zone,
            json!({
                "operationType": "forceDelete",
                "record": { "recordName": record_name },
            }),
        )
        .await?;
        Ok(())
    }

    /// Many records updated at once, as icloud.com sends a selection: up to 200 operations to a
    /// `records/modify`, not atomic — each record stands or falls alone — and a record whose
    /// change tag had moved (the phone changed it) tried once more on its fresh tag. Answers
    /// each update's outcome in the order given: the record as it now is, or why not.
    pub async fn update_records(
        &self,
        zone: &str,
        record_type: &str,
        updates: &[(String, Option<String>, Value)],
    ) -> Result<Vec<std::result::Result<Value, String>>> {
        let mut out: Vec<std::result::Result<Value, String>> = Vec::with_capacity(updates.len());
        for chunk in updates.chunks(200) {
            let first = self.modify_many(zone, record_type, chunk).await?;
            // The stale ones, once more on the tag Apple has now.
            let stale: Vec<usize> = first
                .iter()
                .enumerate()
                .filter(|(_, r)| matches!(r, Err(Error::Conflict)))
                .map(|(i, _)| i)
                .collect();
            let mut answers: Vec<std::result::Result<Value, String>> = first
                .into_iter()
                .map(|r| r.map_err(|e| e.to_string()))
                .collect();
            if !stale.is_empty() {
                let names: Vec<&str> = stale.iter().map(|&i| chunk[i].0.as_str()).collect();
                let fresh = self.lookup_tags(zone, &names).await?;
                let again: Vec<(String, Option<String>, Value)> = stale
                    .iter()
                    .map(|&i| {
                        let (name, _, fields) = &chunk[i];
                        (name.clone(), fresh.get(name).cloned(), fields.clone())
                    })
                    .collect();
                let second = self.modify_many(zone, record_type, &again).await?;
                for (&i, r) in stale.iter().zip(second) {
                    answers[i] = r.map_err(|e| e.to_string());
                }
            }
            out.extend(answers);
        }
        Ok(out)
    }

    /// One `records/modify` of many updates, not atomic; each record's own outcome.
    async fn modify_many(
        &self,
        zone: &str,
        record_type: &str,
        updates: &[(String, Option<String>, Value)],
    ) -> Result<Vec<Result<Value>>> {
        let operations: Vec<Value> = updates
            .iter()
            .map(|(name, tag, fields)| {
                let mut record =
                    json!({ "recordName": name, "recordType": record_type, "fields": fields });
                if let Some(tag) = tag {
                    record["recordChangeTag"] = Value::String(tag.clone());
                }
                json!({ "operationType": "update", "record": record })
            })
            .collect();
        let body = json!({
            "operations": operations,
            "zoneID": self.zone_id(zone),
            "atomic": false,
        });
        let url = self.url("records/modify");
        let (status, text) = self.post_json(&url, &body).await?;
        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit refused the change ({status}): {}",
                text.chars().take(200).collect::<String>()
            )));
        }
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("CloudKit returned invalid JSON: {e}")))?;
        let records = parsed
            .get("records")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Matched by name, not place: an answer missing for one is that one's failure.
        Ok(updates
            .iter()
            .map(|(name, _, _)| {
                let Some(r) = records
                    .iter()
                    .find(|r| r.get("recordName").and_then(Value::as_str) == Some(name.as_str()))
                else {
                    return Err(Error::Protocol("CloudKit did not answer for it".into()));
                };
                match r.get("reason").and_then(Value::as_str) {
                    None => Ok(r.clone()),
                    Some(reason) => {
                        let code = r
                            .get("serverErrorCode")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        // A busy zone is retried as a moved tag is, once more on a fresh read.
                        Err(
                            if code == "CONFLICT"
                                || reason.to_ascii_lowercase().contains("oplock")
                                || zone_was_busy(code, reason)
                            {
                                Error::Conflict
                            } else {
                                Error::Protocol(format!("CloudKit refused the change: {reason}"))
                            },
                        )
                    }
                }
            })
            .collect())
    }

    /// The current change tags of many records, by name; one missing is not there any more.
    async fn lookup_tags(
        &self,
        zone: &str,
        names: &[&str],
    ) -> Result<std::collections::HashMap<String, String>> {
        let url = self.url("records/lookup");
        let body = json!({
            "records": names.iter().map(|n| json!({ "recordName": n })).collect::<Vec<_>>(),
            "zoneID": self.zone_id(zone),
        });
        let (status, text) = self.post_json(&url, &body).await?;
        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit couldn't look those up ({status})"
            )));
        }
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(parsed
            .get("records")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| {
                Some((
                    r.get("recordName")?.as_str()?.to_string(),
                    r.get("recordChangeTag")?.as_str()?.to_string(),
                ))
            })
            .collect())
    }

    /// The names of the container's private zones Apple could read (`zones/list`). An entry it
    /// could not fetch still names its zone but carries `serverErrorCode`, and is left out.
    pub async fn zone_names(&self) -> Result<Vec<String>> {
        let url = self.url("zones/list");
        let (status, text) = self.post_json(&url, &json!({})).await?;
        if status.as_u16() == 450 || status.as_u16() == 401 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit couldn't list its zones ({status})"
            )));
        }
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(parsed
            .get("zones")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|z| z.get("serverErrorCode").is_none())
            .filter_map(|z| z.pointer("/zoneID/zoneName").and_then(Value::as_str))
            .map(str::to_string)
            .collect())
    }

    /// Every zone in the database Apple could read (`zones/list`), as its name and — in the
    /// shared database, where zones belong to other accounts — its owner.
    pub async fn zone_ids(&self) -> Result<Vec<(String, Option<String>)>> {
        let (status, text) = self.post_json(&self.url("zones/list"), &json!({})).await?;
        if status.as_u16() == 450 || status.as_u16() == 401 {
            return Err(Error::ReauthRequired);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit couldn't list its zones ({status})"
            )));
        }
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let shared = self.database == Database::Shared;
        Ok(parsed
            .get("zones")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|z| {
                z.get("serverErrorCode").is_none() && z.get("deleted") != Some(&json!(true))
            })
            .filter_map(|z| {
                let name = z.pointer("/zoneID/zoneName")?.as_str()?.to_string();
                let owner = z
                    .pointer("/zoneID/ownerRecordName")
                    .and_then(Value::as_str)
                    .filter(|_| shared)
                    .map(str::to_string);
                Some((name, owner))
            })
            .collect())
    }

    /// Who this client reads for: the database, the container and the account (`dsid`), so
    /// something learnt of one account is never taken for another's.
    #[must_use]
    pub fn scope(&self) -> String {
        let dsid = self
            .params
            .iter()
            .find(|(k, _)| k == "dsid")
            .map_or("", |(_, v)| v.as_str());
        format!(
            "{}|{}|{}|{}|{dsid}",
            self.base,
            self.container,
            self.database.path(),
            self.owner.as_deref().unwrap_or("")
        )
    }

    pub async fn lookup_record(&self, zone: &str, record_name: &str) -> Result<Option<Value>> {
        let url = self.url("records/lookup");
        let body = json!({
            "records": [{ "recordName": record_name }],
            "zoneID": self.zone_id(zone),
        });
        let (status, text) = self.post_json(&url, &body).await?;
        if status.as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        // Nothing there is an answer; a server that failed to say is not. A busy or broken
        // lookup read as "no such record" sent callers on to create what already existed, or
        // to skip what was there.
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "CloudKit couldn't look that up ({status})"
            )));
        }
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(parsed
            .get("records")
            .and_then(Value::as_array)
            .and_then(|r| r.first())
            .filter(|r| r.get("recordChangeTag").is_some())
            .cloned())
    }

    /// Many records at once, by name: CloudKit takes up to 200 in one lookup, so a backup of
    /// fifty thousand photos asks a few hundred times rather than fifty thousand. Answers those
    /// found, by name; a name with no record (deleted meanwhile) is simply absent.
    pub async fn lookup_records(
        &self,
        zone: &str,
        record_names: &[String],
    ) -> Result<std::collections::HashMap<String, Value>> {
        let mut found = std::collections::HashMap::new();
        for names in record_names.chunks(LOOKUP_AT_ONCE) {
            let url = self.url("records/lookup");
            let body = json!({
                "records": names.iter().map(|n| json!({ "recordName": n })).collect::<Vec<_>>(),
                "zoneID": self.zone_id(zone),
            });
            let (status, text) = self.post_json(&url, &body).await?;
            if status.as_u16() == 450 {
                return Err(Error::ReauthRequired);
            }
            if !status.is_success() {
                return Err(Error::Protocol(format!(
                    "CloudKit couldn't look those up ({status})"
                )));
            }
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            for record in parsed
                .get("records")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|r| r.get("recordChangeTag").is_some())
            {
                if let Some(name) = record.get("recordName").and_then(Value::as_str) {
                    found.insert(name.to_string(), record.clone());
                }
            }
        }
        Ok(found)
    }

    /// A reference field pointing at another record in the same zone.
    #[must_use]
    pub fn reference_to(&self, zone: &str, record_name: &str) -> Value {
        json!({
            "type": "REFERENCE",
            "value": {
                "action": "VALIDATE",
                "recordName": record_name,
                "zoneID": self.zone_id(zone),
            }
        })
    }
}

#[cfg(test)]
mod modify_tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn server_returning(body: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn an_update_sends_only_what_changed_with_its_change_tag() {
        let server = server_returning(json!({ "records": [{ "recordName": "Reminder/1" }] })).await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);

        client
            .update_record(
                "Reminders",
                "Reminder/1",
                "Reminder",
                Some("tag-7"),
                json!({ "Completed": { "value": 1, "type": "NUMBER_INT64" } }),
            )
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        let sent: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(sent["operations"][0]["operationType"], "update");
        assert_eq!(sent["operations"][0]["record"]["recordChangeTag"], "tag-7");
        assert_eq!(
            sent["operations"][0]["record"]["fields"]["Completed"]["value"],
            1
        );
        assert_eq!(sent["zoneID"]["zoneName"], "Reminders");
    }

    /// A refusal that is not a stale tag must surface Apple's own words, and must not be
    /// retried — retrying a quota or permission error just asks twice for the same refusal.
    #[tokio::test]
    async fn a_refusal_that_is_not_a_conflict_is_reported_verbatim() {
        let server = server_returning(json!({
            "records": [{ "reason": "quota exceeded", "serverErrorCode": "QUOTA_EXCEEDED" }]
        }))
        .await;
        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);

        let err = client
            .update_record(
                "Reminders",
                "Reminder/1",
                "Reminder",
                Some("tag"),
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("quota exceeded"), "got: {err}");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "a non-conflict refusal must not be retried"
        );
    }
}

#[cfg(test)]
mod walk_tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A walk that runs out of pages has seen only part of the zone. Saying so is what
    /// stops a caller treating it as the whole truth and deleting everything it missed.
    #[tokio::test]
    async fn a_walk_that_never_ends_is_reported_incomplete() {
        let server = MockServer::start().await;
        // Every page claims there is more, forever.
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{
                    "records": [{ "recordName": "Reminder/1", "recordType": "Reminder" }],
                    "moreComing": true,
                    "syncToken": "next"
                }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let walk = client.all_records("Reminders", None).await.unwrap();
        assert!(!walk.complete, "hitting the page cap is not completion");
    }

    #[tokio::test]
    async fn a_walk_that_finishes_says_so() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{
                    "records": [{ "recordName": "Reminder/1", "recordType": "Reminder" }],
                    "moreComing": false,
                    "syncToken": "done"
                }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let walk = client.all_records("Reminders", None).await.unwrap();
        assert!(walk.complete);
        assert_eq!(walk.records.len(), 1);
        assert_eq!(walk.token.as_deref(), Some("done"));
    }

    /// "More coming" with no cursor to follow would loop forever; stopping is right, and
    /// calling it complete would not be.
    #[tokio::test]
    async fn more_coming_with_no_cursor_stops_and_admits_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "zones": [{ "records": [], "moreComing": true }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let walk = client.all_records("Reminders", None).await.unwrap();
        assert!(!walk.complete);
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A stale change tag is recoverable: read the current one and try again. Handing the
    /// user "client oplock error" for something we can resolve is not acceptable.
    #[tokio::test]
    async fn a_stale_change_tag_is_retried_with_a_fresh_one() {
        let server = MockServer::start().await;

        // First modify: rejected as stale.
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "reason": "client oplock error updating record",
                              "serverErrorCode": "ATOMIC_ERROR" }]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // The lookup that supplies the current tag.
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "Reminder/1", "recordChangeTag": "fresh-tag" }]
            })))
            .mount(&server)
            .await;

        // Second modify: accepted.
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "Reminder/1", "recordChangeTag": "fresh-tag" }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        client
            .update_record(
                "Reminders",
                "Reminder/1",
                "Reminder",
                Some("stale"),
                json!({}),
            )
            .await
            .expect("a stale tag should be recovered, not surfaced");

        let requests = server.received_requests().await.unwrap();
        let modifies: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path().ends_with("/modify"))
            .collect();
        assert_eq!(modifies.len(), 2, "it should retry exactly once");

        let retried: Value = serde_json::from_slice(&modifies[1].body).unwrap();
        assert_eq!(
            retried["operations"][0]["record"]["recordChangeTag"], "fresh-tag",
            "the retry must carry the tag we just read, not the stale one"
        );
    }

    /// A signed write that lost to a phone's is stamped again from the phone's version: the
    /// retry resent the token map worked out from the first read, putting back every counter
    /// the phone had raised in between — here the phone's title edit, which a field this
    /// write never touched then lost to an older title on the next merge.
    #[tokio::test]
    async fn a_signed_retry_keeps_the_tokens_another_device_raised() {
        let server = MockServer::start().await;
        let map = |title: u64| {
            json!({ "map": { "titleDocument": { "counter": title, "modificationTime": 1.0, "replicaID": "phone" } } })
                .to_string()
        };
        // First read: the title's token at 1. Second read: the phone has raised it to 7.
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "records": [{
                "recordName": "Reminder/1", "recordChangeTag": "t1",
                "fields": { "ResolutionTokenMap": { "type": "STRING", "value": map(1) } } }] })),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "records": [{
                "recordName": "Reminder/1", "recordChangeTag": "t2",
                "fields": { "ResolutionTokenMap": { "type": "STRING", "value": map(7) } } }] })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .and(wiremock::matchers::body_string_contains("\"t1\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "reason": "oplock", "serverErrorCode": "CONFLICT" }] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "Reminder/1", "recordChangeTag": "t3" }] })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        client
            .update_record_signed(
                "Reminders",
                "Reminder/1",
                "Reminder",
                Some("t1"),
                json!({ "Flagged": { "type": "NUMBER_INT64", "value": 1 } }),
            )
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let last = requests
            .iter()
            .rev()
            .find(|r| r.url.path().ends_with("/modify"))
            .unwrap();
        let body: Value = serde_json::from_slice(&last.body).unwrap();
        let record = &body["operations"][0]["record"];
        assert_eq!(record["recordChangeTag"], "t2");
        let tokens: Value = serde_json::from_str(
            record["fields"]["ResolutionTokenMap"]["value"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            tokens["map"]["titleDocument"]["counter"], 7,
            "the phone's raised token is kept, not put back to 1"
        );
        assert_eq!(tokens["map"]["flagged"]["counter"], 1);
    }

    /// A create whose answer was lost on the way back is found at Apple under the name it was
    /// sent with and reported made, not failed — failed, it was made a second time.
    #[tokio::test]
    async fn a_create_whose_answer_was_lost_is_found_not_made_twice() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(2))
                    .set_body_json(
                        json!({ "records": [{ "recordName": "N1", "recordChangeTag": "t" }] }),
                    ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "N1", "recordChangeTag": "t" }] })))
            .mount(&server)
            .await;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(300))
            .build()
            .unwrap();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.notes", vec![]);
        let made = client
            .create_record("Notes", "N1", "Note", json!({}))
            .await
            .expect("made at Apple, so made");
        assert_eq!(made["records"][0]["recordName"], "N1");

        // Not there: the failure stands, for the caller to keep and try again.
        let empty = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(2)))
            .mount(&empty)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "N1", "serverErrorCode": "NOT_FOUND" }] })))
            .mount(&empty)
            .await;
        let client = ZoneClient::new(&http, &empty.uri(), "com.apple.notes", vec![]);
        assert!(matches!(
            client.create_record("Notes", "N1", "Note", json!({})).await,
            Err(Error::Network(_))
        ));
    }

    /// CloudKit rejects an update with no change tag as a malformed request, not as a
    /// conflict — so there is nothing to recover from afterwards. The local copy genuinely
    /// may not have one: a record created a moment ago has a name before it has a tag, and
    /// setting a priority on a reminder you just added hits exactly that.
    #[tokio::test]
    async fn a_write_without_a_local_tag_fetches_one_first() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "Reminder/1", "recordChangeTag": "current" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "Reminder/1" }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        client
            .update_record("Reminders", "Reminder/1", "Reminder", None, json!({}))
            .await
            .expect("a missing tag should be fetched, not fatal");

        let requests = server.received_requests().await.unwrap();
        let modify = requests
            .iter()
            .find(|r| r.url.path().ends_with("/modify"))
            .expect("a modify was sent");
        let body: Value = serde_json::from_slice(&modify.body).unwrap();
        assert_eq!(
            body["operations"][0]["record"]["recordChangeTag"], "current",
            "the write must carry the tag Apple currently holds"
        );
    }

    /// A tag we already hold is used as-is: fetching one on every write would double the
    /// number of requests every edit costs.
    #[tokio::test]
    async fn a_known_tag_is_used_without_a_lookup() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "recordName": "Reminder/1" }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        client
            .update_record(
                "Reminders",
                "Reminder/1",
                "Reminder",
                Some("known"),
                json!({}),
            )
            .await
            .unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "one request, not a lookup and a write");
    }

    /// If the record is gone, say so plainly rather than retrying forever.
    #[tokio::test]
    async fn a_vanished_record_is_reported_not_retried() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/modify$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "records": [{ "reason": "client oplock error", "serverErrorCode": "ATOMIC_ERROR" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/records/lookup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "records": [] })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = ZoneClient::new(&http, &server.uri(), "com.apple.reminders", vec![]);
        let err = client
            .update_record(
                "Reminders",
                "Reminder/1",
                "Reminder",
                Some("stale"),
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no longer exists"), "got: {err}");
    }
}

/// What CloudKit does when something is wrong with a zone or a cursor.
///
/// All of it arrives as HTTP 200 with the trouble inside the zone's own result, so a status
/// check alone says everything went fine. The payloads here are what Apple actually answered
/// when asked, not what the documentation suggests.
#[cfg(test)]
mod trouble_tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn zone_reply(zone: Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({ "zones": [zone] }))
    }

    fn record(name: &str, kind: &str) -> Value {
        json!({ "recordName": name, "recordType": kind, "fields": {} })
    }

    fn client<'a>(http: &'a reqwest::Client, uri: &str, container: &str) -> ZoneClient<'a> {
        ZoneClient::new(http, uri, container, vec![])
    }

    /// A brand new Apple ID that has never opened Notes has no Notes zone. That is an empty
    /// library, not a failure, and it used to surface as a hard error on every refresh.
    #[tokio::test]
    async fn a_zone_the_account_has_never_had_reads_as_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "reason": "Zone does not exist",
                "serverErrorCode": "ZONE_NOT_FOUND"
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let uri = server.uri();
        let client = client(&http, &uri, "com.apple.notes");
        let walk = client.all_records("Notes", None).await.unwrap();
        assert!(walk.records.is_empty());
        assert!(
            walk.complete,
            "an account with no such zone has been read in full, not cut short"
        );
    }

    /// CloudKit can throttle one zone inside a 200 — `THROTTLED`, `TRY_AGAIN_LATER` or
    /// `ZONE_BUSY` with a `retryAfter` (timlaing/pyicloud#384). That is a wait, not a refusal:
    /// waited out, the pass carries on; still busy, it says busy in words.
    #[tokio::test]
    async fn a_zone_throttled_inside_a_200_is_waited_out() {
        for code in ["THROTTLED", "TRY_AGAIN_LATER", "ZONE_BUSY"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone"))
                .respond_with(zone_reply(json!({
                    "serverErrorCode": code, "reason": "slow down", "retryAfter": 1
                })))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone"))
                .respond_with(zone_reply(json!({
                    "records": [record("R1", "Reminder")],
                    "syncToken": "T1", "moreComing": false
                })))
                .mount(&server)
                .await;
            let http = reqwest::Client::new();
            let uri = server.uri();
            let walk = client(&http, &uri, "com.apple.reminders")
                .all_records("Reminders", None)
                .await
                .unwrap_or_else(|e| panic!("{code}: {e}"));
            assert_eq!(walk.records.len(), 1, "{code}");
            assert!(walk.complete, "{code}");
        }

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "serverErrorCode": "THROTTLED", "retryAfter": 1
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let uri = server.uri();
        let err = client(&http, &uri, "com.apple.reminders")
            .all_records("Reminders", None)
            .await
            .err()
            .expect("still throttled");
        assert!(matches!(err, Error::Busy), "{err:?}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            BUSY_TRIES as usize,
            "asked again, a few times, then said"
        );
    }

    /// A session the zone will not take, said inside a 200, is a sign-in — not a broken zone.
    #[tokio::test]
    async fn a_zone_that_wants_a_sign_in_says_so() {
        for code in ["AUTHENTICATION_FAILED", "AUTHENTICATION_REQUIRED"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone"))
                .respond_with(zone_reply(json!({ "serverErrorCode": code })))
                .mount(&server)
                .await;
            let http = reqwest::Client::new();
            let uri = server.uri();
            let err = client(&http, &uri, "com.apple.notes")
                .all_records("Notes", None)
                .await
                .err()
                .expect("refused");
            assert!(matches!(err, Error::ReauthRequired), "{code}: {err:?}");
        }
    }

    /// Issue #4: an account with Advanced Data Protection, before its session is approved on
    /// a device. Said as that — typed, so the approval can be asked for and the zone read again
    /// once it is given — never as "iCloud sent back something unexpected" and Apple's JSON.
    #[tokio::test]
    async fn a_zone_kept_by_advanced_data_protection_asks_for_approval() {
        for (status, body) in [
            // As reported, at the HTTP level.
            (
                403,
                json!({ "serverErrorCode": "ACCESS_DENIED",
                        "reason": "private db access disabled for this account" }),
            ),
            // And inside the zone's own result, the way CloudKit answers most zone trouble.
            (
                200,
                json!({ "zones": [{ "zoneID": { "zoneName": "Reminders" },
                    "serverErrorCode": "ACCESS_DENIED",
                    "reason": "private db access disabled for this account" }] }),
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone"))
                .respond_with(ResponseTemplate::new(status).set_body_json(body))
                .mount(&server)
                .await;
            let http = reqwest::Client::new();
            let uri = server.uri();
            let client = client(&http, &uri, "com.apple.reminders");
            let err = client.all_records("Reminders", None).await.err();
            assert!(
                matches!(err, Some(Error::NeedsDeviceApproval)),
                "{status}: {err:?}"
            );
        }
    }

    /// ACCESS_DENIED for anything else — a retired endpoint says it too — is not read as
    /// Advanced Data Protection: approving on a phone would not help.
    #[tokio::test]
    async fn another_access_denied_is_not_read_as_advanced_data_protection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "serverErrorCode": "ACCESS_DENIED", "reason": "This endpoint is deprecated."
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let uri = server.uri();
        let client = client(&http, &uri, "com.apple.reminders");
        let err = client.all_records("Reminders", None).await.err();
        assert!(matches!(err, Some(Error::Protocol(_))), "{err:?}");
    }

    /// CloudKit expires cursors. An app closed for a while comes back holding one Apple will
    /// not read, and answers BAD_REQUEST with the reason naming the continuation. Starting
    /// over is what it expects; erroring leaves the person stuck with a token that can never
    /// come good.
    #[tokio::test]
    async fn a_cursor_apple_will_not_read_starts_the_walk_over() {
        let server = MockServer::start().await;
        // The stale cursor is refused; a walk with no cursor succeeds.
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .and(wiremock::matchers::body_string_contains("stale-cursor"))
            .respond_with(zone_reply(json!({
                "reason": "Unknown sync continuation type",
                "serverErrorCode": "BAD_REQUEST"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "records": [record("Reminder/1", "Reminder")],
                "syncToken": "fresh",
                "moreComing": false
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let uri = server.uri();
        let client = client(&http, &uri, "com.apple.reminders");
        let walk = match client.all_records("Reminders", Some("stale-cursor")).await {
            Ok(walk) => walk,
            Err(e) => panic!("a refused cursor is recovered from, not raised: {e}"),
        };
        assert_eq!(walk.records.len(), 1);
        assert_eq!(walk.token.as_deref(), Some("fresh"));
        assert!(walk.complete);
        assert!(walk.whole(), "started over, the walk is the whole zone");
    }

    /// An expired change token — with Apple's reason, or as the bare code alone — is a cursor
    /// to start over from, never "nothing changed". The bare code used to match nothing and
    /// read as an empty, finished page, so the dead token was kept and nothing ever arrived.
    #[tokio::test]
    async fn an_expired_change_token_starts_the_walk_over() {
        for expired in [
            json!({ "serverErrorCode": "CHANGE_TOKEN_EXPIRED" }),
            json!({ "serverErrorCode": "CHANGE_TOKEN_EXPIRED", "reason": "change token expired" }),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone"))
                .and(wiremock::matchers::body_string_contains("old-token"))
                .respond_with(zone_reply(expired.clone()))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/changes/zone"))
                .respond_with(zone_reply(json!({
                    "records": [record("Note/1", "Note")],
                    "syncToken": "fresh",
                    "moreComing": false
                })))
                .mount(&server)
                .await;
            let http = reqwest::Client::new();
            let uri = server.uri();
            let client = client(&http, &uri, "com.apple.notes");
            let walk = client
                .all_records("Notes", Some("old-token"))
                .await
                .unwrap_or_else(|e| panic!("{expired}: an expired token is started over: {e}"));
            assert_eq!(walk.records.len(), 1, "{expired}");
            assert_eq!(walk.token.as_deref(), Some("fresh"), "{expired}");
            assert!(walk.whole(), "{expired}: a full read, so it may prune");
        }
    }

    /// A cursor answered with no position at all was not taken, whatever the zone result
    /// said: it is started over, not stored as "nothing changed" with no token.
    #[tokio::test]
    async fn a_cursor_answered_with_no_position_starts_the_walk_over() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .and(wiremock::matchers::body_string_contains("old-token"))
            .respond_with(zone_reply(json!({ "records": [], "moreComing": false })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "records": [record("Note/1", "Note")],
                "syncToken": "fresh",
                "moreComing": false
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let uri = server.uri();
        let client = client(&http, &uri, "com.apple.notes");
        let walk = client
            .all_records("Notes", Some("old-token"))
            .await
            .unwrap();
        assert_eq!(walk.records.len(), 1);
        assert!(walk.whole());
    }

    /// A walk from a cursor Apple took carries only changes: never whole, never pruned by.
    #[tokio::test]
    async fn a_walk_from_a_good_cursor_is_not_whole() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "records": [], "syncToken": "next", "moreComing": false
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let uri = server.uri();
        let client = client(&http, &uri, "com.apple.notes");
        let walk = client.all_records("Notes", Some("good")).await.unwrap();
        assert!(walk.complete && !walk.whole());
    }

    /// Trouble that is not a missing zone or a bad cursor is still worth telling somebody
    /// about — it must not be swallowed as an empty library.
    #[tokio::test]
    async fn other_trouble_is_still_raised() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({
                "reason": "Quota exceeded",
                "serverErrorCode": "QUOTA_EXCEEDED"
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let uri = server.uri();
        let client = client(&http, &uri, "com.apple.reminders");
        let Err(err) = client.all_records("Reminders", None).await else {
            panic!("a real refusal is not an empty library");
        };
        assert!(err.to_string().contains("Quota exceeded"), "got: {err}");
    }

    /// An empty string is not a cursor. Apple answers one with a 500, so it must never be
    /// sent — and absent already means "from the beginning", which is what it was for.
    #[tokio::test]
    async fn an_empty_cursor_is_treated_as_no_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .and(wiremock::matchers::body_string_contains("syncToken"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/changes/zone"))
            .respond_with(zone_reply(json!({ "records": [], "moreComing": false })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let uri = server.uri();
        let client = client(&http, &uri, "com.apple.reminders");
        for empty in ["", "   "] {
            client
                .all_records("Reminders", Some(empty))
                .await
                .unwrap_or_else(|e| panic!("{empty:?} should send no cursor at all: {e}"));
        }
    }
}
