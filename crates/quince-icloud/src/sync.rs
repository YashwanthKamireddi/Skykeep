//! Bringing Apple's calendar into the local library.
//!
//! The interface never waits on this. A sync reconciles into SQLite and the UI reads SQLite,
//! so a slow or failed refresh leaves the last known good data on screen rather than an
//! empty window.

use crate::calendar::{CalendarClient, Snapshot};
use crate::{Error, Result};
use jiff::civil::Date;
use quince_core::Library;

/// Apple rejects a calendar window wider than about a month with a bare 400, so requests are
/// split into chunks below that. Measured against the live service: one month is accepted,
/// two months is not.
const MAX_WINDOW_DAYS: i32 = 28;

/// How many of those windows are asked about at once: enough that a refresh is a couple of
/// round trips rather than a dozen, few enough that Apple is not flooded.
const CALENDAR_WINDOWS_AT_ONCE: usize = 4;

fn parse_date(value: &str) -> Result<Date> {
    value
        .parse::<Date>()
        .map_err(|e| Error::Protocol(format!("{value} is not a date: {e}")))
}

/// Split `[from, to]` into windows Apple will accept, as inclusive `YYYY-MM-DD` pairs.
fn chunks(from: Date, to: Date) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut start = from;
    while start <= to {
        // `- 1` keeps the window inclusive, so chunks meet without overlapping.
        let end = start
            .checked_add(jiff::Span::new().days(MAX_WINDOW_DAYS - 1))
            .unwrap_or(to)
            .min(to);
        out.push((start.to_string(), end.to_string()));
        let Ok(next) = end.checked_add(jiff::Span::new().days(1)) else {
            break;
        };
        start = next;
    }
    out
}

/// What one refresh changed.
#[derive(Debug, Default, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub calendars: usize,
    pub events: usize,
}

/// Fetch the window `[from, to]` (both `YYYY-MM-DD`) and reconcile it into the library.
///
/// Collections are written before events so the foreign key always resolves; an event whose
/// calendar Apple did not report would otherwise be rejected.
pub async fn calendar(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut Library,
    from: &str,
    to: &str,
    timezone: &str,
) -> Result<SyncSummary> {
    let snapshot = fetch_calendar(http, root, params, from, to, timezone).await?;
    apply_calendar(library, snapshot)
}

/// Ask Apple for a window of the calendar. Touches no local state, so the slow half runs
/// without holding the library every screen reads from.
pub async fn fetch_calendar(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    from: &str,
    to: &str,
    timezone: &str,
) -> Result<Snapshot> {
    let client = CalendarClient::new(http, root, params);
    let windows = chunks(parse_date(from)?, parse_date(to)?);

    let mut calendars = Vec::new();
    let mut events = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // `/ca/startup` reliably carries the collections but not always the events — on some
    // accounts it returns collections alone. Asking `/ca/events` for the same window is what
    // guarantees them, so do both and merge.
    //
    // Every window's two questions are asked together, a few windows at a time, rather than
    // one after another: five months was a dozen round trips in a row on every change Apple
    // reported. The answers are taken in the windows' order, so what is kept from two that
    // overlap is what it always was.
    use futures_util::StreamExt;
    let client = &client;
    let mut answers = futures_util::stream::iter(windows)
        .map(|(start, end)| async move {
            // A window whose connection stalls is asked again, twice, before the read fails:
            // one stalled request out of thirty failed a whole year's read (seen on Android).
            // Only a connection that failed; Apple refusing is an answer.
            let mut tries = 0u64;
            loop {
                let answer = futures_util::future::try_join(
                    client.startup(&start, &end, timezone),
                    client.events(&start, &end, timezone),
                )
                .await;
                match answer {
                    Err(Error::Network(_)) if tries < 2 => {
                        tries += 1;
                        tokio::time::sleep(std::time::Duration::from_millis(500 * tries)).await;
                    }
                    other => break other,
                }
            }
        })
        .buffered(CALENDAR_WINDOWS_AT_ONCE);
    while let Some(answer) = answers.next().await {
        let (startup, windowed) = answer?;
        if calendars.is_empty() {
            calendars = startup.calendars;
        }
        for event in startup.events.into_iter().chain(windowed.events) {
            if seen.insert(event.guid.clone()) {
                events.push(event);
            }
        }
    }

    // The whole window was asked about, day by day in the viewer's zone, so anything stored
    // as starting inside it that did not come back is no longer in iCloud.
    let zone = jiff::tz::TimeZone::get(timezone).unwrap_or_else(|_| jiff::tz::TimeZone::system());
    let edge = |d: Date| {
        d.to_zoned(zone.clone())
            .map(|z| z.timestamp().as_second())
            .map_err(|e| Error::Protocol(format!("{d} has no start in {timezone}: {e}")))
    };
    // Every iCloud account has a calendar. An answer with none is a broken answer, not an
    // account emptied: refused here, in the one fetch both apps use, so neither stores it (the
    // phone deleted every calendar and their events, unsent edits included, from one such
    // answer; the desktop's `apply_calendar` refuses it too).
    if calendars.is_empty() {
        return Err(Error::Protocol(
            "Apple's calendar answer carried no calendars, so nothing was changed.".into(),
        ));
    }
    let first = parse_date(from)?;
    let after_last = parse_date(to)?
        .checked_add(jiff::Span::new().days(1))
        .map_err(|e| Error::Protocol(format!("{to} has no next day: {e}")))?;
    Ok(Snapshot {
        calendars,
        events,
        covers: Some((edge(first)?, edge(after_last)?)),
    })
}

