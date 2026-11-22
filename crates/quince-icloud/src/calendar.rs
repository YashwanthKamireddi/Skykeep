//! Apple's calendar web service.
//!
//! `/ca/startup` returns the collections plus the events in a window; `/ca/events` returns
//! events alone. Both are shaped for the icloud.com web client rather than for us.

use crate::{zones, Error, Result};
use jiff::{civil, tz::TimeZone};
use quince_core::calendar::{Calendar, Event};
use serde_json::Value;

/// Apple encodes an instant as `[YYYYMMDD, year, month, day, hour, minute, minsSinceMidnight]`
/// alongside a separate named timezone — not as a timestamp.
///
/// Converting needs the zone, because the same wall-clock reading means different instants in
/// different places. A missing or unknown zone falls back to UTC rather than failing the whole
/// sync over one event.
pub(crate) fn parse_apple_date(raw: &Value, timezone: Option<&str>) -> Result<i64> {
    let parts = raw
        .as_array()
        .ok_or_else(|| Error::Protocol("a date was not an array".into()))?;
    // Index 0 is a packed YYYYMMDD that duplicates the fields after it; the components are
    // what we read. Anything shorter than six entries is unusable.
    if parts.len() < 6 {
        return Err(Error::Protocol(format!(
            "a date had {} components, expected at least 6",
            parts.len()
        )));
    }
    let at = |i: usize| -> Result<i64> {
        parts[i]
            .as_i64()
            .ok_or_else(|| Error::Protocol(format!("date component {i} was not a number")))
    };

    let (year, month, day, hour, minute) = (at(1)?, at(2)?, at(3)?, at(4)?, at(5)?);

    // `civil::date` PANICS on an out-of-range component, and these numbers come off the
    // network. The checked constructor is the only safe way to build one from untrusted input.
    let narrow = |v: i64, what: &str| -> Result<i8> {
        i8::try_from(v).map_err(|_| Error::Protocol(format!("implausible {what} {v}")))
    };
    let civil_dt = civil::DateTime::new(
        i16::try_from(year).map_err(|_| Error::Protocol(format!("implausible year {year}")))?,
        narrow(month, "month")?,
        narrow(day, "day")?,
        narrow(hour, "hour")?,
        narrow(minute, "minute")?,
        0,
        0,
    )
    .map_err(|e| {
        Error::Protocol(format!(
            "{year}-{month}-{day} {hour}:{minute} is not a date: {e}"
        ))
    })?;

    // An unknown zone is not worth failing a sync over; UTC keeps the event visible.
    let zone = zone_named(timezone);

    let zoned = civil_dt
        .to_zoned(zone)
        .map_err(|e| Error::Protocol(format!("couldn't place {civil_dt} in its timezone: {e}")))?;
    Ok(zoned.timestamp().as_second())
}

/// The inverse of [`parse_apple_date`]: Apple's `[YYYYMMDD, y, m, d, h, min, minsSinceMidnight]`.
///
/// Writes must speak the same dialect they read, so this is what a completion timestamp or a
/// due date looks like going back.
pub(crate) fn format_apple_date(seconds: i64, timezone: Option<&str>) -> Value {
    let zone = zone_named(timezone);
    let zoned = jiff::Timestamp::from_second(seconds)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(zone);
    let (y, m, d) = (zoned.year(), zoned.month(), zoned.day());
    let (h, min) = (zoned.hour(), zoned.minute());
    let packed = i64::from(y) * 10_000 + i64::from(m) * 100 + i64::from(d);
    serde_json::json!([packed, y, m, d, h, min, i64::from(h) * 60 + i64::from(min)])
}

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Turn one entry of Apple's `Collection` array into a calendar.
///
/// Not everything in that array is a calendar of events. Apple surfaces reminder lists through
/// the same endpoint and tells them apart with `supportedType`, so anything that does not hold
/// events is left out rather than shown as an empty calendar nobody can put anything in.
fn collection_from(v: &Value) -> Option<Calendar> {
    // Absent means events — only the other kinds say what they are.
    let holds_events = v
        .get("supportedType")
        .and_then(Value::as_str)
        .is_none_or(|kind| kind.eq_ignore_ascii_case("Event"));
    if !holds_events {
        return None;
    }
    Some(Calendar {
        guid: string(v, "guid")?,
        title: string(v, "title").unwrap_or_else(|| "Untitled".into()),
        colour: string(v, "color"),
        ctag: string(v, "ctag"),
        sort_order: v.get("order").and_then(Value::as_i64).unwrap_or(0),
        // Apple omits `enabled` for calendars that are on.
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        // A subscribed feed, or a calendar shared for reading only. Apple refuses a write to
        // one, and that refusal reads as a bug here unless nothing offered it in the first
        // place. Absent means writable, which is the common case.
        writable: !v.get("readOnly").and_then(Value::as_bool).unwrap_or(false),
        // Apple names the default outright. Taking the first of the list instead is the same
        // mistake that put every new reminder in whichever list happened to sort first.
        is_default: v.get("isDefault").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// The zone an event's dates are read and written in.
///
/// A timed event's in its own zone. An all-day event's dates are a day on the calendar, not
/// an instant — Apple sends `[20261005, 2026, 10, 5, 0, 0, 0]` with no zone — and it is that
/// day wherever it is looked at, as on the phone. Read as midnight UTC (measured: the account
/// stores it so), it fell on the day before for anyone west of Greenwich. So all-day dates are
/// this machine's own midnight, which is where the interface draws a day.
///
/// A timed event with no zone floats, as Calendar calls it: 9:00 wherever it is looked at.
/// Read as UTC it was drawn hours off, and an edit wrote that wrong instant back, moving the
/// meeting on every device. So it too is this machine's clock.
///
/// A computer whose zone has no IANA name (TZ set to a POSIX rule, a copied `/etc/localtime`)
/// is still in a zone: its own, named [`SYSTEM_ZONE`] here. Read as UTC instead, every
/// all-day event west of Greenwich fell a day early, for good.
fn dates_zone(all_day: bool, timezone: Option<&str>) -> Option<String> {
    match timezone {
        Some(zone) if !all_day => Some(zone.to_string()),
        _ => Some(
            TimeZone::system()
                .iana_name()
                .unwrap_or(SYSTEM_ZONE)
                .to_string(),
        ),
    }
}

/// This computer's own zone, where it has no name to give.
pub(crate) const SYSTEM_ZONE: &str = "<system>";

/// The zone a name names: [`SYSTEM_ZONE`] this computer's; an IANA name, a Windows one (an
/// event Outlook or Exchange made) or an id ending in an IANA name, that zone; one no zone
/// database knows, UTC.
fn zone_named(name: Option<&str>) -> TimeZone {
    match name {
        Some(SYSTEM_ZONE) => TimeZone::system(),
        Some(name) => zones::named(name).map_or(TimeZone::UTC, |z| z.tz),
        None => TimeZone::UTC,
    }
}

/// An event's zone by its IANA name. Apple passes on the `TZID` the event was made with, and
/// one made in Outlook says "W. Europe Standard Time": the window draws in a zone by its IANA
/// name, and an edit then tells Apple the same zone by that name.
fn iana_named(name: String) -> String {
    match zones::named(&name).and_then(|z| z.name) {
        Some(iana) => iana,
        None => name,
    }
}

/// A zone as Apple is told it (`usertz`): by name, or UTC where it has none.
fn apple_tz(name: Option<&str>) -> &str {
    name.filter(|n| *n != SYSTEM_ZONE).unwrap_or("UTC")
}

/// The zone to speak to Apple in about this event — the day it is asked about, the day its
/// dates are written in. One rule everywhere: an all-day event saved in this machine's zone
/// but looked up again in UTC (where Apple, keeping no zone on it, left it) was looked for on
/// the wrong day, and a move then read as "Apple did not keep it" when Apple had.
pub(crate) fn event_zone(event: &Event) -> Option<String> {
    dates_zone(event.all_day, event.timezone.as_deref())
}

/// Turn one entry of Apple's `Event` array into an event.
///
/// Returns `Ok(None)` for entries that carry no usable identity, so one odd row cannot fail
/// an otherwise good sync.
fn event_from(v: &Value) -> Result<Option<Event>> {
    let (Some(guid), Some(calendar_guid)) = (string(v, "guid"), string(v, "pGuid")) else {
        return Ok(None);
    };
    let timezone = string(v, "tz").map(iana_named);
    let Some(start_raw) = v.get("startDate") else {
        return Ok(None);
    };

    let all_day = v.get("allDay").and_then(Value::as_bool).unwrap_or(false);
    let dates = dates_zone(all_day, timezone.as_deref());
    let starts_at = parse_apple_date(start_raw, dates.as_deref())?;
    let ends_at = match v.get("endDate") {
        Some(raw) => parse_apple_date(raw, dates.as_deref())?,
        // Apple omits endDate on some entries; a duration in minutes is the fallback.
        None => {
            let minutes = v.get("duration").and_then(Value::as_i64).unwrap_or(0);
            starts_at + minutes * 60
        }
    };

    Ok(Some(Event {
        guid,
        calendar_guid,
        title: string(v, "title").unwrap_or_else(|| "Untitled".into()),
        location: string(v, "location"),
        starts_at,
        // A zero-length event still needs to be selectable, and an end before its start is
        // nonsense we should not store.
        ends_at: ends_at.max(starts_at),
        all_day,
        timezone,
        url: string(v, "url"),
        notes: string(v, "description"),
        etag: string(v, "etag"),
    }))
}

/// `YYYY-MM-DD` for the day an instant falls on, in the event's own timezone.
pub(crate) fn day_of(seconds: i64, timezone: Option<&str>) -> String {
    let zone = zone_named(timezone);
    jiff::Timestamp::from_second(seconds)
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
        .to_zoned(zone)
        .strftime("%Y-%m-%d")
        .to_string()
}

/// What the web client tells the service it already has.
///
/// Sending the collection's ctag is how Apple decides whether to answer with the whole
/// calendar or just the change, which is the difference between a quick write and refetching
/// the month on every save.
fn client_state(calendar_guid: &str, ctag: Option<&str>) -> Value {
    serde_json::json!({
        "Collection": [{ "guid": calendar_guid, "ctag": ctag }],
        "fullState": false,
        "userTime": 0,
        "alarmRange": 60,
    })
}

/// Apple's calendar palette, in the order icloud.com offers it (calendar bundle `r2`).
pub const PALETTE: [(&str, &str); 7] = [
    ("#ff2d55", "red"),
    ("#ff9500", "orange"),
    ("#ffcc00", "yellow"),
    ("#65db39", "green"),
    ("#34aadc", "blue"),
    ("#cc73e1", "purple"),
    ("#a2845e", "brown"),
];

/// The first palette colour no calendar uses yet, as icloud.com picks one for a new
/// calendar; the first again when all seven are taken.
#[must_use]
pub fn next_colour(existing: &[Calendar]) -> (&'static str, &'static str) {
    let used: Vec<String> = existing
        .iter()
        .filter_map(|c| c.colour.as_deref())
        .map(|c| c.to_lowercase().chars().take(7).collect())
        .collect();
    PALETTE
        .iter()
        .find(|(hex, _)| !used.iter().any(|u| u == hex))
        .copied()
        .unwrap_or(PALETTE[0])
}

/// A colour's name in Apple's palette, or "custom" for any other.
#[must_use]
pub fn symbolic_of(colour: &str) -> &'static str {
    let c = colour.to_lowercase();
    PALETTE
        .iter()
        .find(|(hex, _)| c.starts_with(hex))
        .map_or("custom", |(_, name)| name)
}

/// The one collection with this guid in a `ca/collectiondetail` (or similar) answer.
fn collection_of(body: &Value, guid: &str) -> Option<Value> {
    let list = body.get("Collection")?;
    let found = match list {
        Value::Array(items) => items
            .iter()
            .find(|c| c.get("guid").and_then(Value::as_str) == Some(guid)),
        Value::Object(_) => {
            (list.get("guid").and_then(Value::as_str) == Some(guid)).then_some(list)
        }
        _ => None,
    };
    found.cloned()
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// What to tell someone when Apple's calendar turns a request down.
///
/// The screen showed what Apple sent — `/ca/events/work/4186… returned 404 Not Found:
/// {"ChangeSet":…}` — which says nothing to a person about what happened or what to do. The
/// sentence says that; the path, status and body go to the log for whoever has to look.
fn refusal(path: &str, status: reqwest::StatusCode, body: &str) -> Error {
    let _ = path;
    crate::told("calendar", status, body);
    quiet!("calendar: a request was refused ({status})");
    match status.as_u16() {
        409 | 412 => Error::Conflict,
        400 | 422 => {
            Error::Refused("iCloud didn't accept that change, so nothing was saved.".into())
        }
        401 | 403 => Error::Refused(
            "iCloud won't let this calendar be changed from here — it may be read-only.".into(),
        ),
        // Not a refusal of the change: Apple can't take it now. A change waiting to go is kept
        // and sent again later (the outbox), never held as turned down.
        429 | 500..=599 => Error::Busy(None),
        code => Error::Refused(format!(
            "iCloud's calendar turned that down (status {code}). Nothing was changed."
        )),
    }
}

/// Everything one calendar refresh yields.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    /// The span, in unix seconds `[from, to)`, Apple was asked about in full. Every event
    /// starting inside it that Apple did not return is gone from iCloud. `None` for a reply
    /// that covers only what was written.
    pub covers: Option<(i64, i64)>,
}

/// Read a `/ca/startup` or `/ca/events` body.
///
/// Malformed individual events are skipped rather than failing the batch: Apple's calendar is
/// long-lived data and one unparseable row from years ago should not block today's sync.
pub fn parse_snapshot(body: &Value) -> Snapshot {
    let calendars = body
        .get("Collection")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(collection_from).collect())
        .unwrap_or_default();

    let events = body
        .get("Event")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| event_from(v).ok().flatten())
                .collect()
        })
        .unwrap_or_default();

    Snapshot {
        calendars,
        events,
        covers: None,
    }
}

