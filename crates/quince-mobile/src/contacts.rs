//! Contacts on the phone, sent by the desktop's own code. Android's Contacts is the store: a
//! card changed there (its row marked dirty) is handed over here as it is now, against the card
//! as last read from Apple, and queued in the engine's outbox — the queue the desktop sends from
//! — then sent with `quince_sync::contacts::send`: one read of the book for the run, a change
//! Apple can't take now kept for the next, one it refuses kept for the person with Try Again and
//! Discard. Android's row is let go of only once its changes have gone or been refused.
//!
//! The book is read back with the tokens kept from the last read, so an address book nothing
//! changed in is not read again (`/co/changeset`, as the desktop's pass asks).

use crate::{
    failed, runtime, Account, Book, CList, ChangeKind, EngineError, Method, Person, PhotoEdit,
    PhotoRef,
};
use quince_core::contacts::{Card, ContactDetails, PostalAddress};
use quince_icloud::contacts::ME_KEY;
use quince_sync::contacts::{self as shared, CONTACTS};
use quince_sync::queue::Report;
use std::collections::HashMap;
use tokio::sync::Mutex;

/// One pass at a time over Contacts, sending or reading: a read between a send and its
/// removal from the queue would be read over by what still waits.
static PASS: Mutex<()> = Mutex::const_new(());

/// A postal address, part by part, as iCloud keeps it and Android's structured address holds
/// it. `sub_administrative_area` and `country_code` have no place on the phone: the phone sends
/// them as the card had them.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub label: Option<String>,
    pub street: Option<String>,
    pub sub_locality: Option<String>,
    pub city: Option<String>,
    pub sub_administrative_area: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    /// The lines as one text, as Apple's own apps lay it out, for Android's formatted address.
    pub formatted: String,
}

impl From<PostalAddress> for Address {
    fn from(a: PostalAddress) -> Self {
        Address {
            formatted: a.formatted(),
            label: a.label,
            street: a.street,
            sub_locality: a.sub_locality,
            city: a.city,
            sub_administrative_area: a.sub_administrative_area,
            state: a.state,
            postal_code: a.postal_code,
            country: a.country,
            country_code: a.country_code,
        }
    }
}

impl From<&Address> for PostalAddress {
    fn from(a: &Address) -> Self {
        PostalAddress {
            label: a.label.clone(),
            street: a.street.clone(),
            sub_locality: a.sub_locality.clone(),
            city: a.city.clone(),
            sub_administrative_area: a.sub_administrative_area.clone(),
            state: a.state.clone(),
            postal_code: a.postal_code.clone(),
            country: a.country.clone(),
            country_code: a.country_code.clone(),
        }
    }
}

/// The phone's addresses as iCloud will hold them: the parts the phone has no place for taken
/// from the address in the same place on the card as last read.
fn phone_addresses(now: &[Address], base: &[PostalAddress]) -> Vec<PostalAddress> {
    now.iter()
        .enumerate()
        .map(|(i, a)| {
            let mut out = PostalAddress::from(a);
            if let Some(b) = base.get(i) {
                if out.sub_administrative_area.is_none() {
                    out.sub_administrative_area
                        .clone_from(&b.sub_administrative_area);
                }
                if out.country_code.is_none() && out.country == b.country {
                    out.country_code.clone_from(&b.country_code);
                }
            }
            out
        })
        .collect()
}

/// One card changed on the phone: as it is now (`person`, with the lists it is in now in its
/// `groups`), and as it was last read from Apple (`base`).
#[derive(uniffi::Record)]
pub struct PhoneCard {
    pub kind: ChangeKind,
    pub guid: String,
    pub person: Option<Person>,
    pub base: Option<Person>,
    /// The card's picture changed on the phone; `None`: left as Apple has it.
    pub photo: Option<PhotoEdit>,
}

/// One label (an iCloud list) changed on the phone.
#[derive(uniffi::Enum)]
pub enum PhoneList {
    /// Made on the phone, under the id the phone chose.
    Create {
        guid: String,
        name: String,
    },
    Rename {
        guid: String,
        name: String,
    },
    Delete {
        guid: String,
        name: String,
    },
}

/// Where a phone change is: gone to Apple, still waiting for it (Apple out of reach or busy),
/// or refused by Apple and kept for the person (see [`Account::outbox_refused`]).
#[derive(uniffi::Enum, Debug, PartialEq, Eq)]
pub enum SendState {
    Sent,
    Waiting,
    Refused { reason: String },
}

/// Where the change to one record is now.
#[derive(uniffi::Record, Debug)]
pub struct Settled {
    pub guid: String,
    pub state: SendState,
}

