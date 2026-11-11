//! Contacts' changes: recorded and laid (the desktop), queued (the phone), and sent against a
//! pretend Apple. Moved here from the desktop's outbox with the code they test.

use super::*;
use crate::library_lock::LibraryLock;
use crate::queue::{content_of, drain, drain_as, ACCOUNT_KEY};
use tokio::sync::Mutex as AsyncMutex;

fn person(guid: &str, first: &str) -> Contact {
    Contact {
        guid: guid.into(),
        first_name: Some(first.into()),
        middle_name: None,
        last_name: None,
        company: None,
        is_company: false,
        birthday: None,
        notes: None,
        photo_url: None,
        photo_signature: None,
        photo_crop: None,
        etag: None,
        methods: vec![],
    }
}

/// Contacts: a new card gets its id here, shows at once, and edits before it is sent stay
/// one create; a pass that does not have it yet keeps it.
#[test]
fn a_new_card_shows_at_once_and_stays_one_create() {
    let mut lib = Library::in_memory().unwrap();
    lib.put_contacts(&[person("X", "Old")]).unwrap();
    let guid = record_contact_save(&mut lib, &person("", "Asha"), None).unwrap();
    assert!(!guid.is_empty());
    record_contact_save(
        &mut lib,
        &person(&guid, "Asha Rao"),
        Some(person(&guid, "Asha")),
    )
    .unwrap();
    let p = lib.outbox_pending(CONTACTS).unwrap();
    assert_eq!(p.len(), 1);
    let s: ContactSave = serde_json::from_str(&p[0].payload).unwrap();
    assert!(s.create && s.base.is_none());
    assert_eq!(s.contact.first_name.as_deref(), Some("Asha Rao"));

    // An edit to an existing card keeps the form's base.
    record_contact_save(&mut lib, &person("X", "New"), Some(person("X", "Old"))).unwrap();
    // A pass that read the book before either.
    lib.put_contacts(&[person("X", "Old")]).unwrap();
    replay(&mut lib).unwrap();
    let names: Vec<(String, Option<String>)> = lib
        .contacts(None)
        .unwrap()
        .into_iter()
        .map(|c| (c.guid, c.first_name))
        .collect();
    assert!(names.contains(&(guid.clone(), Some("Asha Rao".into()))));
    assert!(names.contains(&("X".into(), Some("New".into()))));
}

fn address(city: &str) -> PostalAddress {
    PostalAddress {
        label: Some("HOME".into()),
        street: Some("1 Example Road".into()),
        city: Some(city.into()),
        country: Some("India".into()),
        ..PostalAddress::default()
    }
}

/// Contacts: an address added to a card made here, before Apple has the card, shows at
/// once and waits behind the card's own create, in order; a pass that has neither yet
/// keeps both on screen. Nothing is sent to Apple until the card is.
#[test]
fn an_address_on_a_new_card_waits_behind_its_create() {
    let mut lib = Library::in_memory().unwrap();
    let guid = record_contact_save(&mut lib, &person("", "Asha"), None).unwrap();
    let add = vec![AddressEdit::Add {
        address: address("Pune"),
    }];
    let shown = record_contact_address(&mut lib, &guid, vec![], add.clone()).unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(lib.contact_addresses(&guid).unwrap(), shown);

    let p = lib.outbox_pending(CONTACTS).unwrap();
    let kinds: Vec<&str> = p.iter().map(|p| p.kind.as_str()).collect();
    assert_eq!(kinds, ["save", "address"], "the card goes first");
    let a: ContactAddress = serde_json::from_str(&p[1].payload).unwrap();
    assert_eq!((a.base.len(), a.edits.clone()), (0, add));
    assert_eq!(a.name, "Asha");

    // A pass from before either: the book without the card. Both are laid back.
    lib.put_contacts(&[]).unwrap();
    replay(&mut lib).unwrap();
    assert_eq!(lib.contact_addresses(&guid).unwrap(), shown);

    // The notice for a refusal names the person.
    assert_eq!(content_of(&p[1]), "Asha");
}