/// The change tag Apple last gave an event's calendar, which a write carries.
#[must_use]
pub fn calendar_ctag(library: &Library, calendar_guid: &str) -> Option<String> {
    library
        .calendars()
        .ok()?
        .into_iter()
        .find(|c| c.guid == calendar_guid)
        .and_then(|c| c.ctag)
}

/// Create an event, or save changes to one, and store what Apple answers with.
///
/// Apple takes the whole event on every write, so an edit is the event as it should end up
/// rather than a patch. The response carries the stored version back, which is what goes
/// into the library — never the version that was sent.
pub async fn put_event(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut Library,
    event: &quince_core::calendar::Event,
) -> Result<()> {
    let ctag = calendar_ctag(library, &event.calendar_guid);
    let kept = save_event(http, root, params, ctag.as_deref(), event).await?;
    store_events(library, kept)
}

/// The network half of [`put_event`]: write the event, and return Apple's own copy of it.
///
/// Kept apart so a caller can do it without holding the library — everything on screen
/// reads that library, and a save that held it while Apple was slow froze every screen.
pub async fn save_event(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ctag: Option<&str>,
    event: &quince_core::calendar::Event,
) -> Result<Vec<quince_core::calendar::Event>> {
    save_event_with_alerts(http, root, params, ctag, event, None, None, None, None).await
}

/// [`save_event`], with the event's alerts set to `alerts` when given.
#[allow(clippy::too_many_arguments)]
pub async fn save_event_with_alerts(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ctag: Option<&str>,
    event: &quince_core::calendar::Event,
    alerts: Option<&[i64]>,
    repeat: Option<Option<&crate::calendar::EventRepeat>>,
    scope: Option<crate::calendar::Scope>,
    from: Option<(&str, Option<&str>)>,
) -> Result<Vec<quince_core::calendar::Event>> {
    let client = CalendarClient::new(http, root, params);
    let answer = client
        .put_event_moved(event, ctag, alerts, repeat, scope, from)
        .await?;
    // What goes into the library is what Apple holds, never what was sent. Measured: a save
    // to the account answers with an empty list, and storing the sent copy instead kept an
    // event with no etag — so the next edit or delete went without one — and, where Apple
    // had not kept it, a ghost that could never be deleted. So an empty answer is followed
    // by asking for the event's day and taking Apple's own copy.
    if answer.events.iter().any(|e| e.guid == event.guid) {
        return Ok(answer.events);
    }
    let zone = crate::calendar::event_zone(event);
    let day = crate::calendar::day_of(event.starts_at, zone.as_deref());
    let tz = zone.as_deref().unwrap_or("UTC");
    // A repeat set or cleared, or a series split, gives the day's occurrence another id than
    // the one sent (`S` and `S__<start>` are one series): found by its series, not its id.
    let series = event
        .guid
        .split("__")
        .next()
        .unwrap_or(&event.guid)
        .to_string();
    let reshaped = repeat.is_some() || scope.is_some() || series != event.guid;
    let kept: Vec<_> = client
        .events(&day, &day, tz)
        .await?
        .events
        .into_iter()
        .filter(|e| {
            e.guid == event.guid
                || (reshaped && (e.guid == series || e.guid.starts_with(&format!("{series}__"))))
        })
        .collect();
    // A series split ("this and future") lands under a new id Apple chooses: saved all the same,
    // and the next pass brings it.
    if kept.is_empty() && reshaped {
        return Ok(kept);
    }
    if kept.is_empty() {
        return Err(Error::Protocol(
            "iCloud took the event but did not keep it — nothing was saved".into(),
        ));
    }
    Ok(kept)
}