/// The event in an `eventdetail` answer.
fn event_of(body: &Value) -> Option<Value> {
    body.pointer("/Event/0")
        .or_else(|| body.pointer("/Event"))
        .filter(|e| e.is_object())
        .cloned()
}

/// An alert as icloud.com writes one — `{guid: "<event>:<uuid>", pGuid: <event>,
/// messageType: "message", isLocationBased: false, measurement: {weeks, days, hours,
/// minutes, seconds, before}}` — `minutes` before the start, or at it for `0`.
fn alarm_for(event_guid: &str, minutes: i64) -> Value {
    let m = minutes.abs();
    let (days, rest) = (m / 1440, m % 1440);
    serde_json::json!({
        "guid": format!("{event_guid}:{}", uuid::Uuid::new_v4().to_string().to_uppercase()),
        "pGuid": event_guid,
        "messageType": "message",
        "isLocationBased": false,
        "measurement": {
            "weeks": 0, "days": days, "hours": rest / 60, "minutes": rest % 60, "seconds": 0,
            "before": minutes > 0,
        },
    })
}

/// Minutes before the start an alert rings, from Apple's `measurement`; `0` at the time.
fn minutes_of(m: &Value) -> Option<i64> {
    let n = |k: &str| m.get(k).and_then(Value::as_i64).unwrap_or(0);
    let total =
        n("weeks") * 10_080 + n("days") * 1440 + n("hours") * 60 + n("minutes") + n("seconds") / 60;
    let before = m.get("before").and_then(Value::as_bool).unwrap_or(true);
    Some(if before { total } else { -total })
}

/// Which events a change to one occurrence of a repeating event is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    /// This one alone; the rest of the series as it was.
    This,
    /// This one and every one after it — the series before it stays as it was. Apple splits
    /// the series at this occurrence; on the first occurrence it is the same as `All`.
    Future,
    /// The whole series: what changed here — and, if this occurrence was moved, the series
    /// moved by as much — every occurrence, past and future.
    All,
}

/// The series an occurrence's id belongs to — `<series>__<start>` — or the id itself.
#[must_use]
pub fn series_of(guid: &str) -> &str {
    guid.split("__").next().unwrap_or(guid)
}

/// Whether an `Alarm` or `Recurrence` object Apple holds belongs to this event. Its `pGuid` is
/// the series' id, not an occurrence's: matching it against an occurrence's id left the repeat
/// out of an "all events" save, and Apple took the series as no longer repeating.
fn belongs_to(object: &Value, event_guid: &str) -> bool {
    object
        .get("pGuid")
        .and_then(Value::as_str)
        .is_some_and(|p| p == event_guid || p == series_of(event_guid))
}

/// Whether `guid` names one occurrence of a repeating event rather than an event: Apple gives
/// each occurrence its series' id and its UTC start, `<series>__20261102T073300Z`.
#[must_use]
pub fn is_occurrence(guid: &str) -> bool {
    guid.contains("__")
}

/// Someone invited to an event, as Apple holds them: their name and address when Apple has
/// them, their answer, and whether they are the person signed in or the one who invited.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Guest {
    pub name: Option<String>,
    pub email: Option<String>,
    /// `accepted`, `declined`, `tentative`, or `pending` for no answer yet — and for any
    /// answer this app does not know, rather than a guess.
    pub status: &'static str,
    pub is_me: bool,
    pub is_organizer: bool,
}

/// The guests of `event_guid` in an `eventdetail` body: the `Invitee` objects the event names
/// in its `invitees`, or that name it (or its series) as their parent. The organizer first,
/// then by name. Read only.
#[must_use]
pub fn guests_from(body: &Value, event_guid: &str) -> Vec<Guest> {
    let named: Vec<&str> = body
        .get("Event")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| {
            e.get("guid")
                .and_then(Value::as_str)
                .is_some_and(|g| g == event_guid || g == series_of(event_guid))
        })
        .flat_map(|e| {
            e.get("invitees")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(Value::as_str)
        .collect();
    let text = |o: &Value, k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let guests: Vec<Guest> = body
        .get("Invitee")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|i| {
            belongs_to(i, event_guid)
                || i.get("guid")
                    .and_then(Value::as_str)
                    .is_some_and(|g| named.contains(&g))
        })
        .map(|i| Guest {
            name: text(i, "commonName"),
            email: text(i, "email").map(|e| {
                e.strip_prefix("mailto:")
                    .or_else(|| e.strip_prefix("MAILTO:"))
                    .unwrap_or(&e)
                    .to_string()
            }),
            status: match i
                .get("inviteeStatus")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str()
            {
                "ACCEPTED" => "accepted",
                "DECLINED" => "declined",
                "TENTATIVE" => "tentative",
                _ => "pending",
            },
            is_me: i.get("isMe").and_then(Value::as_bool).unwrap_or(false),
            is_organizer: i
                .get("isOrganizer")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect();
    // One person once: an address listed twice — the organizer among the invited too, or in
    // another case — is one guest, with what either copy says of them. Left to a dedup after
    // the sort, the copies sorted apart by name and both were shown.
    let mut guests: Vec<Guest> = guests.into_iter().fold(Vec::new(), |mut out, g| {
        let same = |m: &Guest| {
            m.email.is_some()
                && m.email.as_deref().map(str::to_lowercase)
                    == g.email.as_deref().map(str::to_lowercase)
        };
        if let Some(was) = out.iter_mut().find(|m| same(m)) {
            was.is_me |= g.is_me;
            was.is_organizer |= g.is_organizer;
            if was.name.is_none() {
                was.name = g.name;
            }
            if was.status == "pending" {
                was.status = g.status;
            }
        } else {
            out.push(g);
        }
        out
    });
    guests.sort_by(|a, b| {
        b.is_organizer.cmp(&a.is_organizer).then_with(|| {
            let key = |g: &Guest| {
                g.name
                    .clone()
                    .or_else(|| g.email.clone())
                    .unwrap_or_default()
                    .to_lowercase()
            };
            key(a).cmp(&key(b))
        })
    });
    guests
}

/// How an event repeats, in icloud.com's words: `freq` is `daily`, `weekly`, `monthly` or
/// `yearly`; every `interval` of them; ending after `count` times, on the day `until`, or never.
/// A weekly one may fall on several `week_days`; a monthly one on several `month_days`, or on
/// the `position` weekday of the month (the second Tuesday); a yearly one in several `months`,
/// or on the `position` weekday of its month (the fourth Thursday of November). With none of
/// those it falls on its start's weekday, day or date. That is all Apple's repeat holds
/// (docs/icloud-web/calendar.md §5, each form proven against Apple 10 Nov 2026); rules saying
/// more are read by [`crate::rrule`], which refuses what this cannot hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRepeat {
    pub freq: String,
    pub interval: i64,
    #[serde(default)]
    pub count: Option<i64>,
    /// A weekly repeat's weekdays, `MO` … `SU`, its start's among them; with `position`, the
    /// one weekday it names (Apple's `weekDays`). Empty: the start's weekday alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub week_days: Vec<String>,
    /// Which of the month's `week_days` it is: `1st`, `2nd`, `3rd`, `4th` or `last` (Apple's
    /// `frequencyDays`). Monthly, or yearly in its one month.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    /// A monthly repeat's days of the month, 1–31, its start's among them (Apple's `byDay`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub month_days: Vec<i64>,
    /// A yearly repeat's months, 1–12, its start's among them (Apple's `byMonth`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub months: Vec<i64>,
    /// The last day it may fall on, `YYYY-MM-DD` on the event's own clock (Apple's `until`).
    /// Never together with `count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

/// A `Recurrence` object Apple holds, read whole: frequency, interval, end, and its days,
/// position and months.
fn repeat_from_apple(r: &Value) -> Option<EventRepeat> {
    let numbers = |key: &str| -> Vec<i64> {
        r.get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .collect()
    };
    Some(EventRepeat {
        freq: r.get("freq")?.as_str()?.to_string(),
        interval: r.get("interval").and_then(Value::as_i64).unwrap_or(1),
        count: r.get("count").and_then(Value::as_i64),
        week_days: r
            .get("weekDays")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| Some(d.as_str()?.to_ascii_uppercase()))
            .collect(),
        position: r
            .get("frequencyDays")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(str::to_string),
        month_days: numbers("byDay"),
        months: numbers("byMonth"),
        // Apple's dates are `[yyyymmdd, year, month, day, hour, minute, minute of the day]`.
        until: r
            .get("until")
            .and_then(Value::as_array)
            .and_then(|a| {
                Some((
                    a.get(1)?.as_i64()?,
                    a.get(2)?.as_i64()?,
                    a.get(3)?.as_i64()?,
                ))
            })
            .map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}")),
    })
}

/// A repeat written into a new `Recurrence` object as icloud.com writes one. An end day is
/// Apple's own shape for it: the day's last minute, so a time that day still counts.
fn write_repeat(object: &mut Value, rule: &EventRepeat) {
    if let Some(day) = rule
        .until
        .as_deref()
        .and_then(|d| d.parse::<jiff::civil::Date>().ok())
    {
        let (y, m, d) = (
            i64::from(day.year()),
            i64::from(day.month()),
            i64::from(day.day()),
        );
        object["until"] = serde_json::json!([y * 10_000 + m * 100 + d, y, m, d, 23, 59, 1439]);
    } else if let Some(count) = rule.count {
        object["count"] = Value::from(count);
    }
    if !rule.week_days.is_empty() && (rule.freq == "weekly" || rule.position.is_some()) {
        object["weekDays"] = Value::from(rule.week_days.clone());
    }
    if let Some(position) = &rule.position {
        object["frequencyDays"] = Value::from(position.clone());
    }
    if rule.freq == "monthly" && !rule.month_days.is_empty() {
        object["byDay"] = Value::from(rule.month_days.clone());
    }
    if rule.freq == "yearly" && !rule.months.is_empty() {
        object["byMonth"] = Value::from(rule.months.clone());
    }
}

/// An edit made here, laid over the event as another device has since left it — what a save
/// refused as a conflict is saved as instead, the way Calendar settles two edits.
///
/// `base` is the event as this edit began from it, `mine` the edit, `theirs` Apple's copy now.
/// A field changed only there keeps their change; a field changed here is this edit's, even
/// where they changed it too — the edit being saved is the one the person is looking at. When
/// it happens is one field, not four: a start from one side and an end from the other could
/// make an event that ends before it begins. The etag is Apple's latest, which is what lets
/// the write through.
pub fn merge_edit(base: &Event, mine: &Event, theirs: &Event) -> Event {
    fn pick<T: PartialEq + Clone>(base: &T, mine: &T, theirs: &T) -> T {
        if mine == base {
            theirs.clone()
        } else {
            mine.clone()
        }
    }
    let when = |e: &Event| (e.starts_at, e.ends_at, e.all_day, e.timezone.clone());
    let (starts_at, ends_at, all_day, timezone) = pick(&when(base), &when(mine), &when(theirs));
    Event {
        guid: mine.guid.clone(),
        calendar_guid: pick(
            &base.calendar_guid,
            &mine.calendar_guid,
            &theirs.calendar_guid,
        ),
        title: pick(&base.title, &mine.title, &theirs.title),
        location: pick(&base.location, &mine.location, &theirs.location),
        starts_at,
        ends_at,
        all_day,
        timezone,
        url: pick(&base.url, &mine.url, &theirs.url),
        notes: pick(&base.notes, &mine.notes, &theirs.notes),
        etag: theirs.etag.clone(),
    }
}

/// Of the events Apple answered for `known`, the one that is it as Apple holds it now.
///
/// An event is found by its id. One occurrence of a repeating event may come back as itself
/// (an exception made to it, or the day asked for) or only as its series; read through its
/// series, it is the series' fields on this occurrence's own day and times — the series' first
/// date is not this day's, and taking it would move the occurrence there.
#[must_use]
pub fn theirs_for(known: &Event, found: Vec<Event>) -> Option<Event> {
    if let Some(exact) = found.iter().find(|e| e.guid == known.guid) {
        return Some(exact.clone());
    }
    if !is_occurrence(&known.guid) {
        return found.into_iter().next();
    }
    let series = series_of(&known.guid);
    let mut theirs = found.into_iter().find(|e| series_of(&e.guid) == series)?;
    theirs.guid.clone_from(&known.guid);
    theirs.starts_at = known.starts_at;
    theirs.ends_at = known.ends_at;
    theirs.all_day = known.all_day;
    theirs.timezone.clone_from(&known.timezone);
    Some(theirs)
}