/// An address edit on a card Apple has is laid over each pass: a part the phone changed
/// meanwhile stays, the part edited here wins, and laying it again changes nothing.
#[test]
fn an_address_edit_lays_over_each_pass_once() {
    let mut lib = Library::in_memory().unwrap();
    lib.put_contacts(&[person("X", "Ravi")]).unwrap();
    lib.set_contact_addresses("X", &[address("Pune")]).unwrap();
    let edit = vec![AddressEdit::SetField {
        index: 0,
        field: "city".into(),
        value: Some("Mumbai".into()),
    }];
    record_contact_address(&mut lib, "X", vec![address("Pune")], edit).unwrap();
    // The phone added a postcode meanwhile; a pass brings Apple's older city with it.
    let mut phone = address("Pune");
    phone.postal_code = Some("411001".into());
    lib.set_contact_addresses("X", &[phone]).unwrap();
    replay(&mut lib).unwrap();
    replay(&mut lib).unwrap();
    let now = lib.contact_addresses("X").unwrap();
    assert_eq!(now.len(), 1);
    assert_eq!(now[0].city.as_deref(), Some("Mumbai"));
    assert_eq!(now[0].postal_code.as_deref(), Some("411001"));
}

/// An address edit for a card deleted here before it went: nothing to show it on, and
/// the replay leaves no orphaned address behind.
#[test]
fn an_address_for_a_deleted_card_is_not_laid() {
    let mut lib = Library::in_memory().unwrap();
    lib.put_contacts(&[person("X", "Ravi")]).unwrap();
    record_contact_address(
        &mut lib,
        "X",
        vec![],
        vec![AddressEdit::Add {
            address: address("Pune"),
        }],
    )
    .unwrap();
    record_contact_delete(&mut lib, &person("X", "Ravi")).unwrap();
    replay(&mut lib).unwrap();
    assert!(lib.contact_addresses("X").unwrap().is_empty());
}

/// Contacts: a card deleted shows gone at once and stays gone over a pass that still has
/// it; edited then deleted, both go, in that order.
#[test]
fn a_deleted_card_stays_gone_over_a_pass() {
    let mut lib = Library::in_memory().unwrap();
    lib.put_contacts(&[person("X", "Asha"), person("Y", "Ravi")])
        .unwrap();
    record_contact_save(&mut lib, &person("X", "Asha R"), Some(person("X", "Asha"))).unwrap();
    let x = person("X", "Asha R");
    record_contact_delete(&mut lib, &x).unwrap();
    let names = |lib: &Library| -> Vec<String> {
        lib.contacts(None)
            .unwrap()
            .into_iter()
            .map(|c| c.guid)
            .collect()
    };
    assert_eq!(names(&lib), vec!["Y".to_string()]);
    assert_eq!(
        kinds(&lib, CONTACTS),
        vec![("X".into(), "save".into()), ("X".into(), "delete".into())]
    );
    lib.put_contacts(&[person("X", "Asha"), person("Y", "Ravi")])
        .unwrap();
    replay(&mut lib).unwrap();
    assert_eq!(
        names(&lib),
        vec!["Y".to_string()],
        "the pass does not bring it back"
    );
}

/// Three cards edited offline go in one run from one read of the address book — each read
/// is the whole book — not two reads for each card, while Apple answers each write with
/// the tokens for the next; a card edited twice is read again before its second write, its
/// copy from before the first being old.
#[tokio::test]
async fn queued_card_edits_are_sent_from_one_read_of_the_book() {
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let card = |id: &str| serde_json::json!({ "contactId": id, "firstName": id, "etag": "e" });
    Mock::given(method("GET"))
        .and(path("/co/startup"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "syncToken": "S", "prefToken": "P",
            "contacts": [card("A"), card("B"), card("C")]
        })))
        .mount(&server)
        .await;
    let answered = std::sync::atomic::AtomicU64::new(0);
    Mock::given(path_regex(r".*/co/contacts/card$"))
        .respond_with(move |_: &wiremock::Request| {
            let n = answered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "contacts": [], "syncToken": format!("S{n}"), "prefToken": "P",
            }))
        })
        .mount(&server)
        .await;

    let mut l = Library::in_memory().unwrap();
    for (id, name) in [("A", "Ada"), ("B", "Bea"), ("C", "Cy")] {
        let was = person(id, id);
        l.put_contacts(std::slice::from_ref(&was)).unwrap();
        record_contact_save(&mut l, &person(id, name), Some(was)).unwrap();
    }
    // Behind a save of A that is on its way: a second edit of A, sent after it.
    l.outbox_take(CONTACTS).unwrap();
    record_contact_save(&mut l, &person("A", "Ada L"), Some(person("A", "Ada"))).unwrap();
    l.set_state(ACCOUNT_KEY, "me").unwrap();
    let lib = LibraryLock::new(l, None);
    let http = reqwest::Client::new();
    let book = std::sync::Mutex::new(Book::default());
    let ledger = quince_icloud::contacts::Ledger::new();
    let root = server.uri();
    let report = drain_as(&lib, CONTACTS, &AsyncMutex::new(()), "me", |p| {
        send(&http, &root, Vec::new(), p, &book, &ledger)
    })
    .await;
    assert!(report.refused.is_empty(), "{:?}", report.refused);
    assert!(lib
        .lock()
        .await
        .outbox_pending(CONTACTS)
        .unwrap()
        .is_empty());
    let asked = server.received_requests().await.unwrap();
    let reads = asked
        .iter()
        .filter(|r| r.url.path() == "/co/startup")
        .count();
    let writes = asked
        .iter()
        .filter(|r| r.url.path().ends_with("/co/contacts/card"))
        .count();
    assert_eq!(writes, 4);
    assert_eq!(
        reads, 2,
        "one read for the run, and A's again before its second write"
    );
}

