//! Calendar's changes: recorded and laid (the desktop), queued (the phone), and sent against a
//! pretend Apple. Moved here from the desktop's outbox with the code they test.

use super::*;
use crate::queue::drain;
use tokio::sync::Mutex;

fn calendars(lib: &mut Library) {
    use quince_core::calendar::Calendar;
    let c = |g: &str| Calendar {
        guid: g.into(),
        title: g.into(),
        enabled: true,
        writable: true,
        ..Default::default()
    };
    lib.put_calendars(&[c("work"), c("home")]).unwrap();
}

fn event(guid: &str, title: &str, starts_at: i64) -> Event {
    Event {
        guid: guid.into(),
        calendar_guid: "work".into(),
        title: title.into(),
        starts_at,
        ends_at: starts_at + 3600,
        etag: Some("apple".into()),
        ..Default::default()
    }
}

fn kinds(lib: &Library, service: &str) -> Vec<(String, String)> {
    lib.outbox_pending(service)
        .unwrap()
        .into_iter()
        .map(|p| (p.record, p.kind))
        .collect()
}

/// Calendar: a new event shows at once under the id it will have; edits before it is sent
/// stay one create, from nothing; an edit on its way is followed, from what it sends; a
/// pass from before is laid over; deletes take the days they were asked to.
#[test]
fn event_changes_show_at_once_coalesce_and_survive_a_pass() {
    let mut lib = Library::in_memory().unwrap();
    calendars(&mut lib);
    let save = |event: Event, base: Option<Event>, create: bool| EventSave {
        event,
        base,
        create,
        alerts: None,
        repeat_changed: false,
        repeat: None,
        scope: None,
    };
    let mut fresh = event("NEW", "Lunch", 2_000_000);
    fresh.etag = None;
    record_event_save(&mut lib, save(fresh.clone(), None, true)).unwrap();
    assert_eq!(
        lib.event("NEW").unwrap().unwrap().title,
        "Lunch",
        "shown at once"
    );
    let mut renamed = fresh.clone();
    renamed.title = "Lunch with Asha".into();
    record_event_save(
        &mut lib,
        EventSave {
            alerts: Some(vec![15]),
            ..save(renamed, None, false)
        },
    )
    .unwrap();
    let p = lib.outbox_pending(CALENDAR).unwrap();
    assert_eq!(p.len(), 1, "one create");
    let s: EventSave = serde_json::from_str(&p[0].payload).unwrap();
    assert!(s.create && s.base.is_none());
    assert_eq!(
        (s.event.title.as_str(), s.alerts),
        ("Lunch with Asha", Some(vec![15]))
    );

    // On its way: the next edit follows, made from what it sends.
    lib.outbox_take(CALENDAR).unwrap();
    let mut later = s.event.clone();
    later.location = Some("Cafe".into());
    record_event_save(&mut lib, save(later, None, false)).unwrap();
    let p = lib.outbox_pending(CALENDAR).unwrap();
    let s2: EventSave = serde_json::from_str(&p[1].payload).unwrap();
    assert!(!s2.create);
    assert_eq!(s2.base.unwrap().title, "Lunch with Asha");

    // A repeating series, and deletes of its days.
    let series: Vec<Event> = (0..4)
        .map(|i| event(&format!("S__{i}"), "Standup", 3_000_000 + i * 86_400))
        .collect();
    lib.put_events(&series).unwrap();
    lib.put_events(&[event("E", "Dentist", 1_000_000)]).unwrap();
    record_event_delete(&mut lib, &series[2], Some(Scope::Future)).unwrap();
    let left = |lib: &Library| -> Vec<String> {
        let mut v: Vec<String> = lib
            .events_starting_between(0, i64::MAX / 2)
            .unwrap()
            .into_iter()
            .map(|(g, _)| g)
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        left(&lib),
        vec!["E", "NEW", "S__0", "S__1"],
        "this day and those after"
    );
    let mut moved = lib.event("E").unwrap().unwrap();
    moved.calendar_guid = "home".into();
    let base = lib.event("E").unwrap();
    record_event_save(&mut lib, save(moved, base, false)).unwrap();
    assert_eq!(
        lib.event("E").unwrap().unwrap().etag.as_deref(),
        Some("apple"),
        "Apple's tag kept"
    );

    // A pass from before all of it.
    let mut stale = series.clone();
    stale.push(event("E", "Dentist", 1_000_000));
    lib.put_events(&stale).unwrap();
    lib.forget_events(&["NEW".into()]).unwrap();
    replay(&mut lib, CALENDAR).unwrap();
    assert_eq!(left(&lib), vec!["E", "NEW", "S__0", "S__1"]);
    let new = lib.event("NEW").unwrap().unwrap();
    assert_eq!(
        (new.title.as_str(), new.location.as_deref()),
        ("Lunch with Asha", Some("Cafe"))
    );
    assert_eq!(lib.event("E").unwrap().unwrap().calendar_guid, "home");
    assert_eq!(
        kinds(&lib, CALENDAR)
            .into_iter()
            .map(|(_, k)| k)
            .collect::<Vec<_>>(),
        vec!["save", "save", "delete", "save"],
        "in the order made"
    );
}