/// Apple's calendar service.
pub struct CalendarClient<'a> {
    http: &'a reqwest::Client,
    root: String,
    params: Vec<(String, String)>,
}

impl<'a> CalendarClient<'a> {
    pub fn new(http: &'a reqwest::Client, root: &str, params: Vec<(String, String)>) -> Self {
        Self {
            http,
            root: root.trim_end_matches('/').to_string(),
            params,
        }
    }

    /// Collections plus the events between two dates, formatted `YYYY-MM-DD`.
    pub async fn startup(&self, from: &str, to: &str, tz: &str) -> Result<Snapshot> {
        let body = self.get("/ca/startup", from, to, tz).await?;
        Ok(parse_snapshot(&body))
    }

    /// Events alone, for refreshing a window without refetching the collections.
    pub async fn events(&self, from: &str, to: &str, tz: &str) -> Result<Snapshot> {
        let body = self.get("/ca/events", from, to, tz).await?;
        Ok(parse_snapshot(&body))
    }

    /// Create an event, or replace one that already exists.
    ///
    /// Apple's calendar service takes the whole event on every write rather than a patch, so
    /// an edit sends the event as it should end up. `guid` addresses it; a fresh one is a
    /// create and an existing one is an update.
    ///
    /// The window in the query is not decoration: Apple answers with the events in it, and
    /// sending the day the event falls on is what makes the response carry the event back so
    /// a caller can confirm what was stored rather than assume.
    /// Save an event, creating it where it has no guid yet.
    ///
    /// The calendar id is a path segment, so an empty one does not fail loudly — it builds
    /// `/ca/events//<guid>`, which is a different request entirely. An account with no
    /// calendars is a real account, so it is caught here rather than sent.
    pub async fn put_event(&self, event: &Event, ctag: Option<&str>) -> Result<Snapshot> {
        self.put_event_with_alerts(event, ctag, None).await
    }

    /// [`Self::put_event`], and when `alerts` is given, the event's alerts set to exactly
    /// those — minutes before its start, `0` at the time of the event — as icloud.com writes
    /// them: `Alarm` objects beside the event, each named `<event>:<uuid>`, the event listing
    /// their guids. With `None` the alerts Apple holds are sent back as they are.
    pub async fn put_event_with_alerts(
        &self,
        event: &Event,
        ctag: Option<&str>,
        alerts: Option<&[i64]>,
    ) -> Result<Snapshot> {
        self.put_event_full(event, ctag, alerts, None, None).await
    }

    /// [`Self::put_event_with_alerts`], and when `repeat` is given, the event's repeat set
    /// (`Some(Some(..))`) or cleared (`Some(None)`) as icloud.com writes it: a `Recurrence`
    /// object named `<event>*MME-RID` beside the event, the event naming it. With `None` the
    /// repeat Apple holds goes back as it is, object and all.
    pub async fn put_event_full(
        &self,
        event: &Event,
        ctag: Option<&str>,
        alerts: Option<&[i64]>,
        repeat: Option<Option<&EventRepeat>>,
        scope: Option<Scope>,
    ) -> Result<Snapshot> {
        self.put_event_moved(event, ctag, alerts, repeat, scope, None)
            .await
    }