fn kinds(lib: &Library, service: &str) -> Vec<(String, String)> {
    lib.outbox_pending(service)
        .unwrap()
        .into_iter()
        .map(|p| (p.record, p.kind))
        .collect()
}

fn card(guid: &str, first: &str) -> Card {
    Card {
        contact: person(guid, first),
        details: ContactDetails::default(),
    }
}

fn payload<T: serde::de::DeserializeOwned>(p: &Pending) -> T {
    serde_json::from_str(&p.payload).unwrap()
}

/// A pretend Apple address book: `/co/startup` answers the cards and lists it holds, every
/// write is kept (path, `method` asked, body) and changes what it holds, and each answer moves
/// the sync token on, as Apple's does. `busy_lists` list writes are answered 503 first.
mod apple {
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use wiremock::{Request, Respond, ResponseTemplate};

    #[derive(Default)]
    pub struct Held {
        pub cards: Vec<Value>,
        pub lists: Vec<Value>,
        pub reads: usize,
        pub writes: Vec<(String, Option<String>, Value)>,
        pub busy_lists: usize,
        token: u64,
    }

    #[derive(Clone, Default)]
    pub struct Apple(pub Arc<Mutex<Held>>);

    impl Apple {
        pub fn holding(cards: Vec<Value>, lists: Vec<Value>) -> Self {
            Apple(Arc::new(Mutex::new(Held {
                cards,
                lists,
                ..Held::default()
            })))
        }
    }

    impl Respond for Apple {
        fn respond(&self, r: &Request) -> ResponseTemplate {
            let mut h = self.0.lock().unwrap();
            let path = r.url.path().to_string();
            if path.ends_with("/co/startup") {
                h.reads += 1;
                return ResponseTemplate::new(200).set_body_json(json!({
                    "syncToken": format!("S{}", h.token), "prefToken": "P",
                    "contacts": h.cards, "groups": h.lists,
                }));
            }
            let method = r
                .url
                .query_pairs()
                .find(|(k, _)| k == "method")
                .map(|(_, v)| v.to_string());
            let body: Value = serde_json::from_slice(&r.body).unwrap_or_default();
            if path.contains("/co/groups/card") {
                if h.busy_lists > 0 {
                    h.busy_lists -= 1;
                    return ResponseTemplate::new(503);
                }
                let group = body["groups"][0].clone();
                let id = group["groupId"].clone();
                h.lists.retain(|l| l["groupId"] != id);
                if method.as_deref() != Some("DELETE") {
                    h.lists.push(group);
                }
            } else if path.contains("/co/contacts/card") {
                let card = body["contacts"][0].clone();
                let id = card["contactId"].clone();
                h.cards.retain(|c| c["contactId"] != id);
                if method.as_deref() != Some("DELETE") {
                    h.cards.push(card);
                }
            }
            h.writes.push((path, method, body));
            h.token += 1;
            ResponseTemplate::new(200).set_body_json(json!({
                "contacts": [], "syncToken": format!("S{}", h.token), "prefToken": "P",
            }))
        }
    }
}

async fn pretend(apple: &apple::Apple) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(apple.clone())
        .mount(&server)
        .await;
    server
}