/// [`save_event_with_alerts`], and an edit refused because the event changed on another device
/// since it was read settled as Calendar settles it: Apple's copy is read again and this edit
/// laid over it — their changes to other fields kept, this edit's own fields this edit's — and
/// saved once more against Apple's newest version, with each calendar's tag read afresh (the
/// refusal may have been for the calendar's tag as much as the event's).
///
/// `base` is the event as the edit began from it; for a create sent again it is the event as
/// made, so a create Apple already holds merges to no change. One occurrence of a repeating
/// event is merged the same way: Apple's copy of that day ([`crate::calendar::theirs_for`]),
/// saved with the scope it was asked with — which days it reaches stays Apple's to settle.
/// Gone at Apple, it is refused: never written back.
#[allow(clippy::too_many_arguments)]
pub async fn save_event_merging(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ctag: Option<&str>,
    event: &quince_core::calendar::Event,
    base: Option<&quince_core::calendar::Event>,
    alerts: Option<&[i64]>,
    repeat: Option<Option<&crate::calendar::EventRepeat>>,
    scope: Option<crate::calendar::Scope>,
    from: Option<(&str, Option<&str>)>,
) -> Result<Vec<quince_core::calendar::Event>> {
    let first = save_event_with_alerts(
        http,
        root,
        params.clone(),
        ctag,
        event,
        alerts,
        repeat,
        scope,
        from,
    )
    .await;
    let (Err(Error::Conflict), Some(base)) = (&first, base) else {
        return first;
    };
    let client = CalendarClient::new(http, root, params.clone());
    // Looked for where it was: a create's base is where it was made.
    let mut known = base.clone();
    if let Some((old, _)) = from {
        old.clone_into(&mut known.calendar_guid);
    }
    let Some(theirs) = client.current_event(&known).await? else {
        return Err(Error::Refused(
            "This event was deleted on another device, so it wasn’t saved.".into(),
        ));
    };
    let merged = crate::calendar::merge_edit(base, event, &theirs);
    let ctag = client
        .current_ctag(&merged.calendar_guid)
        .await
        .ok()
        .flatten();
    let from = if theirs.calendar_guid == merged.calendar_guid {
        None
    } else {
        Some((
            theirs.calendar_guid.clone(),
            client
                .current_ctag(&theirs.calendar_guid)
                .await
                .ok()
                .flatten(),
        ))
    };
    save_event_with_alerts(
        http,
        root,
        params,
        ctag.as_deref(),
        &merged,
        alerts,
        repeat,
        scope,
        from.as_ref()
            .map(|(old, tag)| (old.as_str(), tag.as_deref())),
    )
    .await
}

/// The library half of [`put_event`]: store what Apple kept, in calendars this library has.
pub fn store_events(library: &mut Library, kept: Vec<quince_core::calendar::Event>) -> Result<()> {
    let known: std::collections::HashSet<String> =
        library.calendars()?.into_iter().map(|c| c.guid).collect();
    let storable: Vec<_> = kept
        .into_iter()
        .filter(|e| known.contains(&e.calendar_guid))
        .collect();
    library.put_events(&storable)?;
    Ok(())
}