    /// [`Self::put_event_full`] for an event that may be moving from another calendar:
    /// `from` is that calendar and its ctag. As icloud.com moves one: the event is read where it
    /// is, named with `oldpGuid`, and both calendars go in the client state. Read under the
    /// new calendar it was not found, and was rebuilt from nothing — its alerts, guests and
    /// repeat gone — and written as a second event.
    pub async fn put_event_moved(
        &self,
        event: &Event,
        ctag: Option<&str>,
        alerts: Option<&[i64]>,
        repeat: Option<Option<&EventRepeat>>,
        scope: Option<Scope>,
        from: Option<(&str, Option<&str>)>,
    ) -> Result<Snapshot> {
        let from = from.filter(|(old, _)| *old != event.calendar_guid);
        // One occurrence of a repeating event, sent as though it were an event of its own,
        // made Apple take it as the whole series — measured: a weekly event three times over
        // came back one event, repeating no more. An occurrence is changed only with its
        // scope said, as icloud.com asks "this event or all events?".
        let occurrence = is_occurrence(&event.guid);
        if occurrence && scope.is_none() {
            return Err(Error::Refused(
                "This is one occurrence of a repeating event. Say whether the change is for this event alone.".into(),
            ));
        }
        if event.calendar_guid.trim().is_empty() {
            return Err(Error::Protocol(
                "an event has to go in a calendar, and this account has none yet".into(),
            ));
        }
        // The event's own id is the other half of the path. Empty, it builds
        // `/ca/events/<calendar>/` — a different request, which Apple answers with a bare 400
        // and no reason. A new event is given an id before it is sent, so reaching here
        // without one is a mistake worth naming rather than a network error to puzzle over.
        if event.guid.trim().is_empty() {
            return Err(Error::Protocol(
                "that event has no identifier, so there is nothing to save it as".into(),
            ));
        }
        let day = day_of(event.starts_at, event_zone(event).as_deref());
        // What Apple already has, if this event exists. An event carries far more than this
        // app models — its alerts, its guests, its attachments, its link to the series it
        // repeats in — and Apple takes the whole event on every write. Building one from
        // our own fields alone did not merely drop those, it cleared them: a weekly meeting
        // edited here came back a one-off with no alerts and no guests.
        let detail = match from {
            Some((old, _)) => {
                let mut there = event.clone();
                there.calendar_guid = old.to_string();
                self.event_detail_body(&there, &day).await?
            }
            None => self.event_detail_body(event, &day).await?,
        };
        let existing = detail.as_ref().and_then(event_of);
        // For the whole series from one of its occurrences, the series keeps its own dates —
        // icloud.com sends the master's start and end — moved by however far this occurrence
        // was moved here, so dragging Tuesday's 10:00 to 11:00 moves every one to 11:00.
        let shifted;
        let event = match (scope, existing.as_ref()) {
            (Some(Scope::All), Some(found))
                if occurrence && found.get("recurrenceMaster") != Some(&Value::Bool(true)) =>
            {
                let dates = dates_zone(event.all_day, event.timezone.as_deref());
                let was = found
                    .get("startDate")
                    .map(|d| parse_apple_date(d, dates.as_deref()))
                    .transpose()?;
                let master_start = found
                    .get("masterStartDate")
                    .map(|d| parse_apple_date(d, dates.as_deref()))
                    .transpose()?;
                let master_end = found
                    .get("masterEndDate")
                    .map(|d| parse_apple_date(d, dates.as_deref()))
                    .transpose()?;
                match (was, master_start, master_end) {
                    (Some(was), Some(ms), Some(me)) => {
                        let moved = event.starts_at - was;
                        let mut e = event.clone();
                        e.starts_at = ms + moved;
                        e.ends_at = me + moved + ((event.ends_at - event.starts_at) - (me - ms));
                        shifted = e;
                        &shifted
                    }
                    _ => {
                        return Err(Error::Refused(
                            "Apple did not say where this series begins, so nothing was changed."
                                .into(),
                        ))
                    }
                }
            }
            _ => event,
        };
        let mut event_value = self.event_body(event, existing.as_ref());
        // Whose alerts and repeat these are. For all of a series, the series' — Apple's
        // own name for an alert or repeat object carries the series id; made under one day's
        // id, a change meant for every day belonged to that day alone.
        let owner: String = if occurrence && scope == Some(Scope::All) {
            series_of(&event.guid).to_string()
        } else {
            event.guid.clone()
        };
        let held: Vec<Value> = detail
            .as_ref()
            .and_then(|d| d.get("Alarm"))
            .and_then(Value::as_array)
            .map(|all| {
                all.iter()
                    .filter(|a| belongs_to(a, &event.guid))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let alarms: Vec<Value> = match alerts {
            None => held,
            // Every one asked for: icloud.com shows two, but an event made on a Mac can hold
            // more, and cutting to two deleted the rest from every device.
            Some(minutes) => minutes.iter().map(|m| alarm_for(&owner, *m)).collect(),
        };
        // Only a change of alerts rewrites the event's list of them; otherwise it goes back
        // exactly as Apple sent it, beside the alert objects it names.
        if alerts.is_some() {
            event_value["alarms"] = Value::Array(
                alarms
                    .iter()
                    .filter_map(|a| a.get("guid").cloned())
                    .collect(),
            );
        }
        let mut body = serde_json::json!({
            "Event": event_value,
            "ClientState": client_state(&event.calendar_guid, ctag),
        });
        if let Some((old, old_ctag)) = from {
            body["Event"]["oldpGuid"] = Value::from(old);
            if let Some(list) = body["ClientState"]["Collection"].as_array_mut() {
                list.push(serde_json::json!({ "guid": old, "ctag": old_ctag }));
            }
        }
        if !alarms.is_empty() {
            body["Alarm"] = Value::Array(alarms);
        }
        let held_repeat: Vec<Value> = detail
            .as_ref()
            .and_then(|d| d.get("Recurrence"))
            .and_then(Value::as_array)
            .map(|all| {
                all.iter()
                    .filter(|r| belongs_to(r, &event.guid))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        match repeat {
            None => {
                if !held_repeat.is_empty() {
                    body["Recurrence"] = Value::Array(held_repeat);
                }
            }
            Some(None) => {
                body["Event"]["recurrence"] = Value::Null;
            }
            Some(Some(rule)) => {
                let guid = format!("{owner}*MME-RID");
                let mut object = serde_json::json!({
                    "guid": guid,
                    "pGuid": owner,
                    "freq": rule.freq,
                    "interval": rule.interval.max(1),
                });
                write_repeat(&mut object, rule);
                body["Recurrence"] = Value::Array(vec![object]);
                body["Event"]["recurrence"] = Value::String(guid);
            }
        }
        let mut path = format!("/ca/events/{}/{}", event.calendar_guid, event.guid);
        match scope {
            Some(Scope::This) => {
                // As icloud.com marks "only this event": an exception to the series that had one.
                body["Event"]["recurrenceException"] = Value::Bool(true);
                body["Event"]["hadRecurrence"] = Value::Bool(true);
                body["Event"]["changeRecurring"] = Value::from("this");
                path.push_str("/this");
            }
            Some(Scope::All) if occurrence => {
                body["Event"]["changeRecurring"] = Value::from("all");
                path.push_str("/all");
            }
            // The occurrence's own dates go, not the series': from here on is what changes.
            Some(Scope::Future) if occurrence => {
                body["Event"]["changeRecurring"] = Value::from("future");
                path.push_str("/future");
            }
            _ => {}
        }
        let answer = self
            .post(&path, &body, &day, &day, event_zone(event).as_deref(), None)
            .await?;
        Ok(parse_snapshot(&answer))
    }

    /// Remove an event.
    ///
    /// `etag` is what stops a delete from landing on a version of the event someone changed
    /// on another device in the meantime.
    pub async fn delete_event(&self, event: &Event, ctag: Option<&str>) -> Result<()> {
        self.delete_event_scoped(event, ctag, None).await
    }

    /// [`Self::delete_event`], for an occurrence of a repeating event with its scope said:
    /// `This` removes the one day and leaves the series.
    pub async fn delete_event_scoped(
        &self,
        event: &Event,
        ctag: Option<&str>,
        scope: Option<Scope>,
    ) -> Result<()> {
        if is_occurrence(&event.guid) && scope.is_none() {
            return Err(Error::Refused(
                "This is one occurrence of a repeating event. Say whether to delete this event alone.".into(),
            ));
        }
        if event.calendar_guid.trim().is_empty() {
            return Err(Error::Protocol("that event names no calendar".into()));
        }
        if event.guid.trim().is_empty() {
            return Err(Error::Protocol(
                "that event has no identifier, so there is nothing to delete".into(),
            ));
        }
        let day = day_of(event.starts_at, event_zone(event).as_deref());
        let body = serde_json::json!({
            "Event": { "guid": event.guid, "pGuid": event.calendar_guid, "etag": event.etag },
            "ClientState": client_state(&event.calendar_guid, ctag),
        });
        let mut path = format!("/ca/events/{}/{}", event.calendar_guid, event.guid);
        match scope {
            Some(Scope::This) => path.push_str("/this"),
            Some(Scope::All) if is_occurrence(&event.guid) => path.push_str("/all"),
            Some(Scope::Future) if is_occurrence(&event.guid) => path.push_str("/future"),
            _ => {}
        }
        match self
            .post(
                &path,
                &body,
                &day,
                &day,
                event_zone(event).as_deref(),
                Some("DELETE"),
            )
            .await
        {
            // Deleting what is already gone has done what was asked — but only once Apple's
            // own record confirms it is gone, not on the word of a status code alone.
            // Measured: an event Apple no longer has answers 404 with an empty ChangeSet.
            Err(Error::NotFound) if matches!(self.event_detail(event, &day).await, Ok(None)) => {
                Ok(())
            }
            other => other.map(|_| ()),
        }
    }

    /// Apple's own full record of an event, or `None` for one it has never heard of.
    ///
    /// Asked for before every write so an edit can be merged into what is really there.
    /// Only Apple saying it has no such event means "new": a failed request is an error.
    /// Treating a network blip as "new" built the save from nothing and wrote empty
    /// recurrence, alerts and guests over a real event.
    async fn event_detail(&self, event: &Event, day: &str) -> Result<Option<Value>> {
        Ok(self
            .event_detail_body(event, day)
            .await?
            .as_ref()
            .and_then(event_of))
    }

    /// How an event repeats, as Apple holds it, or `None` for a one-off.
    pub async fn repeat_of(&self, event: &Event) -> Result<Option<EventRepeat>> {
        let day = day_of(event.starts_at, event_zone(event).as_deref());
        let Some(body) = self.event_detail_body(event, &day).await? else {
            return Ok(None);
        };
        Ok(body
            .get("Recurrence")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|r| belongs_to(r, &event.guid))
            .and_then(repeat_from_apple))
    }

    /// An event's guests as Apple holds them, the organizer first. Read only.
    pub async fn guests(&self, event: &Event) -> Result<Vec<Guest>> {
        let day = day_of(event.starts_at, event_zone(event).as_deref());
        let Some(body) = self.event_detail_body(event, &day).await? else {
            return Ok(Vec::new());
        };
        Ok(guests_from(&body, &event.guid))
    }

    /// An event's alerts as Apple holds them: minutes before its start, `0` at its time.
    pub async fn alerts(&self, event: &Event) -> Result<Vec<i64>> {
        let day = day_of(event.starts_at, event_zone(event).as_deref());
        let Some(body) = self.event_detail_body(event, &day).await? else {
            return Ok(Vec::new());
        };
        Ok(body
            .get("Alarm")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|a| belongs_to(a, &event.guid))
            .filter_map(|a| minutes_of(a.get("measurement")?))
            .collect())
    }

    /// The whole of `eventdetail`: the event, and beside it its `Alarm`, `Invitee` and
    /// `Recurrence` objects. Public for the probes, which read Apple's own shape from it.
    pub async fn event_detail_body(&self, event: &Event, day: &str) -> Result<Option<Value>> {
        let mut query = self.params.clone();
        query.push(("clientVersion".into(), "5.1".into()));
        query.push(("lang".into(), "en-us".into()));
        query.push((
            "usertz".into(),
            apple_tz(event_zone(event).as_deref()).to_string(),
        ));
        query.push(("startDate".into(), day.to_string()));
        query.push(("endDate".into(), day.to_string()));

        let url = format!(
            "{}/ca/eventdetail/{}/{}",
            self.root, event.calendar_guid, event.guid
        );
        let response = self.http.get(url).query(&query).send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(Error::Protocol(format!(
                "Apple did not say what this event holds now ({status}), so it was not changed."
            )));
        }
        let body: Value = serde_json::from_str(&crate::bounded::reply(response).await?)
            .map_err(|e| Error::Protocol(format!("event detail: {e}")))?;
        Ok(event_of(&body).map(|_| body))
    }

    /// The event as Apple holds it now, read from the calendar and day it was last known in,
    /// or `None` where Apple no longer has it there — deleted, or moved away, on another
    /// device. Read-only.
    pub async fn current_event(&self, known: &Event) -> Result<Option<Event>> {
        let day = day_of(known.starts_at, event_zone(known).as_deref());
        let Some(body) = self.event_detail_body(known, &day).await? else {
            return Ok(None);
        };
        let found: Vec<Event> = match body.get("Event") {
            Some(Value::Array(all)) => all
                .iter()
                .filter_map(|e| event_from(e).ok().flatten())
                .collect(),
            Some(one @ Value::Object(_)) => event_from(one)?.into_iter().collect(),
            _ => Vec::new(),
        };
        Ok(theirs_for(known, found))
    }

    /// A calendar's ctag as Apple holds it now, for a write retried after a refusal.
    pub async fn current_ctag(&self, guid: &str) -> Result<Option<String>> {
        Ok(self
            .collection_detail(guid)
            .await?
            .and_then(|c| c.get("ctag").and_then(Value::as_str).map(str::to_string)))
    }

    /// Apple's event object, in the shape its own web client sends.
    ///
    /// When Apple already has the event, its own record is the starting point and only the
    /// fields this app actually models are written over it — so alerts, guests,
    /// attachments, private comments and the dates linking it to a repeating series all
    /// survive an edit untouched. Only a genuinely new event is built from nothing.
    fn event_body(&self, event: &Event, existing: Option<&Value>) -> Value {
        let zone = event.timezone.as_deref();
        let dates = dates_zone(event.all_day, zone);
        let minutes = ((event.ends_at - event.starts_at) / 60).max(0);

        let mut body = existing.cloned().unwrap_or_else(|| {
            // A new event has no alerts or guests to keep, so saying so is accurate here in
            // a way it never was for an edit.
            serde_json::json!({
                "icon": 0,
                "recurrence": Value::Null,
                "changeRecurring": Value::Null,
                "alarms": [],
                "invitees": [],
                "hasAttachments": false,
                "isJunk": false,
                "readOnly": false,
                "extendedDetailsAreIncluded": true,
            })
        });
        // What Apple sent is the starting point, and Apple is not obliged to send an object.
        // Anything else is set aside for a fresh one: a write that loses what Apple held is
        // bad, but one that panics on network input takes the whole window down with it.
        if !body.is_object() {
            body = serde_json::json!({});
        }
        let Some(object) = body.as_object_mut() else {
            return serde_json::json!({});
        };

        for (key, value) in [
            ("guid", Value::String(event.guid.clone())),
            ("pGuid", Value::String(event.calendar_guid.clone())),
            ("title", Value::String(event.title.clone())),
            (
                "location",
                Value::String(event.location.clone().unwrap_or_default()),
            ),
            (
                "description",
                Value::String(event.notes.clone().unwrap_or_default()),
            ),
            ("url", event.url.clone().map_or(Value::Null, Value::String)),
            (
                "startDate",
                format_apple_date(event.starts_at, dates.as_deref()),
            ),
            (
                "endDate",
                format_apple_date(event.ends_at, dates.as_deref()),
            ),
            (
                "localStartDate",
                format_apple_date(event.starts_at, dates.as_deref()),
            ),
            (
                "localEndDate",
                format_apple_date(event.ends_at, dates.as_deref()),
            ),
            ("duration", Value::from(minutes)),
            ("allDay", Value::Bool(event.all_day)),
            (
                "tz",
                zone.map_or(Value::Null, |z| Value::String(z.to_string())),
            ),
            (
                "etag",
                event.etag.clone().map_or(Value::Null, Value::String),
            ),
        ] {
            object.insert(key.to_string(), value);
        }
        body
    }

    /// Which calendars changed since `known` — each calendar's guid and the ctag last seen —
    /// from `/ca/idle`, icloud.com's own "has anything changed" call. It does not wait: it
    /// answers in a few hundred milliseconds with the calendars whose ctag is not the one
    /// sent, any it was not told of, and any gone. Nothing changed is an empty list.
    ///
    /// Measured: the body is `{ClientState: {Collection: [{guid, ctag}], alarmRange,
    /// fullState: true, userTime}}`, and the answer is `{ChangeSet: {inserts|updates|deletes:
    /// {Collection: [...]}}}`. It needs the same query a read does, dates included.
    pub async fn changed(
        &self,
        known: &[(String, String)],
        tz: &str,
    ) -> Result<Vec<(String, Option<String>)>> {
        let today = jiff::Zoned::now().date().to_string();
        let state: Vec<Value> = known
            .iter()
            .map(|(guid, ctag)| serde_json::json!({ "guid": guid, "ctag": ctag }))
            .collect();
        let body = serde_json::json!({ "ClientState": {
            "Collection": state, "alarmRange": 60, "fullState": true,
            "userTime": jiff::Timestamp::now().as_second(),
        } });
        let answer = self
            .post("/ca/idle", &body, &today, &today, Some(tz), None)
            .await?;
        Ok(changed_collections(&answer))
    }

    /// One calendar exactly as Apple holds it (`ca/collectiondetail`), etag and all, or
    /// `None` for one it has never heard of. Read-only.
    pub async fn collection_detail(&self, guid: &str) -> Result<Option<Value>> {
        let today = day_of(now_seconds(), None);
        match self
            .get(
                &format!("/ca/collectiondetail/{guid}"),
                &today,
                &today,
                "UTC",
            )
            .await
        {
            Ok(body) => Ok(collection_of(&body, guid)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Make a calendar, as icloud.com's "New Calendar" does. The id is a fresh lowercase
    /// UUID, and the colour is the first of Apple's seven that no calendar uses yet.
    /// A title already in use is refused, as icloud.com refuses it, because two calendars
    /// with one name cannot be told apart anywhere. Answers the new calendar's guid once
    /// Apple's own record shows it.
    pub async fn create_calendar(&self, title: &str, existing: &[Calendar]) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::Refused("A calendar needs a name.".into()));
        }
        if existing.iter().any(|c| c.title.trim() == title) {
            return Err(Error::Refused(format!(
                "A calendar with the name \u{201c}{title}\u{201d} already exists."
            )));
        }
        let (color, symbolic) = next_colour(existing);
        let guid = uuid::Uuid::new_v4().to_string();
        let order = existing.iter().map(|c| c.sort_order).max().unwrap_or(0) + 1;
        let collection = serde_json::json!({
            "guid": guid, "title": title, "color": color, "symbolicColor": symbolic,
            "order": order, "visible": true, "shareTitle": "", "supportedType": "Event",
            "objectType": "personal", "extendedDetailsAreIncluded": false, "readOnly": false,
        });
        self.save_collection(&guid, collection, None, None).await?;
        match self.collection_detail(&guid).await? {
            Some(_) => Ok(guid),
            None => Err(Error::Refused(
                "iCloud took the new calendar but does not show it, so nothing is assumed made."
                    .into(),
            )),
        }
    }

    /// Rename or recolour a calendar: Apple's own record of it, with only what changed set,
    /// sent with its etag, so a change made elsewhere meanwhile is refused rather than overwritten.
    pub async fn update_calendar(
        &self,
        guid: &str,
        title: Option<&str>,
        colour: Option<&str>,
        existing: &[Calendar],
    ) -> Result<()> {
        let Some(mut collection) = self.collection_detail(guid).await? else {
            return Err(Error::NotFound);
        };
        if collection.get("readOnly").and_then(Value::as_bool) == Some(true) {
            return Err(Error::Refused(
                "That calendar can't be changed from here.".into(),
            ));
        }
        if let Some(title) = title.map(str::trim) {
            if title.is_empty() {
                return Err(Error::Refused("A calendar needs a name.".into()));
            }
            if existing
                .iter()
                .any(|c| c.guid != guid && c.title.trim() == title)
            {
                return Err(Error::Refused(format!(
                    "A calendar with the name \u{201c}{title}\u{201d} already exists."
                )));
            }
            collection["title"] = title.into();
        }
        if let Some(colour) = colour {
            collection["color"] = colour.into();
            collection["symbolicColor"] = symbolic_of(colour).into();
        }
        let etag = collection
            .get("etag")
            .and_then(Value::as_str)
            .map(str::to_string);
        let ctag = collection
            .get("ctag")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.save_collection(
            guid,
            collection.clone(),
            Some("PUT"),
            etag.as_deref().zip(Some(ctag.as_deref())),
        )
        .await?;
        let after = self.collection_detail(guid).await?.ok_or(Error::NotFound)?;
        let took = title
            .is_none_or(|t| after.get("title").and_then(Value::as_str) == Some(t.trim()))
            && colour.is_none_or(|c| {
                after
                    .get("color")
                    .and_then(Value::as_str)
                    .map(str::to_lowercase)
                    == Some(c.to_lowercase())
            });
        if took {
            Ok(())
        } else {
            Err(Error::Refused("iCloud did not keep that change.".into()))
        }
    }

    /// Delete a calendar and every event in it, as icloud.com does. The default calendar and
    /// one shared to this account are refused here; Apple would refuse them in any case.
    pub async fn delete_calendar(&self, calendar: &Calendar) -> Result<()> {
        if calendar.is_default {
            return Err(Error::Refused(
                "This is the calendar new events go in. Choose another default on an Apple device first.".into(),
            ));
        }
        if !calendar.writable {
            return Err(Error::Refused(
                "That calendar can't be deleted from here.".into(),
            ));
        }
        let today = day_of(now_seconds(), None);
        match self
            .post(
                &format!("/ca/collections/{}", calendar.guid),
                &serde_json::json!({}),
                &today,
                &today,
                None,
                Some("DELETE"),
            )
            .await
        {
            Ok(_) | Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        // Gone only once Apple's own record says so.
        match self.collection_detail(&calendar.guid).await? {
            None => Ok(()),
            Some(_) => Err(Error::Refused("iCloud still has that calendar.".into())),
        }
    }

    async fn save_collection(
        &self,
        guid: &str,
        collection: Value,
        method: Option<&str>,
        tags: Option<(&str, Option<&str>)>,
    ) -> Result<Value> {
        let today = day_of(now_seconds(), None);
        let body = serde_json::json!({
            "Collection": collection,
            "ClientState": { "Collection": [{ "guid": guid, "ctag": tags.and_then(|t| t.1) }], "fullState": false },
        });
        // icloud.com sends the etag as `ifMatch`, beside methodOverride, in the query.
        let extra: Vec<(&str, &str)> = tags
            .map(|(etag, _)| ("ifMatch", etag))
            .into_iter()
            .collect();
        self.post_with(
            &format!("/ca/collections/{guid}"),
            &body,
            &today,
            &today,
            None,
            method,
            &extra,
        )
        .await
    }

    async fn post(
        &self,
        path: &str,
        body: &Value,
        from: &str,
        to: &str,
        tz: Option<&str>,
        method_override: Option<&str>,
    ) -> Result<Value> {
        self.post_with(path, body, from, to, tz, method_override, &[])
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn post_with(
        &self,
        path: &str,
        body: &Value,
        from: &str,
        to: &str,
        tz: Option<&str>,
        method_override: Option<&str>,
        extra: &[(&str, &str)],
    ) -> Result<Value> {
        let mut query = self.params.clone();
        query.extend(
            extra
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string())),
        );
        query.push(("clientVersion".into(), "5.1".into()));
        query.push(("requestID".into(), "4".into()));
        query.push(("startDate".into(), from.into()));
        query.push(("endDate".into(), to.into()));
        query.push(("lang".into(), "en-us".into()));
        query.push(("usertz".into(), apple_tz(tz).into()));
        if let Some(method) = method_override {
            query.push(("methodOverride".into(), method.into()));
        }

        let response = self
            .http
            .post(format!("{}{path}", self.root))
            .query(&query)
            .json(body)
            .send()
            .await?;
        let status = response.status();
        let text = crate::bounded::reply(response).await?;

        if crate::session_lapsed(status) {
            return Err(Error::ReauthRequired);
        }
        if status.as_u16() == 404 {
            return Err(Error::NotFound);
        }
        if !status.is_success() {
            return Err(refusal(path, status, &text));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// A window of the calendar is a few kilobytes: one not answered in this long is a stalled
    /// connection, asked again (sync.rs) rather than waited on for the client's minute.
    pub(crate) const READ_TIMEOUT: std::time::Duration = if cfg!(test) {
        std::time::Duration::from_secs(1)
    } else {
        std::time::Duration::from_secs(20)
    };

    async fn get(&self, path: &str, from: &str, to: &str, tz: &str) -> Result<Value> {
        let mut query = self.params.clone();
        // Apple's calendar service returns an empty body unless the web client's own
        // versioning parameters are present. Without `clientVersion` it answers 200 with no
        // events at all, which looks exactly like an empty calendar.
        query.push(("clientVersion".into(), "5.1".into()));
        query.push(("requestID".into(), "4".into()));
        query.push(("startDate".into(), from.into()));
        query.push(("endDate".into(), to.into()));
        query.push(("lang".into(), "en-us".into()));
        query.push(("usertz".into(), tz.into()));

        let response = self
            .http
            .get(format!("{}{path}", self.root))
            .query(&query)
            .timeout(Self::READ_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        let text = crate::bounded::reply(response).await?;

        if crate::session_lapsed(status) {
            return Err(Error::ReauthRequired);
        }
        if status.as_u16() == 404 {
            return Err(Error::NotFound);
        }
        if !status.is_success() {
            return Err(refusal(path, status, &text));
        }
        serde_json::from_str(&text)
            .map_err(|e| Error::Protocol(format!("{path} returned invalid JSON: {e}")))
    }
}

/// The calendars an `/ca/idle` answer says changed: guid and new ctag, `None` when gone.
#[must_use]
pub fn changed_collections(answer: &Value) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for (kind, gone) in [("inserts", false), ("updates", false), ("deletes", true)] {
        let Some(list) = answer
            .pointer(&format!("/ChangeSet/{kind}/Collection"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for c in list {
            // A delete may name the calendar by guid alone, or be the guid itself.
            let guid = c.get("guid").and_then(Value::as_str).or_else(|| c.as_str());
            if let Some(guid) = guid {
                let ctag = if gone {
                    None
                } else {
                    c.get("ctag").and_then(Value::as_str).map(str::to_string)
                };
                out.push((guid.to_string(), ctag));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Guests as an eventdetail answer holds them: named by the event, or naming it; the
    /// organizer first; answers read whatever their case; anything unknown "pending".
    #[test]
    fn guests_are_read_from_the_invitees_an_event_names() {
        let body = json!({
            "Event": [{ "guid": "E1", "invitees": ["I1", "I2", "I3"] }],
            "Invitee": [
                { "guid": "I1", "commonName": "Zara", "email": "mailto:zara@example.com", "inviteeStatus": "ACCEPTED", "isMe": false, "isOrganizer": false },
                { "guid": "I2", "commonName": "Ana", "email": "ana@example.com", "inviteeStatus": "needs-action", "isMe": true, "isOrganizer": false },
                { "guid": "I3", "commonName": " ", "email": "boss@example.com", "inviteeStatus": "TENTATIVE", "isOrganizer": true },
                { "guid": "I9", "pGuid": "OTHER", "commonName": "Not here", "inviteeStatus": "ACCEPTED" },
                { "guid": "I4", "pGuid": "E1", "commonName": "Ravi", "inviteeStatus": "DELEGATED" },
            ]
        });
        let g = guests_from(&body, "E1");
        let shown: Vec<_> = g
            .iter()
            .map(|g| {
                (
                    g.name.as_deref(),
                    g.email.as_deref(),
                    g.status,
                    g.is_me,
                    g.is_organizer,
                )
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                (None, Some("boss@example.com"), "tentative", false, true),
                (Some("Ana"), Some("ana@example.com"), "pending", true, false),
                (Some("Ravi"), None, "pending", false, false),
                (
                    Some("Zara"),
                    Some("zara@example.com"),
                    "accepted",
                    false,
                    false
                ),
            ]
        );
    }

    /// One person listed twice — the organizer also among those invited, as an invitation from
    /// another calendar lists them, their address written in another case — is one guest: the
    /// organizer, with the name either copy gives. Sorted apart by name, both copies were shown.
    #[test]
    fn a_guest_listed_twice_is_shown_once() {
        let body = json!({
            "Event": [{ "guid": "E1", "invitees": ["I1", "I2", "I3"] }],
            "Invitee": [
                { "guid": "I1", "email": "mailto:Zoe@Example.com", "inviteeStatus": "ACCEPTED", "isOrganizer": true },
                { "guid": "I2", "commonName": "Amir", "email": "amir@example.com", "inviteeStatus": "ACCEPTED" },
                { "guid": "I3", "commonName": "Zoe Park", "email": "mailto:zoe@example.com", "inviteeStatus": "ACCEPTED", "isMe": true },
            ]
        });
        let g = guests_from(&body, "E1");
        let shown: Vec<_> = g
            .iter()
            .map(|g| (g.name.as_deref(), g.is_organizer, g.is_me))
            .collect();
        assert_eq!(
            shown,
            vec![(Some("Zoe Park"), true, true), (Some("Amir"), false, false)]
        );
    }

    /// An occurrence of a repeating event finds its series' guests; a body with none, or
    /// shaped otherwise, gives none — never a panic.
    #[test]
    fn guests_of_an_occurrence_and_of_odd_bodies() {
        let body = json!({ "Event": [{ "guid": "S1", "invitees": ["I1"] }], "Invitee": [{ "guid": "I1", "email": "a@example.com", "inviteeStatus": "DECLINED" }] });
        assert_eq!(
            guests_from(&body, "S1__20261102T073300Z")[0].status,
            "declined"
        );
        assert!(guests_from(&json!({}), "E1").is_empty());
        assert!(guests_from(&json!({ "Invitee": "nope", "Event": 3 }), "E1").is_empty());
    }

    fn meeting() -> Event {
        Event {
            guid: "EV".into(),
            calendar_guid: "work".into(),
            title: "Planning".into(),
            location: Some("Room 1".into()),
            starts_at: 1_000_000,
            ends_at: 1_003_600,
            all_day: false,
            timezone: Some("Asia/Kolkata".into()),
            url: None,
            notes: None,
            etag: Some("v1".into()),
        }
    }

    /// Edits to different fields on two devices both survive; Apple's newest etag goes with it.
    #[test]
    fn a_conflict_on_different_fields_keeps_both_edits() {
        let base = meeting();
        let mut mine = base.clone();
        mine.title = "Planning, moved".into();
        mine.starts_at += 3600;
        mine.ends_at += 3600;
        let mut theirs = base.clone();
        theirs.location = Some("Room 7".into());
        theirs.notes = Some("Bring the numbers".into());
        theirs.etag = Some("v2".into());
        let merged = merge_edit(&base, &mine, &theirs);
        assert_eq!(merged.title, "Planning, moved");
        assert_eq!(merged.starts_at, base.starts_at + 3600);
        assert_eq!(merged.ends_at, base.ends_at + 3600);
        assert_eq!(merged.location.as_deref(), Some("Room 7"));
        assert_eq!(merged.notes.as_deref(), Some("Bring the numbers"));
        assert_eq!(merged.etag.as_deref(), Some("v2"));
    }

    /// The same field changed on both: this edit's, and their other changes are still kept.
    /// When it happens moves as one — never their start with this end.
    #[test]
    fn a_conflict_on_the_same_field_keeps_this_edit_for_it() {
        let base = meeting();
        let mut mine = base.clone();
        mine.title = "Mine".into();
        mine.ends_at += 1800;
        let mut theirs = base.clone();
        theirs.title = "Theirs".into();
        theirs.starts_at += 7200;
        theirs.ends_at += 7200;
        theirs.url = Some("https://example.com".into());
        theirs.etag = Some("v3".into());
        let merged = merge_edit(&base, &mine, &theirs);
        assert_eq!(merged.title, "Mine");
        assert_eq!(
            (merged.starts_at, merged.ends_at),
            (base.starts_at, base.ends_at + 1800)
        );
        assert_eq!(merged.url.as_deref(), Some("https://example.com"));
        assert_eq!(merged.etag.as_deref(), Some("v3"));
        // Nothing edited here: theirs, whole.
        let untouched = merge_edit(&base, &base, &theirs);
        assert_eq!(untouched, theirs);
    }

    fn coloured(colour: &str) -> Calendar {
        Calendar {
            guid: colour.into(),
            title: colour.into(),
            colour: Some(colour.into()),
            ctag: None,
            sort_order: 0,
            enabled: true,
            writable: true,
            is_default: false,
        }
    }

    /// A new calendar takes the first of Apple's colours no calendar has, as icloud.com does,
    /// matching Apple's colours whatever their case or alpha; and red again when all are used.
    #[test]
    fn a_new_calendar_takes_the_first_unused_colour() {
        assert_eq!(next_colour(&[]), ("#ff2d55", "red"));
        let some = [coloured("#FF2D55FF"), coloured("#ff9500")];
        assert_eq!(next_colour(&some), ("#ffcc00", "yellow"));
        let all: Vec<Calendar> = PALETTE.iter().map(|(hex, _)| coloured(hex)).collect();
        assert_eq!(next_colour(&all), ("#ff2d55", "red"));
        assert_eq!(symbolic_of("#34AADC"), "blue");
        assert_eq!(symbolic_of("#123456"), "custom");
    }

    /// The shapes `/ca/idle` was measured answering with: nothing, a calendar it was not told
    /// of, a calendar whose tag moved; and a calendar gone.
    #[test]
    fn idle_answers_read_as_the_calendars_that_changed() {
        assert!(
            changed_collections(&json!({"ChangeSet":{"UpcomingAlarm":[],"fullState":true}}))
                .is_empty()
        );
        let answer = json!({"ChangeSet":{"fullState":true,
            "inserts":{"Collection":[{"guid":"MMEBirthdays","ctag":"c1","title":"Untitled"}]},
            "updates":{"Collection":[{"guid":"411F","ctag":"c2"}]},
            "deletes":{"Collection":[{"guid":"OLD"}]}}});
        assert_eq!(
            changed_collections(&answer),
            vec![
                ("MMEBirthdays".to_string(), Some("c1".to_string())),
                ("411F".to_string(), Some("c2".to_string())),
                ("OLD".to_string(), None),
            ]
        );
    }

    #[test]
    fn a_date_is_read_in_its_own_timezone() {
        // 14:30 in Kolkata is 09:00 UTC — the zone is what makes this unambiguous.
        let raw = json!([20260912, 2026, 9, 12, 14, 30, 870]);
        let seconds = parse_apple_date(&raw, Some("Asia/Kolkata")).unwrap();
        let utc = jiff::Timestamp::from_second(seconds).unwrap();
        assert_eq!(utc.to_string(), "2026-09-12T09:00:00Z");
    }

    /// A meeting an Exchange organiser sent: Apple passes on the zone it was made in by its
    /// Windows name. Read as UTC it was drawn two hours late; it is Berlin, and kept by
    /// Berlin's IANA name for the window and for an edit sent back.
    #[test]
    fn an_event_in_a_windows_zone_is_read_in_that_zone() {
        let event = event_from(&json!({
            "guid": "EX", "pGuid": "work", "title": "Review",
            "tz": "W. Europe Standard Time", "allDay": false,
            "startDate": [20260715, 2026, 7, 15, 10, 0, 600],
            "endDate": [20260715, 2026, 7, 15, 11, 0, 660],
        }))
        .unwrap()
        .unwrap();
        assert_eq!(event.timezone.as_deref(), Some("Europe/Berlin"));
        let start = jiff::Timestamp::from_second(event.starts_at).unwrap();
        assert_eq!(start.to_string(), "2026-07-15T08:00:00Z");
        assert_eq!(event.ends_at - event.starts_at, 3600);
        // And the date read back in that zone is the one Apple sent.
        let raw = json!([20260715, 2026, 7, 15, 10, 0, 600]);
        assert_eq!(
            format_apple_date(event.starts_at, Some("W. Europe Standard Time")),
            raw
        );
        // A name no zone database knows stays as Apple sent it.
        let odd = event_from(&json!({
            "guid": "O", "pGuid": "work", "tz": "Nowhere Time",
            "startDate": [20260715, 2026, 7, 15, 10, 0, 600],
        }))
        .unwrap()
        .unwrap();
        assert_eq!(odd.timezone.as_deref(), Some("Nowhere Time"));
    }

    #[test]
    fn the_same_reading_in_another_zone_is_another_instant() {
        let raw = json!([20260912, 2026, 9, 12, 14, 30, 870]);
        let kolkata = parse_apple_date(&raw, Some("Asia/Kolkata")).unwrap();
        let new_york = parse_apple_date(&raw, Some("America/New_York")).unwrap();
        assert_ne!(
            kolkata, new_york,
            "wall-clock time alone does not identify an instant"
        );
        // New York is behind, so the same reading happens later in absolute terms.
        assert!(new_york > kolkata);
    }

    /// The hour that does not exist on a spring-forward day must not fail a sync.
    #[test]
    fn a_daylight_saving_gap_resolves_rather_than_erroring() {
        // 02:30 on 8 March 2026 never happens in New York.
        let raw = json!([20260308, 2026, 3, 8, 2, 30, 150]);
        let seconds = parse_apple_date(&raw, Some("America/New_York")).unwrap();
        assert!(
            seconds > 0,
            "a nonexistent local time should shift, not blow up"
        );
    }

    #[test]
    fn an_unknown_timezone_falls_back_to_utc() {
        let raw = json!([20260912, 2026, 9, 12, 14, 30, 870]);
        let unknown = parse_apple_date(&raw, Some("Mars/Olympus_Mons")).unwrap();
        let utc = parse_apple_date(&raw, None).unwrap();
        assert_eq!(unknown, utc, "one bad zone must not lose the event");
    }

    /// Network input must never panic, however malformed.
    #[test]
    fn malformed_dates_are_errors_not_panics() {
        for bad in [
            json!("2026-09-12"),
            json!([]),
            json!([20260912, 2026, 9]),
            json!([20260912, 2026, 99, 12, 14, 30]), // month out of range
            json!([20260912, 2026, 2, 31, 14, 30]),  // a day that does not exist
            json!([20260912, 2026, 9, 12, 99, 30]),  // hour out of range
            json!([20260912, 2026, 9, 12, 14, 999]), // minute out of range
            json!([20260912, 2026, 0, 12, 14, 30]),  // zero month
            json!([20260912, 2026, 9, 12, "two", 30]),
            json!([20260912, 999_999, 9, 12, 14, 30]),
        ] {
            assert!(parse_apple_date(&bad, None).is_err(), "should reject {bad}");
        }
    }

    /// A date written back must read back as the same instant, or a completion would land on
    /// the wrong day.
    #[test]
    fn dates_survive_a_round_trip() {
        for zone in [Some("Asia/Kolkata"), Some("America/New_York"), None] {
            for seconds in [0_i64, 1_757_000_000, 1_788_888_888] {
                let encoded = format_apple_date(seconds, zone);
                let decoded = parse_apple_date(&encoded, zone).unwrap();
                // Apple's format carries no seconds, so equality is to the minute.
                assert_eq!(
                    decoded,
                    seconds - seconds.rem_euclid(60),
                    "{seconds} in {zone:?} did not survive"
                );
            }
        }
    }

    #[test]
    fn an_encoded_date_has_apples_shape() {
        let encoded = format_apple_date(1_757_000_000, Some("UTC"));
        let parts = encoded.as_array().unwrap();
        assert_eq!(parts.len(), 7);
        // First element packs the date as YYYYMMDD.
        let packed = parts[0].as_i64().unwrap();
        assert_eq!(packed / 10_000, parts[1].as_i64().unwrap(), "year");
        assert_eq!((packed / 100) % 100, parts[2].as_i64().unwrap(), "month");
        assert_eq!(packed % 100, parts[3].as_i64().unwrap(), "day");
        // Last element is minutes since midnight.
        assert_eq!(
            parts[6].as_i64().unwrap(),
            parts[4].as_i64().unwrap() * 60 + parts[5].as_i64().unwrap()
        );
    }

    #[test]
    fn a_snapshot_reads_collections_and_events() {
        let body = json!({
            "Collection": [
                { "guid": "cal-1", "title": "Work", "color": "#C8912A", "ctag": "c1", "order": 1 },
                { "guid": "cal-2", "title": "Home", "enabled": false }
            ],
            "Event": [
                {
                    "guid": "ev-1", "pGuid": "cal-1", "title": "Standup",
                    "location": "Room 2", "tz": "Asia/Kolkata",
                    "startDate": [20260912, 2026, 9, 12, 9, 30, 570],
                    "endDate":   [20260912, 2026, 9, 12, 9, 45, 585],
                    "etag": "e1"
                }
            ]
        });
        let snap = parse_snapshot(&body);

        assert_eq!(snap.calendars.len(), 2);
        assert_eq!(snap.calendars[0].colour.as_deref(), Some("#C8912A"));
        assert!(
            snap.calendars[0].enabled,
            "a calendar with no `enabled` flag is on"
        );
        assert!(!snap.calendars[1].enabled);

        assert_eq!(snap.events.len(), 1);
        let event = &snap.events[0];
        assert_eq!(event.title, "Standup");
        assert_eq!(event.calendar_guid, "cal-1");
        assert_eq!(event.ends_at - event.starts_at, 15 * 60);
    }

    #[test]
    fn an_event_without_an_end_uses_its_duration() {
        let body = json!({ "Event": [{
            "guid": "ev-1", "pGuid": "cal-1", "title": "Focus",
            "startDate": [20260912, 2026, 9, 12, 9, 0, 540],
            "duration": 90
        }]});
        let snap = parse_snapshot(&body);
        assert_eq!(snap.events[0].ends_at - snap.events[0].starts_at, 90 * 60);
    }

    #[test]
    fn an_end_before_its_start_is_clamped() {
        let body = json!({ "Event": [{
            "guid": "ev-1", "pGuid": "cal-1", "title": "Backwards",
            "startDate": [20260912, 2026, 9, 12, 10, 0, 600],
            "endDate":   [20260912, 2026, 9, 12, 9, 0, 540]
        }]});
        let event = &parse_snapshot(&body).events[0];
        assert_eq!(
            event.ends_at, event.starts_at,
            "a negative duration is not storable"
        );
    }

    /// One unparseable row from years ago must not block today's sync.
    #[test]
    fn one_bad_event_does_not_lose_the_good_ones() {
        let body = json!({ "Event": [
            { "guid": "bad", "pGuid": "cal-1", "startDate": "not-an-array" },
            { "guid": "no-identity", "title": "orphan" },
            { "guid": "good", "pGuid": "cal-1", "title": "Fine",
              "startDate": [20260912, 2026, 9, 12, 9, 0, 540], "duration": 30 }
        ]});
        let snap = parse_snapshot(&body);
        assert_eq!(snap.events.len(), 1);
        assert_eq!(snap.events[0].guid, "good");
    }

    #[test]
    fn an_empty_body_is_an_empty_snapshot() {
        let snap = parse_snapshot(&json!({}));
        assert!(snap.calendars.is_empty() && snap.events.is_empty());
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn event() -> Event {
        Event {
            guid: "E-1".into(),
            calendar_guid: "work".into(),
            title: "Dentist".into(),
            location: Some("Gachibowli".into()),
            // 2026-09-14T09:30:00+05:30
            starts_at: 1_789_358_400,
            ends_at: 1_789_362_000,
            all_day: false,
            timezone: Some("Asia/Kolkata".into()),
            url: None,
            notes: None,
            etag: None,
        }
    }

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": "E-1",
                    "pGuid": "work",
                    "title": "Dentist",
                    "tz": "Asia/Kolkata",
                    "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
                    "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
                    "etag": "stored"
                }]
            })))
            .mount(&server)
            .await;
        server
    }

    /// The write itself, not the detail fetch that precedes it — a write reads what Apple
    /// already has so an edit can be merged into it rather than replace it.
    async fn request(server: &MockServer) -> Request {
        server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.method == reqwest::Method::POST)
            .expect("a write was sent")
    }

    #[tokio::test]
    async fn a_write_addresses_the_event_by_calendar_and_guid() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), Some("ctag-1")).await.unwrap();
        assert_eq!(request(&server).await.url.path(), "/ca/events/work/E-1");
    }

    /// Apple answers with the events in the window it was given, so sending the event's own
    /// day is what makes the reply carry the stored event back.
    #[tokio::test]
    async fn the_window_is_the_day_the_event_falls_on_locally() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), None).await.unwrap();

        let sent = request(&server).await;
        let query: std::collections::HashMap<_, _> = sent.url.query_pairs().collect();
        // 04:00 UTC on the 14th is 09:30 on the 14th in Kolkata — the local day is what counts.
        assert_eq!(query["startDate"], "2026-09-14");
        assert_eq!(query["endDate"], "2026-09-14");
        assert_eq!(query["usertz"], "Asia/Kolkata");
    }

    #[tokio::test]
    async fn the_whole_event_is_sent_in_apples_own_shape() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), Some("ctag-1")).await.unwrap();

        let sent = request(&server).await;
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        let e = &body["Event"];
        assert_eq!(e["title"], "Dentist");
        assert_eq!(e["location"], "Gachibowli");
        assert_eq!(e["duration"], 60, "duration is minutes, not seconds");
        assert_eq!(e["tz"], "Asia/Kolkata");
        // The date is Apple's component array in the event's own zone, not a timestamp.
        assert_eq!(
            e["startDate"],
            serde_json::json!([20260914, 2026, 9, 14, 9, 30, 570])
        );
        assert_eq!(body["ClientState"]["Collection"][0]["ctag"], "ctag-1");
    }

    /// What comes back is what gets stored — never what was sent.
    #[tokio::test]
    async fn the_stored_event_is_read_from_apples_reply() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let answer = client.put_event(&event(), None).await.unwrap();
        assert_eq!(answer.events.len(), 1);
        assert_eq!(answer.events[0].etag.as_deref(), Some("stored"));
    }

    #[tokio::test]
    async fn deleting_says_so_in_the_query_and_carries_the_etag() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut gone = event();
        gone.etag = Some("v3".into());
        client.delete_event(&gone, None).await.unwrap();

        let sent = request(&server).await;
        let query: std::collections::HashMap<_, _> = sent.url.query_pairs().collect();
        assert_eq!(query["methodOverride"], "DELETE");
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(
            body["Event"]["etag"], "v3",
            "a delete must not race another device"
        );
    }

    /// An all-day event has no meaningful duration, and a negative one is nonsense.
    #[tokio::test]
    async fn a_zero_length_event_reports_zero_minutes() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut instant = event();
        instant.ends_at = instant.starts_at;
        client.put_event(&instant, None).await.unwrap();

        let body: Value = serde_json::from_slice(&request(&server).await.body).unwrap();
        assert_eq!(body["Event"]["duration"], 0);
    }

    /// The one that matters. An event carries alerts, guests, attachments and the dates
    /// linking it to the series it repeats in; Apple takes the whole event on every write.
    /// Building one from our own fields alone did not merely drop those, it cleared them —
    /// a weekly meeting edited here came back a one-off with no alerts and no guests.
    #[tokio::test]
    async fn an_edit_keeps_the_alerts_guests_and_repeat_apple_holds() {
        let server = MockServer::start().await;
        // Apple's own record of the event, with everything this app does not model.
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": "E-1",
                    "pGuid": "work",
                    "title": "Dentist",
                    "tz": "Asia/Kolkata",
                    "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
                    "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
                    "etag": "v1",
                    "alarms": [{ "messageType": "message", "measurement": { "before": true, "minutes": 15 } }],
                    "invitees": [{ "email": "someone@example.invalid" }],
                    "attachments": [{ "name": "form.pdf" }],
                    "privateComments": [{ "comment": "bring the referral" }],
                    "recurrenceMaster": true,
                    "masterStartDate": [20260901, 2026, 9, 1, 9, 30, 570],
                    "transparent": true
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut renamed = event();
        renamed.title = "Dentist — moved".into();
        client.put_event(&renamed, None).await.unwrap();

        let body: Value = serde_json::from_slice(&request(&server).await.body).unwrap();
        let sent = &body["Event"];

        assert_eq!(sent["title"], "Dentist — moved", "the change lands");
        // And nothing else was taken away with it.
        assert_eq!(
            sent["alarms"][0]["measurement"]["minutes"], 15,
            "alerts survive"
        );
        assert_eq!(
            sent["invitees"][0]["email"], "someone@example.invalid",
            "guests survive"
        );
        assert_eq!(
            sent["attachments"][0]["name"], "form.pdf",
            "attachments survive"
        );
        assert_eq!(
            sent["privateComments"][0]["comment"], "bring the referral",
            "private comments survive"
        );
        assert_eq!(sent["recurrenceMaster"], true, "the repeat survives");
        assert!(
            sent.get("masterStartDate").is_some(),
            "its series link survives"
        );
        assert_eq!(sent["transparent"], true, "free/busy survives");
    }

    /// A genuinely new event has no alerts or guests to keep, so saying so is accurate
    /// there in a way it never was for an edit.
    #[tokio::test]
    async fn a_new_event_is_still_built_from_nothing() {
        let server = MockServer::start().await;
        // Apple has never heard of it: no detail comes back.
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.put_event(&event(), None).await.unwrap();

        let body: Value = serde_json::from_slice(&request(&server).await.body).unwrap();
        assert_eq!(body["Event"]["title"], "Dentist");
        assert_eq!(body["Event"]["alarms"], serde_json::json!([]));
    }

    /// One occurrence of a repeating event, sent as an event, collapsed the series on a real
    /// account. Without its scope it is refused before anything is sent.
    #[tokio::test]
    async fn an_occurrence_is_not_saved_or_deleted_without_its_scope() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut one = event();
        one.guid = "SERIES__20261102T073300Z".into();
        assert!(client.put_event(&one, None).await.is_err());
        assert!(client.delete_event(&one, None).await.is_err());
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "nothing sent"
        );
    }

    /// With "this event" said, it goes to the occurrence's own path, marked as an exception.
    #[tokio::test]
    async fn this_occurrence_alone_goes_to_its_this_path() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut one = event();
        one.guid = "SERIES__20261102T073300Z".into();
        client
            .put_event_full(&one, None, None, None, Some(Scope::This))
            .await
            .unwrap();
        let sent = request(&server).await;
        assert!(
            sent.url.path().ends_with("/SERIES__20261102T073300Z/this"),
            "{}",
            sent.url.path()
        );
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(body["Event"]["recurrenceException"], true);
        assert_eq!(body["Event"]["changeRecurring"], "this");
    }

    /// A conflict on one occurrence of a repeating event is settled, not refused: Apple's copy
    /// of that day is read again — found through its series — this edit laid over it, and
    /// saved once more on the occurrence's own path, its own day, against Apple's newest tags.
    #[tokio::test]
    async fn a_conflict_on_one_occurrence_is_read_again_and_merged() {
        let server = MockServer::start().await;
        let occurrence = "SERIES__20260914T040000Z";
        // Apple reads the day back as its series, changed on a phone since: a new room.
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": "SERIES",
                    "pGuid": "work",
                    "title": "Dentist",
                    "location": "Room 2",
                    "tz": "Asia/Kolkata",
                    "startDate": [20260901, 2026, 9, 1, 9, 30, 570],
                    "endDate": [20260901, 2026, 9, 1, 10, 30, 630],
                    "etag": "v2",
                    "recurrenceMaster": true
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/collectiondetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Collection": [{ "guid": "work", "ctag": "ctag-2" }]
            })))
            .mount(&server)
            .await;
        // The first save is refused: the event moved on at Apple.
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(412))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": occurrence,
                    "pGuid": "work",
                    "title": "Dentist — bring forms",
                    "location": "Room 2",
                    "tz": "Asia/Kolkata",
                    "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
                    "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
                    "etag": "v3"
                }]
            })))
            .mount(&server)
            .await;

        let mut base = event();
        base.guid = occurrence.into();
        base.location = Some("Room 1".into());
        base.etag = Some("v1".into());
        let mut mine = base.clone();
        mine.title = "Dentist — bring forms".into();
        let http = reqwest::Client::new();
        let kept = crate::sync::save_event_merging(
            &http,
            &server.uri(),
            vec![],
            Some("ctag-1"),
            &mine,
            Some(&base),
            None,
            None,
            Some(Scope::This),
            None,
        )
        .await
        .expect("merged and saved, not refused");
        assert_eq!(kept[0].etag.as_deref(), Some("v3"));

        let writes: Vec<Request> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method == reqwest::Method::POST)
            .collect();
        assert_eq!(writes.len(), 2, "sent once more, after the merge");
        let again = &writes[1];
        assert!(
            again.url.path().ends_with(&format!("/{occurrence}/this")),
            "{}",
            again.url.path()
        );
        let body: Value = serde_json::from_slice(&again.body).unwrap();
        assert_eq!(
            body["Event"]["title"], "Dentist — bring forms",
            "this edit's field is this edit's"
        );
        assert_eq!(
            body["Event"]["location"], "Room 2",
            "theirs kept where this edit left it"
        );
        assert_eq!(
            body["Event"]["etag"], "v2",
            "against Apple's newest version"
        );
        assert_eq!(
            body["Event"]["startDate"],
            serde_json::json!([20260914, 2026, 9, 14, 9, 30, 570]),
            "on this occurrence's own day, not the series' first"
        );
        assert_eq!(body["ClientState"]["Collection"][0]["ctag"], "ctag-2");
    }

    /// A create sent again after Apple took the first — the answer lost, the app closed —
    /// merges against itself: Apple's copy goes back as it is, one event, not two.
    #[tokio::test]
    async fn a_create_sent_twice_is_one_event() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": "E-1", "pGuid": "work", "title": "Dentist", "location": "Gachibowli",
                    "tz": "Asia/Kolkata",
                    "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
                    "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
                    "etag": "made"
                }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/collectiondetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "Collection": [{ "guid": "work", "ctag": "c2" }] }),
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(412))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Event": [{
                    "guid": "E-1", "pGuid": "work", "title": "Dentist", "tz": "Asia/Kolkata",
                    "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
                    "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
                    "etag": "same"
                }]
            })))
            .mount(&server)
            .await;
        let made = event();
        let http = reqwest::Client::new();
        crate::sync::save_event_merging(
            &http,
            &server.uri(),
            vec![],
            None,
            &made,
            Some(&made),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let writes: Vec<Request> = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method == reqwest::Method::POST)
            .collect();
        assert_eq!(writes.len(), 2);
        assert!(
            writes.iter().all(|w| w.url.path() == "/ca/events/work/E-1"),
            "one event, by its own id"
        );
        let body: Value = serde_json::from_slice(&writes[1].body).unwrap();
        assert_eq!(
            body["Event"]["etag"], "made",
            "written over Apple's copy as an edit"
        );
        assert_eq!(body["Event"]["title"], "Dentist");
    }

    /// Read through its series, an occurrence keeps its own day; found as itself, it is itself.
    #[test]
    fn an_occurrence_is_found_as_itself_or_through_its_series() {
        let mut known = event();
        known.guid = "S__20260914T040000Z".into();
        let mut series = event();
        series.guid = "S".into();
        series.starts_at -= 13 * 86_400;
        series.ends_at -= 13 * 86_400;
        series.title = "Theirs".into();
        let via_series = theirs_for(&known, vec![series.clone()]).unwrap();
        assert_eq!(via_series.guid, known.guid);
        assert_eq!(via_series.starts_at, known.starts_at);
        assert_eq!(via_series.title, "Theirs");
        let mut itself = series.clone();
        itself.guid.clone_from(&known.guid);
        itself.starts_at = known.starts_at + 60;
        assert_eq!(
            theirs_for(&known, vec![series, itself.clone()]),
            Some(itself)
        );
        let mut other = event();
        other.guid = "OTHER".into();
        assert_eq!(
            theirs_for(&known, vec![other]),
            None,
            "another event is not it"
        );
    }

    /// "All future events" goes to the occurrence's `/future` path with its own dates —
    /// measured live, Apple splits the series there and the days before keep their name.
    #[tokio::test]
    async fn from_this_occurrence_on_goes_to_its_future_path() {
        let server = server().await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let mut one = event();
        one.guid = "SERIES__20261102T073300Z".into();
        client
            .put_event_full(&one, None, None, None, Some(Scope::Future))
            .await
            .unwrap();
        let sent = request(&server).await;
        assert!(
            sent.url
                .path()
                .ends_with("/SERIES__20261102T073300Z/future"),
            "{}",
            sent.url.path()
        );
        let body: Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(body["Event"]["changeRecurring"], "future");
        assert!(body["Event"].get("recurrenceException").is_none());
    }

    /// Apple failing to say what an event holds is not "a new event": building the save
    /// from nothing would write empty recurrence, alerts and guests over the real one.
    #[tokio::test]
    async fn an_edit_is_refused_when_apples_record_cannot_be_read() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        assert!(client.put_event(&event(), None).await.is_err());
        let wrote = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method == reqwest::Method::POST);
        assert!(
            !wrote,
            "nothing may be written over an event that could not be read"
        );
    }

    /// Deleting an event Apple no longer has: it answers 404, and its own record confirms
    /// there is nothing there, so the delete has done what was asked.
    #[tokio::test]
    async fn deleting_what_apple_no_longer_has_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(404).set_body_json(
                serde_json::json!({ "ChangeSet": { "fullState": false, "UpcomingAlarm": [] } }),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        client.delete_event(&event(), None).await.unwrap();
    }

    /// But a 404 for an event Apple can still show is not passed off as a delete.
    #[tokio::test]
    async fn a_404_for_an_event_still_there_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "Event": [{ "guid": "E-1", "title": "Dentist" }] }),
            ))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        assert!(client.delete_event(&event(), None).await.is_err());
    }

    /// An all-day event is a day, not an instant: the 5th is the 5th here, whatever this
    /// machine's zone. Run under `TZ=America/New_York` it fell on the 4th when read as UTC.
    #[test]
    fn an_all_day_event_is_its_own_day_wherever_it_is_read() {
        let body = serde_json::json!({ "Event": [{
            "guid": "A", "pGuid": "work", "title": "Holiday", "allDay": true,
            "startDate": [20261005, 2026, 10, 5, 0, 0, 0],
            "endDate": [20261006, 2026, 10, 6, 0, 0, 0] }] });
        let event = &parse_snapshot(&body).events[0];
        let here = TimeZone::system();
        let day = |t: i64| {
            jiff::Timestamp::from_second(t)
                .unwrap()
                .to_zoned(here.clone())
        };
        let (start, end) = (day(event.starts_at), day(event.ends_at));
        assert_eq!(
            (start.month(), start.day(), start.hour()),
            (10, 5, 0),
            "{start}"
        );
        assert_eq!((end.month(), end.day(), end.hour()), (10, 6, 0), "{end}");
        // And written back, it says the same day.
        let sent = format_apple_date(event.starts_at, dates_zone(true, None).as_deref());
        assert_eq!(sent[0], 20261005);
    }

    #[tokio::test]
    async fn a_rejection_reads_as_a_sentence() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/.*"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad event"))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, &server.uri(), vec![]);
        let message = client
            .put_event(&event(), None)
            .await
            .unwrap_err()
            .to_string();
        // A sentence saying what happened, not the request and Apple's body.
        assert_eq!(
            message,
            "iCloud didn't accept that change, so nothing was saved."
        );
    }
}