/// Every change waiting for Contacts sent as the phone's engine sends it: one run, one book.
async fn send_waiting(lib: &LibraryLock, root: &str) -> crate::queue::Report {
    let http = reqwest::Client::new();
    let book = std::sync::Mutex::new(Book::default());
    let ledger = Ledger::new();
    drain_as(lib, CONTACTS, &AsyncMutex::new(()), "me", |p| {
        send(&http, root, Vec::new(), p, &book, &ledger)
    })
    .await
}

fn owned(mut l: Library) -> LibraryLock {
    l.set_state(ACCOUNT_KEY, "me").unwrap();
    LibraryLock::new(l, None)
}

/// The phone's state of a card is queued again on every pass until Apple has it: asked for
/// exactly what waits, nothing is queued; edited before it went, it stays one save from the
/// first base; edited after it went, the edit goes behind it, made from what it sent — so a
/// name changed and then put back on the phone is put back at Apple too. Nothing is laid over
/// the library: Android's Contacts already shows it.
#[test]
fn the_phones_card_queued_again_is_no_change_and_follows_what_went() {
    let mut lib = Library::in_memory().unwrap();
    let base = card("X", "Asha");
    assert!(queue_phone_card(&mut lib, &card("X", "Asha R"), Some(&base), false).unwrap());
    assert!(
        !queue_phone_card(&mut lib, &card("X", "Asha R"), Some(&base), false).unwrap(),
        "asked again for what waits: nothing new"
    );
    assert!(queue_phone_card(&mut lib, &card("X", "Asha Rao"), Some(&base), false).unwrap());
    let p = lib.outbox_pending(CONTACTS).unwrap();
    assert_eq!(p.len(), 1, "one save");
    let s: ContactSave = payload(&p[0]);
    assert_eq!(s.base.unwrap().first_name.as_deref(), Some("Asha"));

    lib.outbox_take(CONTACTS).unwrap();
    assert!(!queue_phone_card(&mut lib, &card("X", "Asha Rao"), Some(&base), false).unwrap());
    assert!(queue_phone_card(&mut lib, &base, Some(&base), false).unwrap());
    let p = lib.outbox_pending(CONTACTS).unwrap();
    assert_eq!(p.len(), 2, "behind the one on its way");
    let s: ContactSave = payload(&p[1]);
    assert_eq!(
        (
            s.base.unwrap().first_name.as_deref(),
            s.contact.first_name.as_deref()
        ),
        (Some("Asha Rao"), Some("Asha")),
        "from what the first sends, so the name goes back"
    );
    assert!(lib.contacts(None).unwrap().is_empty(), "nothing laid");

    assert!(queue_phone_delete(&mut lib, &base.contact).unwrap());
    assert!(
        !queue_phone_delete(&mut lib, &base.contact).unwrap(),
        "once"
    );

    // A card made on the phone is a create under the phone's id, once.
    let made = card("NEW-ON-PHONE", "Ravi");
    assert!(queue_phone_card(&mut lib, &made, None, true).unwrap());
    assert!(!queue_phone_card(&mut lib, &made, None, true).unwrap());
    let s: ContactSave = payload(lib.outbox_pending(CONTACTS).unwrap().last().unwrap());
    assert!(s.create && s.base.is_none());
}

/// The edits that make one list of addresses another, as a form makes them: each address
/// part by part, added ones added, taken-out ones taken out — and applied, they give the list.
#[test]
fn address_edits_make_the_list_the_phone_has() {
    let mut work = address("Pune");
    work.label = Some("WORK".into());
    let base = vec![address("Pune"), work.clone()];
    let mut moved = address("Mumbai");
    moved.postal_code = Some("400001".into());
    let cases: Vec<Vec<PostalAddress>> = vec![
        vec![moved.clone(), work.clone()],
        vec![moved.clone()],
        vec![],
        // Added, an address takes its country's code from the name, as Apple keeps it.
        vec![
            address("Pune"),
            work.clone(),
            PostalAddress {
                country_code: Some("in".into()),
                ..address("Delhi")
            },
        ],
        base.clone(),
    ];
    for now in cases {
        let edits = address_edits(&base, &now);
        let (after, _) = postal::apply_edits(&base, &edits).unwrap();
        assert_eq!(after, now, "{edits:?}");
    }
    assert!(address_edits(&base, &base).is_empty());
    // A city changed is that part alone: the street, set on another device meanwhile, stays.
    let edits = address_edits(&base, &[moved.clone(), work]);
    assert!(edits
        .iter()
        .all(|e| matches!(e, AddressEdit::SetField { index: 0, .. })));
}