/// What the address book read came to: unchanged since the last read kept, or read again whole.
#[derive(uniffi::Enum)]
pub enum BookRead {
    Unchanged,
    /// Read: written into the phone, then `token` handed back ([`Account::contacts_kept`]) so
    /// the next pass reads it again only once it changed.
    Read {
        book: Book,
        token: String,
    },
}

/// The account as the person knows it (see [`Account::account_profile`]).
#[derive(uniffi::Record)]
pub struct AccountProfile {
    pub name: Option<String>,
    pub email: Option<String>,
    /// Apple's id for the person's own card: the raw contact of that id holds their picture.
    pub me_card: Option<String>,
}

/// The phone's name for a card, for a notice about it.
fn name_of(p: &Person) -> String {
    let parts: Vec<&str> = [&p.first_name, &p.last_name]
        .into_iter()
        .filter_map(|x| x.as_deref())
        .filter(|x| !x.trim().is_empty())
        .collect();
    if parts.is_empty() {
        p.company.clone().unwrap_or_default()
    } else {
        parts.join(" ")
    }
}

/// A card as the engine's, under `guid`, with the phone's picture change in it. The base an
/// edit is made against holds the picture key the phone last had, so a picture chosen or taken
/// away on the phone is exactly that change, and no change leaves Apple's picture as it is.
fn card_with_photo(
    p: &Person,
    guid: &str,
    photo: Option<&PhotoEdit>,
) -> quince_icloud::Result<Card> {
    let contact = crate::with_photo(crate::contact_of(p, guid), photo)?;
    Ok(crate::card_of(contact, p))
}

fn base_card(b: &Person, guid: &str, photo: Option<&PhotoEdit>) -> Card {
    let mut contact = crate::contact_of(b, guid);
    if photo.is_some() {
        contact.photo_url = b.photo.as_ref().map(|r| r.key.clone());
    }
    crate::card_of(contact, b)
}

/// Queue one card's change as the phone has it now. A change that can't be made into one (a
/// picture that can't be read) is refused at once, with why.
fn queue_card(l: &mut quince_core::Library, c: &PhoneCard) -> Result<(), String> {
    let store = |e: quince_core::Error| e.to_string();
    match c.kind {
        ChangeKind::Delete => {
            // As it last was, for the person to see should Apple refuse it.
            let blank = blank();
            let was = c.base.as_ref().or(c.person.as_ref()).unwrap_or(&blank);
            let contact = crate::contact_of(was, &c.guid);
            shared::queue_phone_delete(l, &contact).map_err(store)?;
        }
        ChangeKind::Create | ChangeKind::Update => {
            let Some(now) = c.person.as_ref() else {
                return Err("This contact has nothing in it.".into());
            };
            let create = c.kind == ChangeKind::Create;
            let base = c.base.as_ref().filter(|_| !create);
            // The kinds the phone never shows, as the base had them (crate::keep_unmodelled).
            let now = crate::keep_unmodelled(now, base);
            let mine =
                card_with_photo(&now, &c.guid, c.photo.as_ref()).map_err(|e| e.to_string())?;
            let from = base.map(|b| base_card(b, &c.guid, c.photo.as_ref()));
            shared::queue_phone_card(l, &mine, from.as_ref(), create).map_err(store)?;
            let name = name_of(&now);
            // Addresses: against the card's as last read; a base kept before addresses were
            // (no list in it) can't say what changed, so they are left as Apple has them.
            let base_addresses: Option<Vec<PostalAddress>> = if create {
                Some(Vec::new())
            } else {
                base.and_then(|b| b.addresses.as_ref())
                    .map(|a| a.iter().map(PostalAddress::from).collect())
            };
            if let (Some(base_addresses), Some(now_addresses)) =
                (base_addresses, now.addresses.as_ref())
            {
                let phone = phone_addresses(now_addresses, &base_addresses);
                shared::queue_phone_addresses(l, &c.guid, &base_addresses, &phone, &name)
                    .map_err(store)?;
            }
            let base_groups = base.map(|b| b.groups.clone()).unwrap_or_default();
            shared::queue_phone_lists(l, &c.guid, &base_groups, &now.groups, &name)
                .map_err(store)?;
        }
    }
    Ok(())
}

pub(crate) fn blank() -> Person {
    Person {
        guid: String::new(),
        etag: None,
        first_name: None,
        middle_name: None,
        last_name: None,
        company: None,
        is_company: false,
        birthday: None,
        notes: None,
        methods: Vec::new(),
        prefix: None,
        suffix: None,
        nickname: None,
        phonetic_first_name: None,
        phonetic_last_name: None,
        job_title: None,
        department: None,
        groups: Vec::new(),
        photo: None,
        addresses: None,
    }
}