/// A pretend Apple that holds one event: a save sent with its current tag is taken, one sent
/// with any other is answered "changed since" (412), as Apple's calendar does.
mod apple_calendar {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    fn apple_json(e: &Event, etag: &str) -> serde_json::Value {
        serde_json::json!({
            "guid": e.guid, "pGuid": e.calendar_guid, "title": e.title,
            "location": e.location, "tz": "Asia/Kolkata",
            "startDate": [20260914, 2026, 9, 14, 9, 30, 570],
            "endDate": [20260914, 2026, 9, 14, 10, 30, 630],
            "etag": etag
        })
    }

    /// Apple's copy and its tag number; each save taken makes a new tag.
    #[derive(Clone)]
    struct Held(Arc<StdMutex<(Event, u32)>>);

    impl Held {
        fn tag(&self) -> String {
            format!("v{}", self.0.lock().unwrap().1)
        }
        fn event(&self) -> Event {
            self.0.lock().unwrap().0.clone()
        }
    }

    struct Detail(Held);
    impl Respond for Detail {
        fn respond(&self, _: &Request) -> ResponseTemplate {
            let body = serde_json::json!({ "Event": [apple_json(&self.0.event(), &self.0.tag())] });
            ResponseTemplate::new(200).set_body_json(body)
        }
    }