/// The phone's addresses are queued as edits, once; put back before they went, they are taken
/// back; changed again after they went, the next goes behind, from what the first sends.
#[test]
fn the_phones_addresses_are_queued_once_and_taken_back_when_put_back() {
    let mut lib = Library::in_memory().unwrap();
    let base = vec![address("Pune")];
    let now = vec![address("Mumbai")];
    assert!(queue_phone_addresses(&mut lib, "X", &base, &now, "Ravi").unwrap());
    assert!(!queue_phone_addresses(&mut lib, "X", &base, &now, "Ravi").unwrap());
    assert_eq!(kinds(&lib, CONTACTS), vec![("X".into(), "address".into())]);
    assert!(queue_phone_addresses(&mut lib, "X", &base, &base, "Ravi").unwrap());
    assert!(
        lib.outbox_pending(CONTACTS).unwrap().is_empty(),
        "taken back"
    );
    assert!(!queue_phone_addresses(&mut lib, "X", &base, &base, "Ravi").unwrap());

    queue_phone_addresses(&mut lib, "X", &base, &now, "Ravi").unwrap();
    lib.outbox_take(CONTACTS).unwrap();
    let later = vec![address("Delhi")];
    assert!(queue_phone_addresses(&mut lib, "X", &base, &later, "Ravi").unwrap());
    let p = lib.outbox_pending(CONTACTS).unwrap();
    let a: ContactAddress = payload(&p[1]);
    assert_eq!(a.base, now, "from what the first sends");
    assert_eq!(content_of(&p[1]), "Ravi");
}

/// M5: an address edited on the phone reaches Apple, the part changed and only it: the street
/// another device wrote meanwhile stays, and the card's other details are left alone.
#[tokio::test]
async fn an_address_edited_on_the_phone_is_sent_part_by_part() {
    use serde_json::json;
    let apple = apple::Apple::holding(
        vec![json!({
            "contactId": "X", "etag": "e1", "firstName": "Ravi",
            "phones": [{ "label": "MOBILE", "field": "+1 555 0100" }],
            "streetAddresses": [{ "label": "HOME", "field": {
                "street": "9 Other Road", "city": "Pune", "country": "India", "countryCode": "in"
            } }],
        })],
        vec![],
    );
    let server = pretend(&apple).await;
    let mut l = Library::in_memory().unwrap();
    let mut base = address("Pune");
    base.country_code = Some("in".into());
    let mut now = base.clone();
    now.city = Some("Mumbai".into());
    queue_phone_addresses(&mut l, "X", &[base], &[now], "Ravi").unwrap();
    let lib = owned(l);
    let report = send_waiting(&lib, &server.uri()).await;
    assert!(
        report.refused.is_empty() && report.sent,
        "{:?}",
        report.refused
    );
    let held = apple.0.lock().unwrap();
    let card = &held.cards[0];
    let at = &card["streetAddresses"][0]["field"];
    assert_eq!(at["city"], "Mumbai");
    assert_eq!(
        at["street"], "9 Other Road",
        "the other device's street kept"
    );
    assert_eq!(card["phones"][0]["field"], "+1 555 0100");
}

/// H6: a run of changes made on the phone — cards edited, one deleted, a label renamed and a
/// card's labels changed — goes from one read of the address book, not one read per change.
#[tokio::test]
async fn a_run_of_phone_changes_reads_the_book_once() {
    use serde_json::json;
    let entry = |id: &str| json!({ "contactId": id, "etag": "e", "firstName": id });
    let apple = apple::Apple::holding(
        vec![entry("A"), entry("B"), entry("C"), entry("D")],
        vec![
            json!({ "groupId": "L1", "name": "Family", "contactIds": ["A"], "etag": "g" }),
            json!({ "groupId": "L2", "name": "Work", "contactIds": [], "etag": "g" }),
        ],
    );
    let server = pretend(&apple).await;
    let mut l = Library::in_memory().unwrap();
    for (id, name) in [("A", "Ada"), ("B", "Bea"), ("C", "Cy")] {
        queue_phone_card(&mut l, &card(id, name), Some(&card(id, id)), false).unwrap();
    }
    queue_phone_delete(&mut l, &person("D", "D")).unwrap();
    queue_phone_list(&mut l, "L2", "Work and school", false).unwrap();
    queue_phone_lists(&mut l, "B", &[], &["L1".into()], "Bea").unwrap();
    let lib = owned(l);
    let report = send_waiting(&lib, &server.uri()).await;
    assert!(
        report.refused.is_empty() && report.sent,
        "{:?}",
        report.refused
    );
    let held = apple.0.lock().unwrap();
    assert_eq!(held.reads, 1, "{:?}", held.writes);
    assert_eq!(held.writes.len(), 6);
    assert!(!held.cards.iter().any(|c| c["contactId"] == "D"));
}