fn queue_list(l: &mut quince_core::Library, change: &PhoneList) -> quince_core::Result<bool> {
    match change {
        PhoneList::Create { guid, name } => shared::queue_phone_list(l, guid, name, true),
        PhoneList::Rename { guid, name } => shared::queue_phone_list(l, guid, name, false),
        PhoneList::Delete { guid, name } => shared::queue_phone_list_delete(l, guid, name),
    }
}

fn list_guid(change: &PhoneList) -> &str {
    match change {
        PhoneList::Create { guid, .. }
        | PhoneList::Rename { guid, .. }
        | PhoneList::Delete { guid, .. } => guid,
    }
}

/// Where each record's changes are after a run: still queued is waiting; refused in this run
/// is refused, with Apple's reason (or the reason it could not be queued); otherwise sent.
fn settle(
    guids: impl IntoIterator<Item = String>,
    pending: &[quince_core::outbox::Pending],
    report: &Report,
    unqueued: &HashMap<String, String>,
) -> Vec<Settled> {
    guids
        .into_iter()
        .map(|guid| {
            let state = if let Some(reason) = unqueued.get(&guid) {
                SendState::Refused {
                    reason: reason.clone(),
                }
            } else if pending.iter().any(|p| p.record == guid) {
                SendState::Waiting
            } else if let Some((_, reason)) = report.refused.iter().rev().find(|(r, _)| *r == guid)
            {
                SendState::Refused {
                    reason: reason.clone(),
                }
            } else {
                SendState::Sent
            };
            Settled { guid, state }
        })
        .collect()
}

/// The book as the phone takes it: each person with their lists and addresses.
fn book_of(book: quince_icloud::contacts::AddressBook) -> Book {
    let me = book.me.clone().filter(|g| !g.is_empty());
    let mut details: HashMap<String, ContactDetails> = book.details.into_iter().collect();
    let mut addresses: HashMap<String, Vec<PostalAddress>> = book.addresses.into_iter().collect();
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for list in &book.lists {
        for member in &list.members {
            groups
                .entry(member.clone())
                .or_default()
                .push(list.guid.clone());
        }
    }
    let lists = book
        .lists
        .iter()
        .map(|l| CList {
            guid: l.guid.clone(),
            name: l.name.clone(),
            etag: l.etag.clone(),
        })
        .collect();
    let people = book
        .contacts
        .into_iter()
        .map(|p| {
            let d = details.remove(&p.guid).unwrap_or_default();
            let in_lists = groups.remove(&p.guid).unwrap_or_default();
            let at = addresses.remove(&p.guid).unwrap_or_default();
            Person {
                guid: p.guid,
                etag: p.etag,
                first_name: p.first_name,
                middle_name: p.middle_name,
                last_name: p.last_name,
                company: p.company,
                is_company: p.is_company,
                birthday: p
                    .birthday
                    .map(|b| quince_icloud::occasions::birthday_for_phone(&b)),
                notes: p.notes,
                // Addresses go part by part (`addresses`), not as one line of text.
                methods: p
                    .methods
                    .into_iter()
                    .filter(|m| m.kind != "address")
                    .map(|m| Method {
                        kind: m.kind,
                        label: m.label,
                        value: m.value,
                    })
                    .collect(),
                photo: p.photo_url.map(|url| PhotoRef {
                    key: p.photo_signature.unwrap_or_else(|| url.clone()),
                    url,
                    crop: p.photo_crop,
                }),
                prefix: d.prefix,
                suffix: d.suffix,
                nickname: d.nickname,
                phonetic_first_name: d.phonetic_first_name,
                phonetic_last_name: d.phonetic_last_name,
                job_title: d.job_title,
                department: d.department,
                groups: in_lists,
                addresses: Some(at.into_iter().map(Address::from).collect()),
            }
        })
        .collect();
    Book { people, lists, me }
}

impl Account {
    fn contacts_service(&self) -> Result<crate::Service, EngineError> {
        self.service("contacts")
            .ok_or_else(|| failed("Apple hasn't offered a contacts service for this account."))
    }
}