    struct Save(Held);
    impl Respond for Save {
        fn respond(&self, r: &Request) -> ResponseTemplate {
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            let sent = body["Event"]["etag"].as_str().map(str::to_owned);
            if sent.as_deref() != Some(self.0.tag().as_str()) {
                return ResponseTemplate::new(412);
            }
            let mut held = self.0 .0.lock().unwrap();
            held.0.title = body["Event"]["title"].as_str().unwrap_or_default().into();
            held.0.location = body["Event"]["location"].as_str().map(str::to_owned);
            held.1 += 1;
            let tag = format!("v{}", held.1);
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "Event": [apple_json(&held.0, &tag)] }))
        }
    }

    async fn apple(holds: &Event, tag: u32) -> (MockServer, Held) {
        let held = Held(Arc::new(StdMutex::new((holds.clone(), tag))));
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(Detail(held.clone()))
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
            .respond_with(Save(held.clone()))
            .mount(&server)
            .await;
        (server, held)
    }

    fn dentist() -> Event {
        Event {
            guid: "E".into(),
            calendar_guid: "work".into(),
            title: "Dentist".into(),
            location: Some("Room 1".into()),
            starts_at: 1_789_358_400,
            ends_at: 1_789_362_000,
            timezone: Some("Asia/Kolkata".into()),
            etag: Some("v1".into()),
            ..Default::default()
        }
    }

    fn edit_of(base: &Event) -> EventSave {
        let mut mine = base.clone();
        mine.title = "Dentist — bring forms".into();
        EventSave {
            event: mine,
            base: Some(base.clone()),
            create: false,
            alerts: None,
            repeat_changed: false,
            repeat: None,
            scope: None,
        }
    }

    fn in_room_2(opened: &Event) -> Event {
        let mut phone = opened.clone();
        phone.location = Some("Room 2".into());
        phone.etag = Some("v2".into());
        phone
    }

    /// Sent as the desktop sends it: what Apple kept stored, what waits laid back over.
    async fn send_event(
        lib: &crate::library_lock::LibraryLock,
        http: &reqwest::Client,
        root: &str,
        p: Pending,
    ) -> Result<(), Error> {
        let kept = send(lib, http, root, Vec::new(), &p).await?;
        let mut l = lib.lock().await;
        l.atomically(|l| {
            quince_icloud::sync::store_events(l, kept)?;
            replay_except(l, CALENDAR, p.id)?;
            Ok::<_, Error>(())
        })
    }

    async fn send_waiting(lib: &crate::library_lock::LibraryLock, root: &str) {
        let http = reqwest::Client::new();
        let report = drain(lib, CALENDAR, &Mutex::new(()), |p| {
            send_event(lib, &http, root, p)
        })
        .await;
        assert!(report.refused.is_empty(), "{:?}", report.refused);
        assert!(lib
            .lock()
            .await
            .outbox_pending(CALENDAR)
            .unwrap()
            .is_empty());
    }

    fn title_and_room(e: &Event) -> (&str, Option<&str>) {
        (e.title.as_str(), e.location.as_deref())
    }

    /// Edited here offline; meanwhile the iPhone moved it to another room and a pass
    /// brought that in. Both changes survive — shown at once, and at Apple once sent: the
    /// save goes against the version it was made from, Apple answers "changed", and it is
    /// merged. Sent with the library's newest tag, Apple took it whole and the room went
    /// back.
    #[tokio::test]
    async fn an_offline_edit_and_a_newer_change_elsewhere_both_survive() {
        let mut lib = Library::in_memory().unwrap();
        calendars(&mut lib);
        let opened = dentist();
        lib.put_events(std::slice::from_ref(&opened)).unwrap();
        record_event_save(&mut lib, edit_of(&opened)).unwrap();

        // The iPhone's change, brought by a pass, with what waits laid back over it.
        let phone = in_room_2(&opened);
        lib.put_events(std::slice::from_ref(&phone)).unwrap();
        replay(&mut lib, CALENDAR).unwrap();
        assert_eq!(
            title_and_room(&lib.event("E").unwrap().unwrap()),
            ("Dentist — bring forms", Some("Room 2")),
            "both shown before it is sent"
        );

        let (server, held) = apple(&phone, 2).await;
        let lib = crate::library_lock::LibraryLock::new(lib, None);
        send_waiting(&lib, &server.uri()).await;
        assert_eq!(
            title_and_room(&held.event()),
            ("Dentist — bring forms", Some("Room 2")),
            "both kept at Apple"
        );
        let posts = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.method == reqwest::Method::POST)
            .count();
        assert_eq!(posts, 2, "refused as stale, then merged and taken");
        let stored = lib.lock().await.event("E").unwrap().unwrap();
        assert_eq!(stored.location.as_deref(), Some("Room 2"));
        assert_eq!(
            stored.etag.as_deref(),
            Some("v3"),
            "Apple's newest tag stored"
        );
    }

    /// The form was open while a pass landed with the iPhone's change: the save is made
    /// from the event as the form opened it, not the library's newer copy, so the room
    /// the form never showed is not put back.
    #[tokio::test]
    async fn a_form_open_while_a_pass_lands_does_not_overwrite_it() {
        let mut lib = Library::in_memory().unwrap();
        calendars(&mut lib);
        let opened = dentist();
        lib.put_events(std::slice::from_ref(&opened)).unwrap();

        // The pass, while the form is open.
        let phone = in_room_2(&opened);
        lib.put_events(std::slice::from_ref(&phone)).unwrap();

        // Saved from the form: its fields as opened, the title changed.
        let stored = lib.event("E").unwrap();
        let base = save_base("E", Some(opened.clone()), stored.clone());
        assert_eq!(
            base.as_ref(),
            Some(&opened),
            "the form's base, not the library's"
        );
        assert_eq!(
            save_base("E", None, stored.clone()),
            stored,
            "none sent: the library's"
        );
        let other = Event {
            guid: "OTHER".into(),
            ..dentist()
        };
        assert_eq!(
            save_base("E", Some(other), stored.clone()),
            stored,
            "another event's base is never taken"
        );
        record_event_save(
            &mut lib,
            EventSave {
                base,
                ..edit_of(&opened)
            },
        )
        .unwrap();
        let shown = lib.event("E").unwrap().unwrap();
        assert_eq!(
            title_and_room(&shown),
            ("Dentist — bring forms", Some("Room 2")),
            "the phone's room still shown"
        );
        assert_eq!(
            shown.etag.as_deref(),
            Some("v2"),
            "the library's tag is Apple's"
        );

        let (server, held) = apple(&phone, 2).await;
        let lib = crate::library_lock::LibraryLock::new(lib, None);
        send_waiting(&lib, &server.uri()).await;
        assert_eq!(
            title_and_room(&held.event()),
            ("Dentist — bring forms", Some("Room 2"))
        );
    }

    /// A second save made while the first is on its way follows it, made from what the
    /// first sends: once the first has landed, the second still goes, merged over it.
    #[tokio::test]
    async fn a_save_behind_one_that_landed_still_goes() {
        let mut lib = Library::in_memory().unwrap();
        calendars(&mut lib);
        let opened = dentist();
        lib.put_events(std::slice::from_ref(&opened)).unwrap();
        record_event_save(&mut lib, edit_of(&opened)).unwrap();
        lib.outbox_take(CALENDAR).unwrap();
        let mut later = lib.event("E").unwrap().unwrap();
        later.location = Some("Room 9".into());
        record_event_save(
            &mut lib,
            EventSave {
                event: later,
                ..edit_of(&opened)
            },
        )
        .unwrap();
        let (server, held) = apple(&opened, 1).await;
        let lib = crate::library_lock::LibraryLock::new(lib, None);
        send_waiting(&lib, &server.uri()).await;
        assert_eq!(
            title_and_room(&held.event()),
            ("Dentist — bring forms", Some("Room 9"))
        );
    }

    /// Whether any request read a span of days (`/ca/startup`, `/ca/events` listing): a
    /// change sent from the phone never needs one.
    async fn read_a_window(server: &MockServer) -> bool {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.method == reqwest::Method::GET && !r.url.path().contains("detail"))
    }

    /// H2: an event edited on the phone however far from today is sent by its id, against the
    /// version the phone had, merged with what changed elsewhere — no span of days is read, so
    /// an edit outside any span is never taken for a new event — and Apple's version comes
    /// back for the phone's row to take as its new base.
    #[tokio::test]
    async fn a_phone_edit_far_from_today_is_sent_by_id_and_comes_back() {
        let opened = dentist();
        let elsewhere = in_room_2(&opened);
        let (server, held) = apple(&elsewhere, 2).await;
        let mut l = Library::in_memory().unwrap();
        assert!(queue_phone_save(&mut l, edit_of(&opened)).unwrap());
        assert!(
            !queue_phone_save(&mut l, edit_of(&opened)).unwrap(),
            "asked again: nothing new"
        );
        let lib = crate::library_lock::LibraryLock::new(l, None);
        let http = reqwest::Client::new();
        let kept = std::sync::Mutex::new(Vec::new());
        let report = drain(&lib, CALENDAR, &Mutex::new(()), |p| {
            let (http, kept, lib, root) = (&http, &kept, &lib, server.uri());
            async move {
                let back = send(lib, http, &root, Vec::new(), &p).await?;
                kept.lock().unwrap().extend(back);
                Ok(())
            }
        })
        .await;
        assert!(
            report.refused.is_empty() && report.sent,
            "{:?}",
            report.refused
        );
        assert_eq!(
            title_and_room(&held.event()),
            ("Dentist — bring forms", Some("Room 2")),
            "both changes kept at Apple"
        );
        assert!(!read_a_window(&server).await);
        let kept = kept.into_inner().unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].etag.as_deref(),
            Some("v3"),
            "Apple's newest, for the row"
        );
    }

    /// H2: an event deleted on the phone however far from today is deleted at Apple — against
    /// Apple's newest version when it changed elsewhere since — never reported done unsent.
    #[tokio::test]
    async fn a_phone_delete_far_from_today_is_deleted_at_apple() {
        let server = MockServer::start().await;
        let deleted = Arc::new(StdMutex::new(Vec::<Option<String>>::new()));
        let seen = Arc::clone(&deleted);
        Mock::given(method("POST"))
            .and(path_regex(r".*/ca/events/work/E$"))
            .respond_with(move |r: &Request| {
                let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
                let tag = body["Event"]["etag"].as_str().map(str::to_owned);
                seen.lock().unwrap().push(tag.clone());
                if tag.as_deref() == Some("v2") {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({}))
                } else {
                    ResponseTemplate::new(412)
                }
            })
            .mount(&server)
            .await;
        let now = in_room_2(&dentist());
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/eventdetail/.*"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "Event": [apple_json(&now, "v2")] })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/ca/collectiondetail/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "Collection": [{ "guid": "work", "ctag": "c2" }] }),
            ))
            .mount(&server)
            .await;
        let mut l = Library::in_memory().unwrap();
        assert!(queue_phone_delete(&mut l, &dentist(), None).unwrap());
        assert!(
            !queue_phone_delete(&mut l, &dentist(), None).unwrap(),
            "once"
        );
        let lib = crate::library_lock::LibraryLock::new(l, None);
        let http = reqwest::Client::new();
        let report = drain(&lib, CALENDAR, &Mutex::new(()), |p| {
            let (http, lib, root) = (&http, &lib, server.uri());
            async move { send(lib, http, &root, Vec::new(), &p).await.map(|_| ()) }
        })
        .await;
        assert!(
            report.refused.is_empty() && report.sent,
            "{:?}",
            report.refused
        );
        assert_eq!(
            *deleted.lock().unwrap(),
            vec![Some("v1".to_string()), Some("v2".to_string())],
            "against the phone's version, then Apple's newest"
        );
        assert!(!read_a_window(&server).await);
    }
}