/// M4: a card's labels changed on the phone are kept until Apple has them: Apple busy for the
/// list write leaves the card's own save sent and the label change waiting, sent next time.
#[tokio::test]
async fn a_label_change_apple_could_not_take_waits_and_goes_next_time() {
    use serde_json::json;
    let apple = apple::Apple::holding(
        vec![json!({ "contactId": "A", "etag": "e", "firstName": "A" })],
        vec![json!({ "groupId": "L1", "name": "Family", "contactIds": [], "etag": "g" })],
    );
    apple.0.lock().unwrap().busy_lists = 1;
    let server = pretend(&apple).await;
    let mut l = Library::in_memory().unwrap();
    queue_phone_card(&mut l, &card("A", "Ada"), Some(&card("A", "A")), false).unwrap();
    queue_phone_lists(&mut l, "A", &[], &["L1".into()], "Ada").unwrap();
    let lib = owned(l);
    let first = send_waiting(&lib, &server.uri()).await;
    assert!(matches!(first.outcome, crate::queue::Outcome::Later));
    assert_eq!(
        kinds(&*lib.lock().await, CONTACTS),
        vec![("A".into(), CARD_LISTS.into())],
        "the card went, its labels wait"
    );
    let second = send_waiting(&lib, &server.uri()).await;
    assert!(second.outcome.is_clear() && second.sent);
    assert_eq!(apple.0.lock().unwrap().lists[0]["contactIds"], json!(["A"]));
}

/// M8: an edit to a card deleted on another device meanwhile is refused, as on the desktop —
/// kept with what was typed for Try Again or Discard — never made again as a new card.
#[tokio::test]
async fn an_edit_to_a_card_deleted_elsewhere_is_refused_not_made_again() {
    let apple = apple::Apple::holding(vec![], vec![]);
    let server = pretend(&apple).await;
    let mut l = Library::in_memory().unwrap();
    queue_phone_card(
        &mut l,
        &card("X", "Asha R"),
        Some(&card("X", "Asha")),
        false,
    )
    .unwrap();
    let lib = owned(l);
    let report = send_waiting(&lib, &server.uri()).await;
    assert_eq!(report.refused.len(), 1);
    assert!(apple.0.lock().unwrap().writes.is_empty(), "nothing made");
    let refused = lib.lock().await.outbox_refused().unwrap();
    assert_eq!(content_of(&refused[0].change), "Asha R");
}

/// M6: a change Apple keeps answering as a conflict is held as refused after PROTOCOL_TRIES
/// sends, never retried for ever with everything queued behind it waiting.
#[tokio::test]
async fn a_conflict_that_never_settles_is_refused_after_its_tries() {
    let mut l = Library::in_memory().unwrap();
    queue_phone_card(
        &mut l,
        &card("X", "Asha R"),
        Some(&card("X", "Asha")),
        false,
    )
    .unwrap();
    queue_phone_card(&mut l, &card("Y", "Ravi"), Some(&card("Y", "R")), false).unwrap();
    let lib = owned(l);
    let pass = AsyncMutex::new(());
    let mut runs = 0;
    loop {
        runs += 1;
        let report = drain(&lib, CONTACTS, &pass, |p| async move {
            if p.record == "X" {
                Err(Error::Conflict)
            } else {
                Ok(())
            }
        })
        .await;
        if report.outcome.is_clear() {
            assert_eq!(report.refused.len(), 1);
            break;
        }
        assert!(
            runs < crate::queue::PROTOCOL_TRIES,
            "retried past its tries"
        );
    }
    assert_eq!(runs, crate::queue::PROTOCOL_TRIES);
    let l = lib.lock().await;
    assert!(
        l.outbox_pending(CONTACTS).unwrap().is_empty(),
        "Y went after"
    );
    assert_eq!(l.outbox_refused().unwrap()[0].change.record, "X");
}