/// Calendars this account does not have, but other people's do.
///
/// This account has four calendars, all writable, none subscribed. A subscribed holiday feed,
/// a calendar shared for reading only, and a reminder list surfaced through the same endpoint
/// are all ordinary things to have and none of them appear here to be learned from — so they
/// are checked against the shapes Apple documents in the fields it does send.
#[cfg(test)]
mod other_calendars_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_subscribed_or_shared_calendar_is_kept_but_not_written_to() {
        let feed = collection_from(&json!({
            "guid": "holidays", "title": "Holidays in India", "readOnly": true,
            "supportedType": "Event"
        }))
        .unwrap();
        assert!(!feed.writable, "nothing may be put in a feed");
        assert_eq!(feed.title, "Holidays in India", "but it is still shown");
    }

    /// Absent means writable — most calendars never say so.
    #[test]
    fn a_calendar_that_says_nothing_is_writable() {
        let plain = collection_from(&json!({ "guid": "c1", "title": "Work" })).unwrap();
        assert!(plain.writable);
        assert!(!plain.is_default);
        assert!(plain.enabled);
    }

    /// Apple names the default outright, and it is not always the first.
    #[test]
    fn the_default_is_the_one_apple_says_it_is() {
        let collections = json!([
            { "guid": "a", "title": "Work", "order": 0 },
            { "guid": "b", "title": "Home", "order": 1, "isDefault": true },
        ]);
        let parsed: Vec<Calendar> = collections
            .as_array()
            .unwrap()
            .iter()
            .filter_map(collection_from)
            .collect();
        assert_eq!(parsed.iter().filter(|c| c.is_default).count(), 1);
        assert_eq!(parsed.iter().find(|c| c.is_default).unwrap().guid, "b");
    }

    /// Apple surfaces reminder lists through the calendar endpoint. Shown as calendars they
    /// are empty ones nobody can put an event in.
    #[test]
    fn a_collection_that_does_not_hold_events_is_not_a_calendar() {
        assert!(collection_from(&json!({
            "guid": "r1", "title": "Groceries", "supportedType": "Reminder"
        }))
        .is_none());
        assert!(collection_from(&json!({
            "guid": "c1", "title": "Work", "supportedType": "Event"
        }))
        .is_some());
        assert!(
            collection_from(&json!({ "guid": "c2", "title": "Work" })).is_some(),
            "saying nothing means events"
        );
    }

    /// An account with nothing but a subscribed feed has nowhere to put an event, and the
    /// interface has to be able to tell.
    #[test]
    fn an_account_of_only_feeds_offers_nowhere_to_write() {
        let parsed: Vec<Calendar> = json!([
            { "guid": "a", "title": "Holidays", "readOnly": true },
            { "guid": "b", "title": "Fixtures", "readOnly": true },
        ])
        .as_array()
        .unwrap()
        .iter()
        .filter_map(collection_from)
        .collect();
        assert_eq!(parsed.len(), 2);
        assert!(parsed.iter().all(|c| !c.writable));
    }
}