#[uniffi::export]
impl Account {
    /// The phone's changes to contacts sent to iCloud, as the desktop sends its own: labels made,
    /// renamed or deleted first, then each card (its fields, addresses and labels), all queued
    /// in the engine's outbox and sent from one read of the book. Where each label and card is
    /// now, by its id, in the order given.
    pub fn contacts_send(
        &self,
        cards: Vec<PhoneCard>,
        lists: Vec<PhoneList>,
    ) -> Result<Vec<Settled>, EngineError> {
        let (lock, account) = self.library()?;
        let mut unqueued = HashMap::new();
        runtime().block_on(async {
            let mut l = lock.lock().await;
            l.atomically(|l| {
                for change in &lists {
                    queue_list(l, change)?;
                }
                for card in &cards {
                    if let Err(why) = queue_card(l, card) {
                        unqueued.insert(card.guid.clone(), why);
                    }
                }
                Ok::<_, quince_core::Error>(())
            })
            .map_err(failed)
        })?;
        let report = match self.contacts_service() {
            Err(_) => Report {
                outcome: quince_sync::queue::Outcome::Later,
                sent: false,
                refused: Vec::new(),
            },
            Ok((http, root, params)) => runtime().block_on(async {
                let book = std::sync::Mutex::new(shared::Book::default());
                let ledger = quince_icloud::contacts::Ledger::new();
                quince_sync::queue::drain_as(&lock, CONTACTS, &PASS, &account, |p| {
                    shared::send(&http, &root, params.clone(), p, &book, &ledger)
                })
                .await
            }),
        };
        for (_, why) in &report.refused {
            crate::log::error(&crate::log::redact(why));
        }
        let pending = runtime()
            .block_on(async { lock.lock().await.outbox_pending(CONTACTS) })
            .map_err(failed)?;
        let guids = lists
            .iter()
            .map(|c| list_guid(c).to_string())
            .chain(cards.iter().map(|c| c.guid.clone()));
        Ok(settle(guids, &pending, &report, &unqueued))
    }

    /// iCloud's address book, read only when it changed since the read last kept
    /// ([`Self::contacts_kept`]); `full` reads it whatever the tokens say (the phone has rows to
    /// write again, or nothing read yet).
    pub fn contacts_read(&self, full: bool) -> Result<BookRead, EngineError> {
        use quince_icloud::contacts::{Pass, Tokens};
        let (lock, _) = self.library()?;
        let (http, root, params) = self.contacts_service()?;
        runtime().block_on(async {
            let _pass = PASS.lock().await;
            let since = if full {
                None
            } else {
                lock.lock()
                    .await
                    .sync_token(CONTACTS)
                    .map_err(failed)?
                    .and_then(|t| Tokens::from_stored(&t))
            };
            match quince_icloud::contacts::pass(&http, &root, params, since)
                .await
                .map_err(failed)?
            {
                Pass::Unchanged(tokens) => {
                    lock.lock()
                        .await
                        .mark_synced(CONTACTS, Some(&tokens.stored()))
                        .map_err(failed)?;
                    Ok(BookRead::Unchanged)
                }
                Pass::Read(book, tokens) => {
                    let book = book_of(book);
                    // Which card is the person's own, kept as the desktop keeps it, so the
                    // account's picture is known on a pass that finds the book unchanged.
                    lock.lock()
                        .await
                        .set_state(ME_KEY, book.me.as_deref().unwrap_or_default())
                        .map_err(failed)?;
                    Ok(BookRead::Read {
                        book,
                        token: tokens.stored(),
                    })
                }
            }
        })
    }

    /// The signed-in account as the person knows it, as the desktop's sidebar shows it: the name
    /// and address Apple gave at sign-in, and the card they have said is their own (its picture
    /// is the account's), once Contacts has been read.
    pub fn account_profile(&self) -> AccountProfile {
        let (name, email) = {
            let c = self.client();
            let s = c.session();
            (s.account_name.clone(), s.account_email.clone())
        };
        let me_card = self.library().ok().and_then(|(lock, _)| {
            runtime()
                .block_on(async { lock.lock().await.state(ME_KEY) })
                .ok()
                .flatten()
                .filter(|g| !g.is_empty())
        });
        AccountProfile {
            name,
            email,
            me_card,
        }
    }

