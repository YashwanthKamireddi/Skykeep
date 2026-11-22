//! Reminders' changes sent against a pretend Apple.

use super::*;
use crate::queue::drain;
use quince_core::reminders::ReminderList;
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;
use wiremock::matchers::{body_string_contains, method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn library_with_list() -> Library {
    let mut lib = Library::in_memory().unwrap();
    lib.put_reminder_lists(&[ReminderList {
        guid: "List/A".into(),
        title: "Errands".into(),
        colour: None,
        ctag: None,
        sort_order: 0,
        enabled: true,
        emblem: None,
        ..Default::default()
    }])
    .unwrap();
    lib
}

fn reminder(guid: &str, parent: Option<&str>) -> Reminder {
    Reminder {
        guid: guid.into(),
        list_guid: "List/A".into(),
        title: guid.into(),
        parent_guid: parent.map(Into::into),
        etag: Some("t1".into()),
        ..Default::default()
    }
}

/// A pretend Apple that takes every create and answers every other write with `writes`: a
/// 500 while it fails, a 200 once it does not. `made` is whether it holds the reminder already.
async fn apple(writes: u16, made: bool) -> MockServer {
    let server = MockServer::start().await;
    let lookup = if made {
        ResponseTemplate::new(200).set_body_json(json!({ "records": [{
            "recordName": "x", "recordType": "Reminder", "recordChangeTag": "t2", "fields": {}
        }] }))
    } else {
        ResponseTemplate::new(404)
    };
    Mock::given(method("POST"))
        .and(path_regex(r".*/records/lookup$"))
        .respond_with(lookup)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/records/modify$"))
        .and(body_string_contains(r#""operationType":"create""#))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({ "records": [{ "recordName": "x", "recordChangeTag": "t2" }] }),
            ),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/records/modify$"))
        .respond_with(
            ResponseTemplate::new(writes).set_body_json(
                json!({ "records": [{ "recordName": "x", "recordChangeTag": "t2" }] }),
            ),
        )
        .mount(&server)
        .await;
    server
}

async fn send_all(lib: &LibraryLock, server: &MockServer) -> crate::queue::Outcome {
    let http = reqwest::Client::new();
    let root = server.uri();
    drain(lib, REMINDERS, &AsyncMutex::new(()), |p| {
        send(lib, &http, &root, vec![], p)
    })
    .await
    .outcome
}

/// What was asked of each record: `create`, `update` (writes) in the order sent.
async fn writes(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/records/modify"))
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .flat_map(|b| {
            b["operations"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|o| o["operationType"].as_str().unwrap_or("").to_string())
        })
        .collect()
}

/// A new reminder's date and flag go in a second write after the create. Apple failing that
/// one kept the change queued, and sent again it made nothing twice: the date reached Apple.
/// Before, the change came off the queue as sent and the date was gone for good (#240).
#[tokio::test]
async fn a_new_reminders_date_waits_until_apple_takes_it() {
    let mut l = library_with_list();
    record_reminder_create(
        &mut l,
        &Create {
            list_guid: "List/A".into(),
            title: "Buy milk".into(),
            due_at: Some(1_800_000_000),
            all_day: Some(false),
            flagged: Some(true),
        },
    )
    .unwrap();
    let lib = LibraryLock::new(l, None);

    let failing = apple(500, false).await;
    assert!(!send_all(&lib, &failing).await.is_clear());
    assert_eq!(
        lib.lock().await.outbox_pending(REMINDERS).unwrap().len(),
        1,
        "the date not taken keeps the change queued"
    );

    assert_eq!(writes(&failing).await, ["create", "update"]);

    // Sent again, to an Apple that holds the reminder now: only the date and flag go.
    let free = apple(200, true).await;
    assert!(send_all(&lib, &free).await.is_clear());
    assert!(lib
        .lock()
        .await
        .outbox_pending(REMINDERS)
        .unwrap()
        .is_empty());
    assert!(
        !writes(&free).await.iter().any(|w| w == "create"),
        "made once, never twice"
    );
    let sent = free.received_requests().await.unwrap();
    let date = sent
        .iter()
        .filter(|r| r.url.path().ends_with("/records/modify"))
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .find(|b| !b["operations"][0]["record"]["fields"]["DueDate"].is_null())
        .expect("the date was sent again");
    let fields = &date["operations"][0]["record"]["fields"];
    assert_eq!(fields["DueDate"]["value"], 1_800_000_000_000_i64);
    assert_eq!(fields["Flagged"]["value"], 1);
}

/// A subtask Apple did not take — moved with its parent, or deleted with it — kept the change
/// queued instead of leaving it in the old list or behind as an orphan (#240).
#[tokio::test]
async fn a_subtask_left_behind_keeps_the_change_queued() {
    let mut l = library_with_list();
    l.put_reminder_lists(&[
        ReminderList {
            guid: "List/A".into(),
            title: "Errands".into(),
            enabled: true,
            ..Default::default()
        },
        ReminderList {
            guid: "List/B".into(),
            title: "Home".into(),
            enabled: true,
            ..Default::default()
        },
    ])
    .unwrap();
    l.put_reminders(&[
        reminder("Reminder/P", None),
        reminder("Reminder/C", Some("Reminder/P")),
    ])
    .unwrap();
    let parent = l.reminder("Reminder/P").unwrap().unwrap();
    record_reminder_edit(
        &mut l,
        &parent,
        &ReminderEdit {
            list_guid: Some("List/B".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let lib = LibraryLock::new(l, None);

    // Apple takes the parent’s move and fails its subtask’s.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/records/lookup$"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/records/modify$"))
        .and(body_string_contains("Reminder/C"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/records/modify$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({ "records": [{ "recordName": "x", "recordChangeTag": "t2" }] }),
            ),
        )
        .mount(&server)
        .await;
    assert!(!send_all(&lib, &server).await.is_clear());
    assert_eq!(
        lib.lock().await.outbox_pending(REMINDERS).unwrap().len(),
        1,
        "move"
    );

    // Deleted, the same: the parent goes, its subtask does not, the change waits.
    let lib = {
        let mut l = library_with_list();
        l.put_reminders(&[
            reminder("Reminder/P", None),
            reminder("Reminder/C", Some("Reminder/P")),
        ])
        .unwrap();
        let parent = l.reminder("Reminder/P").unwrap().unwrap();
        record_reminder_delete(&mut l, &parent).unwrap();
        LibraryLock::new(l, None)
    };
    assert!(!send_all(&lib, &server).await.is_clear());
    assert_eq!(
        lib.lock().await.outbox_pending(REMINDERS).unwrap().len(),
        1,
        "delete"
    );
    assert!(writes(&server).await.iter().all(|w| w == "update"));
}