/// Clocks that move, which this account's never does.
///
/// Asia/Kolkata has no daylight saving and never has, so every path through these conversions
/// had only ever been exercised on a zone with one fixed offset. Most of the world is not like
/// that, and the two hours a year when a clock jumps are exactly where date code fails: one
/// local time never happens, and another happens twice.
#[cfg(test)]
mod daylight_tests {
    use super::*;
    use serde_json::json;

    fn apple(y: i64, m: i64, d: i64, h: i64, min: i64) -> Value {
        json!([y * 10_000 + m * 100 + d, y, m, d, h, min, h * 60 + min])
    }

    fn read(raw: &Value, zone: &str) -> i64 {
        parse_apple_date(raw, Some(zone)).unwrap_or_else(|e| panic!("{zone}: {e}"))
    }

    /// In New York on 8 March 2026 the clock goes straight from 01:59 to 03:00, so 02:30 is a
    /// local time that never happens. Apple can still send it — a repeating event lands there,
    /// or another device wrote it. It has to become some real instant rather than fail a sync.
    #[test]
    fn a_local_time_that_never_happens_still_becomes_an_instant() {
        let at = read(&apple(2026, 3, 8, 2, 30), "America/New_York");
        let back = jiff::Timestamp::from_second(at)
            .unwrap()
            .to_zoned(TimeZone::get("America/New_York").unwrap());
        assert_eq!(back.date().to_string(), "2026-03-08");
        assert!(
            back.hour() == 3 || back.hour() == 1,
            "it has to land either side of the gap, not inside it: {back}"
        );
    }