/// Remove an event, locally as well as at Apple.
pub async fn delete_event(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut Library,
    event: &quince_core::calendar::Event,
) -> Result<()> {
    let ctag = calendar_ctag(library, &event.calendar_guid);
    CalendarClient::new(http, root, params)
        .delete_event(event, ctag.as_deref())
        .await?;
    library.forget_events(std::slice::from_ref(&event.guid))?;
    Ok(())
}

/// Store a fetched window. Touches no network.
pub fn apply_calendar(library: &mut Library, snapshot: Snapshot) -> Result<SyncSummary> {
    // Every iCloud account has a calendar. An answer with none is a broken answer, not an
    // account emptied: stored, it deleted every calendar here and every event with them.
    if snapshot.calendars.is_empty() {
        return Err(crate::Error::Protocol(
            "Apple's calendar answer carried no calendars, so nothing was changed.".into(),
        ));
    }
    library.put_calendars(&snapshot.calendars)?;

    // Drop events belonging to calendars Apple did not report: storing them would fail the
    // foreign key, and they are not ours to show.
    let known: std::collections::HashSet<&str> =
        snapshot.calendars.iter().map(|c| c.guid.as_str()).collect();
    let events: Vec<_> = snapshot
        .events
        .iter()
        .filter(|e| known.contains(e.calendar_guid.as_str()))
        .cloned()
        .collect();

    library.put_events(&events)?;
    // An event deleted on the phone stayed here for good — a pass only ever added — and
    // deleting that ghost here then failed, because Apple had nothing to delete. Within the
    // window Apple was asked about, what it did not return is gone; outside it, nothing is
    // assumed. Only calendars Apple reported are judged.
    if let Some((from, to)) = snapshot.covers {
        let returned: std::collections::HashSet<&str> =
            events.iter().map(|e| e.guid.as_str()).collect();
        let gone: Vec<String> = library
            .events_starting_between(from, to)?
            .into_iter()
            .filter(|(guid, calendar)| {
                known.contains(calendar.as_str()) && !returned.contains(guid.as_str())
            })
            .map(|(guid, _)| guid)
            .collect();
        library.forget_events(&gone)?;
    }
    library.mark_synced("calendar", None)?;

    Ok(SyncSummary {
        calendars: snapshot.calendars.len(),
        events: events.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn body() -> serde_json::Value {
        json!({
            "Collection": [{ "guid": "cal-1", "title": "Work", "ctag": "c1" }],
            "Event": [
                { "guid": "ev-1", "pGuid": "cal-1", "title": "Standup", "tz": "Asia/Kolkata",
                  "startDate": [20260912, 2026, 9, 12, 9, 30, 570],
                  "endDate":   [20260912, 2026, 9, 12, 9, 45, 585] },
                // Belongs to a calendar Apple did not report — must be dropped, not stored.
                { "guid": "ev-2", "pGuid": "cal-missing", "title": "Orphan",
                  "startDate": [20260912, 2026, 9, 12, 11, 0, 660], "duration": 30 }
            ]
        })
    }

    fn saved_library() -> Library {
        let mut library = Library::in_memory().unwrap();
        library
            .put_calendars(&[quince_core::calendar::Calendar {
                guid: "work".into(),
                title: "Work".into(),
                ..Default::default()
            }])
            .unwrap();
        library
    }

    fn new_event() -> quince_core::calendar::Event {
        let start = "2026-10-03T04:30:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_second();
        quince_core::calendar::Event {
            guid: "NEW-1".into(),
            calendar_guid: "work".into(),
            title: "Dentist".into(),
            starts_at: start,
            ends_at: start + 3600,
            timezone: Some("Asia/Kolkata".into()),
            ..Default::default()
        }
    }

    /// Five months is six windows of two questions each. Asked side by side a few windows at
    /// a time, the whole takes a couple of Apple's answers, not twelve in a row — and gives
    /// the same calendars and events, each once.
    #[tokio::test]
    async fn the_windows_are_asked_about_together() {
        let server = MockServer::start().await;
        let slow = std::time::Duration::from_millis(150);
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(body())
                    .set_delay(slow),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "Event": body()["Event"].clone() }))
                    .set_delay(slow),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let started = std::time::Instant::now();
        let snapshot = fetch_calendar(
            &http,
            &server.uri(),
            vec![],
            "2026-08-01",
            "2026-12-31",
            "Asia/Kolkata",
        )
        .await
        .unwrap();
        let took = started.elapsed();
        let asked = server.received_requests().await.unwrap().len();
        assert_eq!(
            asked,
            2 * chunks(
                parse_date("2026-08-01").unwrap(),
                parse_date("2026-12-31").unwrap()
            )
            .len()
        );
        assert!(
            took < slow * 6,
            "{asked} questions answered {slow:?} apart took {took:?}: they waited on each other"
        );
        assert_eq!(snapshot.calendars.len(), 1);
        let guids: Vec<&str> = snapshot.events.iter().map(|e| e.guid.as_str()).collect();
        assert_eq!(
            guids,
            ["ev-1", "ev-2"],
            "each event once, in the order first seen"
        );
    }

    /// An answer with no calendars is refused by the fetch itself, so no app stores it: the
    /// phone, which doesn't go through `apply_calendar`, deleted every calendar from one.
    #[tokio::test]
    async fn an_answer_without_calendars_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "Collection": [], "Event": [] })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Event": [] })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let got = fetch_calendar(
            &http,
            &server.uri(),
            vec![],
            "2026-08-01",
            "2026-08-31",
            "UTC",
        )
        .await;
        assert!(
            matches!(got, Err(Error::Protocol(_))),
            "{:?}",
            got.map(|s| s.calendars.len())
        );
    }

    /// A window whose connection stalls is asked again, not the whole read failed: the first
    /// answer for it never comes in time, the second does. (On Android one stalled request out
    /// of thirty failed a year's read.)
    #[tokio::test]
    async fn a_stalled_window_is_asked_again() {
        let server = MockServer::start().await;
        let stall = CalendarClient::READ_TIMEOUT * 3;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(body())
                    .set_delay(stall),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "Event": body()["Event"].clone() })),
            )
            .mount(&server)
            .await;
        let snapshot = fetch_calendar(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "2026-11-01",
            "2026-11-10",
            "UTC",
        )
        .await
        .expect("asked again, the window answers");
        assert_eq!(snapshot.events.len(), 2);
    }

    /// A save answers with nothing (measured on the account); what is stored is Apple's own
    /// copy, read back, with the etag the next edit or delete needs.
    #[tokio::test]
    async fn a_saved_event_is_stored_as_apple_keeps_it() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Event": [{ "guid": "NEW-1", "pGuid": "work", "title": "Dentist", "etag": "apple-1",
                            "tz": "Asia/Kolkata",
                            "startDate": [20261003, 2026, 10, 3, 10, 0, 600], "duration": 60 }]
            })))
            .mount(&server)
            .await;
        let mut library = saved_library();
        let http = reqwest::Client::new();
        put_event(&http, &server.uri(), vec![], &mut library, &new_event())
            .await
            .unwrap();
        assert_eq!(
            library.event("NEW-1").unwrap().unwrap().etag.as_deref(),
            Some("apple-1")
        );
    }

    /// And where Apple did not keep it, nothing is stored — no ghost that cannot be deleted
    /// — and the person is told.
    #[tokio::test]
    async fn an_event_apple_did_not_keep_is_not_stored() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Event": [] })))
            .mount(&server)
            .await;
        let mut library = saved_library();
        let http = reqwest::Client::new();
        assert!(
            put_event(&http, &server.uri(), vec![], &mut library, &new_event())
                .await
                .is_err()
        );
        assert!(library.event("NEW-1").unwrap().is_none());
    }

    /// An all-day event Apple keeps with no zone, moved to the 6th: the save asks for the 6th
    /// here and finds it there. Looked up by its UTC date it was searched for on the 5th in
    /// India, not found, and the move — which Apple had kept — reported as not saved.
    #[tokio::test]
    async fn a_moved_all_day_event_is_found_on_its_own_day() {
        use wiremock::matchers::query_param;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .and(query_param("startDate", "2026-10-06"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Event": [{ "guid": "DAY-1", "pGuid": "work", "title": "Holiday", "allDay": true,
                            "etag": "apple-2",
                            "startDate": [20261006, 2026, 10, 6, 0, 0, 0],
                            "endDate": [20261007, 2026, 10, 7, 0, 0, 0] }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Event": [] })))
            .mount(&server)
            .await;
        let here = jiff::tz::TimeZone::system();
        let midnight = |d: &str| {
            d.parse::<jiff::civil::Date>()
                .unwrap()
                .to_zoned(here.clone())
                .unwrap()
                .timestamp()
                .as_second()
        };
        let moved = quince_core::calendar::Event {
            guid: "DAY-1".into(),
            calendar_guid: "work".into(),
            title: "Holiday".into(),
            starts_at: midnight("2026-10-06"),
            ends_at: midnight("2026-10-07"),
            all_day: true,
            timezone: None,
            ..Default::default()
        };
        let mut library = saved_library();
        let http = reqwest::Client::new();
        put_event(&http, &server.uri(), vec![], &mut library, &moved)
            .await
            .unwrap();
        assert_eq!(
            library.event("DAY-1").unwrap().unwrap().etag.as_deref(),
            Some("apple-2")
        );
    }

    /// An event deleted on the phone leaves this library at the next pass over its window —
    /// the ghost of one is what a delete here then failed on — while an event outside the
    /// window, which Apple was not asked about, is left alone.
    #[tokio::test]
    async fn an_event_deleted_elsewhere_leaves_but_one_outside_the_window_stays() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Collection": [{ "guid": "cal-1", "title": "Work" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "Event": [] })))
            .mount(&server)
            .await;
        let mut library = Library::in_memory().unwrap();
        library
            .put_calendars(&[quince_core::calendar::Calendar {
                guid: "cal-1".into(),
                title: "Work".into(),
                ..Default::default()
            }])
            .unwrap();
        let at = |s: &str| s.parse::<jiff::Timestamp>().unwrap().as_second();
        let ev = |guid: &str, start: i64| quince_core::calendar::Event {
            guid: guid.into(),
            calendar_guid: "cal-1".into(),
            title: guid.into(),
            starts_at: start,
            ends_at: start + 3600,
            ..Default::default()
        };
        library
            .put_events(&[
                ev("deleted", at("2026-09-12T09:00:00Z")),
                ev("later", at("2026-11-02T09:00:00Z")),
            ])
            .unwrap();

        let http = reqwest::Client::new();
        calendar(
            &http,
            &server.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "UTC",
        )
        .await
        .unwrap();
        assert!(
            library.event("deleted").unwrap().is_none(),
            "deleted on the phone, gone here"
        );
        assert!(
            library.event("later").unwrap().is_some(),
            "outside the window, untouched"
        );
    }

    /// Some accounts get collections from /ca/startup and nothing else; the events arrive
    /// only from /ca/events. A sync that trusts startup alone silently shows an empty
    /// calendar.
    #[tokio::test]
    async fn events_are_collected_even_when_startup_omits_them() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Collection": [{ "guid": "cal-1", "title": "Work" }]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "Event": [{ "guid": "ev-9", "pGuid": "cal-1", "title": "From events",
                            "startDate": [20260912, 2026, 9, 12, 9, 0, 540], "duration": 30 }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();
        let summary = calendar(
            &http,
            &server.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "UTC",
        )
        .await
        .unwrap();

        assert_eq!(
            summary.events, 1,
            "events must come from /ca/events when startup omits them"
        );
        assert_eq!(
            library.events_between(0, i64::MAX).unwrap()[0].title,
            "From events"
        );
    }

    #[tokio::test]
    async fn a_sync_lands_in_the_library() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();

        let summary = calendar(
            &http,
            &server.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "Asia/Kolkata",
        )
        .await
        .unwrap();

        assert_eq!(summary.calendars, 1);
        assert_eq!(summary.events, 1, "the orphaned event must be dropped");

        let stored = library.events_between(0, i64::MAX).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].title, "Standup");
        assert!(library.last_synced("calendar").unwrap().is_some());
    }

    /// Running twice must converge, not accumulate.
    #[tokio::test]
    async fn syncing_twice_is_idempotent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();
        for _ in 0..3 {
            calendar(
                &http,
                &server.uri(),
                vec![],
                &mut library,
                "2026-09-01",
                "2026-09-30",
                "UTC",
            )
            .await
            .unwrap();
        }
        assert_eq!(library.events_between(0, i64::MAX).unwrap().len(), 1);
        assert_eq!(library.calendars().unwrap().len(), 1);
    }

    /// A failed refresh must leave the last good data alone.
    #[tokio::test]
    async fn a_failed_sync_does_not_empty_the_library() {
        let ok = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body()))
            .mount(&ok)
            .await;
        Mock::given(method("GET"))
            .and(path("/ca/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&ok)
            .await;

        let http = reqwest::Client::new();
        let mut library = Library::in_memory().unwrap();
        calendar(
            &http,
            &ok.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "UTC",
        )
        .await
        .unwrap();

        let broken = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ca/startup"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&broken)
            .await;

        assert!(calendar(
            &http,
            &broken.uri(),
            vec![],
            &mut library,
            "2026-09-01",
            "2026-09-30",
            "UTC"
        )
        .await
        .is_err());
        assert_eq!(
            library.events_between(0, i64::MAX).unwrap().len(),
            1,
            "a failed refresh must not clear what we already had"
        );
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    /// Apple answers 400 for anything much wider than a month, so no chunk may exceed the cap.
    #[test]
    fn no_chunk_exceeds_what_apple_accepts() {
        let windows = chunks(d("2026-01-01"), d("2026-12-31"));
        assert!(!windows.is_empty());
        for (start, end) in &windows {
            let span = d(end) - d(start);
            let days = i64::from(span.get_days());
            assert!(
                days < i64::from(MAX_WINDOW_DAYS),
                "{start}..{end} spans {days} days, over the limit"
            );
        }
    }

    /// Chunks must tile the range exactly: no gap would silently lose events, and no overlap
    /// means no wasted request.
    #[test]
    fn chunks_tile_the_range_without_gaps_or_overlap() {
        let (from, to) = (d("2026-01-01"), d("2026-06-30"));
        let windows = chunks(from, to);

        assert_eq!(windows[0].0, from.to_string());
        assert_eq!(windows.last().unwrap().1, to.to_string());

        for pair in windows.windows(2) {
            let previous_end = d(&pair[0].1);
            let next_start = d(&pair[1].0);
            assert_eq!(
                (next_start - previous_end).get_days(),
                1,
                "chunks must meet exactly, not skip or repeat a day"
            );
        }
    }

    #[test]
    fn a_short_range_is_a_single_window() {
        let windows = chunks(d("2026-03-01"), d("2026-03-10"));
        assert_eq!(
            windows,
            vec![("2026-03-01".to_string(), "2026-03-10".to_string())]
        );
    }

    #[test]
    fn a_single_day_still_produces_a_window() {
        assert_eq!(chunks(d("2026-03-01"), d("2026-03-01")).len(), 1);
    }

    /// A backwards range should produce nothing rather than looping.
    #[test]
    fn an_inverted_range_is_empty() {
        assert!(chunks(d("2026-06-01"), d("2026-01-01")).is_empty());
    }
}

#[cfg(test)]
mod empty_answer_tests {
    /// An answer with no calendars changes nothing: it used to delete every calendar stored
    /// here, and every event with them.
    #[test]
    fn an_answer_with_no_calendars_is_refused() {
        let mut library = quince_core::Library::in_memory().unwrap();
        let empty = crate::calendar::Snapshot::default();
        assert!(super::apply_calendar(&mut library, empty).is_err());
    }
}