    /// The book read with `token` is written into the phone: the next pass reads it again only
    /// once Apple says it changed.
    pub fn contacts_kept(&self, token: String) -> Result<(), EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async {
            lock.lock()
                .await
                .mark_synced(CONTACTS, Some(&token))
                .map_err(failed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(first: &str) -> Person {
        Person {
            guid: "G".into(),
            first_name: Some(first.into()),
            ..blank()
        }
    }

    fn at(city: &str) -> Address {
        Address {
            label: Some("HOME".into()),
            street: Some("1 Example Road".into()),
            sub_locality: None,
            city: Some(city.into()),
            sub_administrative_area: None,
            state: None,
            postal_code: None,
            country: Some("India".into()),
            country_code: None,
            formatted: String::new(),
        }
    }

    /// A card edited on the phone is queued as the desktop queues a form's save, its address
    /// change part by part and its labels as joined and left — and queued again unchanged on
    /// the next pass, nothing more is queued.
    #[test]
    fn a_phone_card_is_queued_with_its_addresses_and_labels_once() {
        let mut l = quince_core::Library::in_memory().unwrap();
        let mut base = person("Ada");
        base.addresses = Some(vec![at("Pune")]);
        base.groups = vec!["L1".into()];
        let mut now = person("Ada L");
        now.addresses = Some(vec![at("Mumbai")]);
        now.groups = vec!["L2".into()];
        let change = PhoneCard {
            kind: ChangeKind::Update,
            guid: "G".into(),
            person: Some(now),
            base: Some(base),
            photo: None,
        };
        queue_card(&mut l, &change).unwrap();
        let kinds = |l: &quince_core::Library| -> Vec<String> {
            l.outbox_pending(CONTACTS)
                .unwrap()
                .into_iter()
                .map(|p| p.kind)
                .collect()
        };
        assert_eq!(kinds(&l), ["save", "address", "lists"]);
        queue_card(&mut l, &change).unwrap();
        assert_eq!(
            kinds(&l),
            ["save", "address", "lists"],
            "asked again: nothing new"
        );
    }

    /// A birthday without a year reaches the phone as Android's `--MM-DD` (not 1604, shown as
    /// an age of four hundred), and goes back as Apple's 1604; addresses arrive part by part.
    #[test]
    fn a_yearless_birthday_is_androids_on_the_phone_and_apples_in_icloud() {
        let mut card = crate::contact_of(&blank(), "G");
        card.first_name = Some("Ada".into());
        card.birthday = Some("1604-03-17".into());
        let book = book_of(quince_icloud::contacts::AddressBook {
            contacts: vec![card],
            lists: vec![],
            me: Some("G".into()),
            addresses: vec![(
                "G".into(),
                vec![PostalAddress {
                    city: Some("Pune".into()),
                    ..PostalAddress::default()
                }],
            )],
            details: vec![],
        });
        assert_eq!(
            book.me.as_deref(),
            Some("G"),
            "the person's own card, for their picture"
        );
        let p = &book.people[0];
        assert_eq!(p.birthday.as_deref(), Some("--03-17"));
        assert_eq!(
            p.addresses.as_ref().unwrap()[0].city.as_deref(),
            Some("Pune")
        );
        assert_eq!(
            crate::contact_of(p, "G").birthday.as_deref(),
            Some("1604-03-17")
        );
    }

    /// A base kept before addresses were (none in it) can't say what changed: the card goes,
    /// its addresses are left as Apple has them.
    #[test]
    fn an_old_base_leaves_the_addresses_alone() {
        let mut l = quince_core::Library::in_memory().unwrap();
        let mut now = person("Ada L");
        now.addresses = Some(vec![at("Mumbai")]);
        let change = PhoneCard {
            kind: ChangeKind::Update,
            guid: "G".into(),
            person: Some(now),
            base: Some(person("Ada")),
            photo: None,
        };
        queue_card(&mut l, &change).unwrap();
        let p = l.outbox_pending(CONTACTS).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].kind, "save");
    }

    /// Where each change is after a run, by its record: queued still, refused in the run, or
    /// sent; one that couldn't be queued is refused with why.
    #[test]
    fn each_record_is_settled_by_what_the_queue_holds() {
        let pending = vec![quince_core::outbox::Pending {
            id: 1,
            service: CONTACTS.into(),
            record: "W".into(),
            kind: "save".into(),
            payload: String::new(),
            tries: 1,
        }];
        let report = Report {
            outcome: quince_sync::queue::Outcome::Later,
            sent: true,
            refused: vec![("R".into(), "deleted elsewhere".into())],
        };
        let unqueued = HashMap::from([("U".to_string(), "no picture".to_string())]);
        let got = settle(
            ["W", "R", "S", "U"].map(String::from),
            &pending,
            &report,
            &unqueued,
        );
        let states: Vec<&SendState> = got.iter().map(|s| &s.state).collect();
        assert_eq!(
            states,
            [
                &SendState::Waiting,
                &SendState::Refused {
                    reason: "deleted elsewhere".into()
                },
                &SendState::Sent,
                &SendState::Refused {
                    reason: "no picture".into()
                },
            ]
        );
    }
}