    /// And on 1 November the clock goes from 01:59 back to 01:00, so 01:30 happens twice.
    /// Either instant is defensible; silently producing something on another day is not.
    #[test]
    fn a_local_time_that_happens_twice_picks_one_and_stays_on_the_day() {
        let at = read(&apple(2026, 11, 1, 1, 30), "America/New_York");
        let back = jiff::Timestamp::from_second(at)
            .unwrap()
            .to_zoned(TimeZone::get("America/New_York").unwrap());
        assert_eq!(back.date().to_string(), "2026-11-01");
        assert_eq!((back.hour(), back.minute()), (1, 30));
    }

    /// Nine in the morning is nine in the morning on both sides of a clock change. An event
    /// written on one side and read on the other must not drift an hour.
    #[test]
    fn an_ordinary_morning_survives_the_round_trip_on_both_sides_of_a_change() {
        for zone in [
            "America/New_York",
            "Europe/London",
            "Australia/Sydney",
            "Australia/Lord_Howe",
            "Asia/Kolkata",
            "Pacific/Chatham",
        ] {
            for (m, d) in [(1, 15), (3, 20), (7, 4), (11, 15)] {
                let sent = apple(2026, m, d, 9, 0);
                let at = read(&sent, zone);
                let written = format_apple_date(at, Some(zone));
                assert_eq!(
                    written, sent,
                    "{zone} on {m}/{d}: what was read did not come back the same"
                );
            }
        }
    }

    /// Half-hour and three-quarter-hour offsets are real places, and Lord Howe's clock moves
    /// by thirty minutes rather than an hour.
    #[test]
    fn zones_that_are_not_whole_hours_round_trip_too() {
        for zone in ["Asia/Kathmandu", "Australia/Eucla", "Australia/Lord_Howe"] {
            let sent = apple(2026, 6, 10, 14, 45);
            assert_eq!(
                format_apple_date(read(&sent, zone), Some(zone)),
                sent,
                "{zone}"
            );
        }
    }

    /// A zone this machine has never heard of must keep the event visible rather than fail the
    /// whole sync — someone's calendar carrying one bad zone is not a reason to show them
    /// nothing.
    #[test]
    fn an_unknown_zone_falls_back_instead_of_failing_the_sync() {
        assert!(parse_apple_date(&apple(2026, 6, 10, 9, 0), Some("Mars/Olympus_Mons")).is_ok());
        assert!(parse_apple_date(&apple(2026, 6, 10, 9, 0), None).is_ok());
    }
}

#[cfg(test)]
mod address_tests {
    use super::*;

    fn event(calendar: &str, guid: &str) -> quince_core::calendar::Event {
        quince_core::calendar::Event {
            guid: guid.into(),
            calendar_guid: calendar.into(),
            title: "Standup".into(),
            location: None,
            starts_at: 1_789_000_000,
            ends_at: 1_789_003_600,
            all_day: false,
            timezone: Some("UTC".into()),
            url: None,
            notes: None,
            etag: None,
        }
    }

    /// Both halves of the path are ids, and an empty one does not fail loudly — it builds a
    /// different address, which Apple answers with a bare 400 and no reason at all.
    #[tokio::test]
    async fn an_event_with_no_identifier_is_refused_before_anything_is_sent() {
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, "http://127.0.0.1:1", vec![]);
        for (calendar, guid, expect) in [
            ("CAL", "", "no identifier"),
            ("CAL", "   ", "no identifier"),
            ("", "EVENT", "has none yet"),
        ] {
            let err = client
                .put_event(&event(calendar, guid), None)
                .await
                .expect_err("nothing addressable, so nothing to send");
            assert!(
                err.to_string().contains(expect),
                "wanted {expect:?}, got: {err}"
            );
        }
    }

    /// Deleting has the same two halves.
    #[tokio::test]
    async fn deleting_an_event_with_no_identifier_is_refused_too() {
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, "http://127.0.0.1:1", vec![]);
        let err = client
            .delete_event(&event("CAL", ""), None)
            .await
            .expect_err("nothing addressable");
        assert!(err.to_string().contains("no identifier"), "got: {err}");
    }
}

#[cfg(test)]
mod odd_event_tests {
    use super::*;

    /// The event Apple sent is where a write starts. If it is not an object, the write must
    /// still produce one rather than panic on network input.
    #[test]
    fn an_event_that_is_not_an_object_is_set_aside_rather_than_panicking() {
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, "http://127.0.0.1:1", vec![]);
        let event = quince_core::calendar::Event {
            guid: "E1".into(),
            calendar_guid: "C1".into(),
            title: "Standup".into(),
            location: None,
            starts_at: 1_789_000_000,
            ends_at: 1_789_003_600,
            all_day: false,
            timezone: Some("UTC".into()),
            url: None,
            notes: None,
            etag: None,
        };
        for odd in [
            Value::Null,
            serde_json::json!([1]),
            serde_json::json!("text"),
        ] {
            let body = client.event_body(&event, Some(&odd));
            assert_eq!(body["title"], "Standup", "{odd} still produces the event");
        }
    }

    /// A title edited here, on an event made on a Mac with everything this app does not show:
    /// its alerts, guests, attachments, the repeat and its exceptions, a private comment, an
    /// organiser, and fields Apple adds that nobody here knows. Every one goes back to Apple
    /// exactly as Apple sent it; only the title — and the fields this app writes, with the
    /// same values — differ.
    #[test]
    fn an_edit_keeps_everything_this_app_does_not_show() {
        let http = reqwest::Client::new();
        let client = CalendarClient::new(&http, "http://127.0.0.1:1", vec![]);
        let existing = serde_json::json!({
            "guid": "E1", "pGuid": "C1", "title": "Planning", "location": "Room 4",
            "description": "Agenda", "url": null, "tz": "Europe/Berlin", "allDay": false,
            "startDate": [20261105, 2026, 11, 5, 10, 0, 600],
            "endDate": [20261105, 2026, 11, 5, 11, 0, 660],
            "localStartDate": [20261105, 2026, 11, 5, 10, 0, 600],
            "localEndDate": [20261105, 2026, 11, 5, 11, 0, 660],
            "duration": 60, "etag": "C=9@U=x",
            "alarms": ["E1:A1", "E1:A2"],
            "invitees": ["E1:organiser@example.com", "E1:guest@example.com"],
            "organizer": "organiser@example.com",
            "hasAttachments": true, "attachments": ["E1:ATT1"],
            "recurrence": "E1*MME-RID",
            "recurrenceMaster": true,
            "recurrenceException": ["20261112T090000Z"],
            "privateComments": "bring the numbers",
            "changeRecurring": null,
            "birthdayIsYearlessBday": false,
            "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR": "AUTOMATIC",
            "someFieldAppleAddsLater": { "nested": [1, 2, 3] },
        });
        let event = event_from(&existing).unwrap().unwrap();
        let mut renamed = event.clone();
        renamed.title = "Planning, round 2".into();
        let body = client.event_body(&renamed, Some(&existing));
        let (Some(body), Some(was)) = (body.as_object(), existing.as_object()) else {
            panic!("an event is an object");
        };
        for (key, value) in was {
            if key == "title" {
                continue;
            }
            assert_eq!(body.get(key), Some(value), "{key} changed or went missing");
        }
        assert_eq!(body["title"], "Planning, round 2");
        assert_eq!(
            body.len(),
            was.len(),
            "nothing added beyond what Apple holds"
        );
    }

    /// Apple's repeat object, both ways, in the shape Apple itself returned for a test event
    /// (10 Nov 2026): every two weeks on Tuesday and Thursday, until a day. Written, it reads
    /// back as the same repeat; an end day is the day's last minute, as Apple stores it.
    #[test]
    fn a_repeat_on_weekdays_until_a_day_is_written_as_apple_holds_it() {
        let apple = serde_json::json!({
            "freq": "weekly", "interval": 2, "weekStart": "MO",
            "weekDays": ["TU", "TH"],
            "until": [20261215, 2026, 12, 15, 23, 59, 1439],
            "localizedRecurrenceRuleDisplayString": "Repeats every 2 weeks on Tuesday and Thursday",
        });
        let read = repeat_from_apple(&apple).unwrap();
        assert_eq!(
            read,
            EventRepeat {
                freq: "weekly".into(),
                interval: 2,
                count: None,
                week_days: vec!["TU".into(), "TH".into()],
                until: Some("2026-12-15".into()),
                ..EventRepeat::default()
            }
        );
        let mut written = serde_json::json!({ "freq": "weekly", "interval": 2 });
        write_repeat(&mut written, &read);
        assert_eq!(written["weekDays"], apple["weekDays"]);
        assert_eq!(written["until"], apple["until"]);
        assert!(
            written.get("count").is_none(),
            "an end day, never a count beside it"
        );
        // Each other form, in the shape Apple returned for it: written, it is Apple's own.
        for apple in [
            serde_json::json!({ "freq": "monthly", "interval": 1, "frequencyDays": "2nd", "weekDays": ["TU"] }),
            serde_json::json!({ "freq": "monthly", "interval": 1, "byDay": [1, 15] }),
            serde_json::json!({ "freq": "yearly", "interval": 1, "byMonth": [11], "frequencyDays": "4th", "weekDays": ["TH"] }),
        ] {
            let read = repeat_from_apple(&apple).unwrap();
            let mut written = serde_json::json!({ "freq": read.freq, "interval": read.interval });
            write_repeat(&mut written, &read);
            assert_eq!(written, apple);
        }
        // A plain repeat writes neither.
        let mut plain = serde_json::json!({});
        write_repeat(
            &mut plain,
            &EventRepeat {
                freq: "monthly".into(),
                interval: 1,
                count: Some(3),
                ..EventRepeat::default()
            },
        );
        assert_eq!(plain, serde_json::json!({ "count": 3 }));
    }
}
