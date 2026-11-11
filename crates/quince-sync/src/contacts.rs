//! Contacts' changes made here: recorded, laid over the library and sent, by the code the
//! desktop's outbox worker and the phone's engine both run, so a fix to how a card is sent is a
//! fix on both.
//!
//! The desktop records a change and lays it over its library in one transaction
//! ([`record_card_save`] and its kin): every view shows it at once, and each sync pass lays what
//! still waits back over what it read ([`replay`]). The phone's change is already shown — it was
//! made in Android's own Contacts — so the phone only queues it ([`queue_phone_card`] and its
//! kin), as the state its row is in now: queued again on every pass until Apple has it, which is
//! no change while the queue already says exactly that.
//!
//! Sending ([`send`]) is one: the address book read once for a run of changes, not once for each
//! ([`Book`]); a create under the name it was given here, made at most once; an edit as only what
//! changed from its base, laid over Apple's card as it is now; a delete against the card as it
//! is now; an address edit part by part; a list change laid over the list as Apple has it.

use quince_core::contacts::{Card, Contact, ContactDetails, PostalAddress};
use quince_core::outbox::Pending;
use quince_core::Library;
use quince_icloud::contacts::address::{self as postal, AddressEdit};
use quince_icloud::contacts::{ContactList, Ledger};
use quince_icloud::Error;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use crate::queue::{json, last_word, unreadable};

pub const CONTACTS: &str = "contacts";

/// A card saved here: as the form left it, and as it opened (`None` for a new card).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactSave {
    pub contact: Contact,
    pub base: Option<Contact>,
    pub create: bool,
    /// The rest of the name and work, from a form that shows them; `None` (a change queued
    /// before there was one) leaves Apple's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ContactDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_details: Option<ContactDetails>,
}

/// A card's addresses edited here: the edits, and the list the form opened them from. Sent,
/// they are laid over Apple's addresses as they stand then (`contacts::address::edit`), so a
/// send that already landed lands again as no change. `name` is who the card is, for a notice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactAddress {
    pub base: Vec<PostalAddress>,
    pub edits: Vec<AddressEdit>,
    #[serde(default)]
    pub name: String,
}

/// A card deleted here, as it was.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactDelete {
    pub contact: Contact,
}

/// The lists a card is in, changed here: the lists it was in (`base`) and is in now, by the
/// lists' ids. Sent, the card is added to each list it joined and taken out of each it left,
/// laid over the list as Apple has it then, so a member added on another device stays.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CardLists {
    pub base: Vec<String>,
    pub now: Vec<String>,
    #[serde(default)]
    pub name: String,
}

/// A list made or renamed here (`create` for one made here, under the id it was given here).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListSave {
    pub name: String,
    pub create: bool,
}

/// A list deleted here; its people stay in Contacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListDelete {
    #[serde(default)]
    pub name: String,
}

/// The kinds a list's own changes are queued as (the record is the list's id).
pub const LIST_SAVE: &str = "list";
pub const LIST_DELETE: &str = "list-delete";
/// The kind a card's lists are queued as (the record is the card's id).
pub const CARD_LISTS: &str = "lists";

fn io(e: impl std::fmt::Display) -> quince_core::Error {
    quince_core::Error::Io(std::io::Error::other(e.to_string()))
}

// --- Recording: the desktop's commands. Each records the change and lays it over the library
// as one transaction, so a crash leaves both or neither.

/// A card saved here — made (no id yet: one is given now) or edited. Answers its id. The
/// shape a save had before the form showed the rest of the name and work: kept for the tests
/// that queue one, as a window from before would have.
pub fn record_contact_save(
    library: &mut Library,
    contact: &Contact,
    base: Option<Contact>,
) -> quince_core::Result<String> {
    record_save(library, contact, None, base.map(|b| (b, None)))
}

/// [`record_contact_save`] for a whole card: the rest of its name and work go too.
pub fn record_card_save(
    library: &mut Library,
    card: &Card,
    base: Option<Card>,
) -> quince_core::Result<String> {
    record_save(
        library,
        &card.contact,
        Some(&card.details),
        base.map(|b| (b.contact, Some(b.details))),
    )
}

fn record_save(
    library: &mut Library,
    contact: &Contact,
    details: Option<&ContactDetails>,
    base: Option<(Contact, Option<ContactDetails>)>,
) -> quince_core::Result<String> {
    let create = contact.guid.is_empty();
    let mut contact = contact.clone();
    if create {
        contact.guid = quince_icloud::contacts::new_contact_id();
        contact.etag = None;
    }
    let guid = contact.guid.clone();
    let details = details.cloned();
    library.atomically(|l| {
        let waiting = last_word(l, CONTACTS, &guid, "save")?.and_then(|p| {
            serde_json::from_str::<ContactSave>(&p.payload)
                .ok()
                .map(|s| (p, s))
        });
        match waiting {
            Some((p, before)) if p.tries == 0 => {
                // One save, from the card as the form first opened it (or still a create).
                let save = ContactSave {
                    contact: contact.clone(),
                    base: before.base,
                    create: before.create,
                    details: details.clone().or(before.details),
                    base_details: before.base_details,
                };
                if !l.outbox_amend(p.id, &json(&save)?)? {
                    l.outbox_append(CONTACTS, &guid, "save", &json(&save)?)?;
                }
            }
            Some((_, before)) => {
                // Behind one that may have landed: an edit of the card that one sends.
                let save = ContactSave {
                    contact: contact.clone(),
                    base: Some(before.contact),
                    create: false,
                    base_details: before.details.clone(),
                    details: details.clone().or(before.details),
                };
                l.outbox_append(CONTACTS, &guid, "save", &json(&save)?)?;
            }
            None => {
                let (base, base_details) = match (&base, create) {
                    (Some((b, d)), false) => (Some(b.clone()), d.clone()),
                    _ => (None, None),
                };
                let save = ContactSave {
                    contact: contact.clone(),
                    base,
                    create,
                    details: details.clone(),
                    base_details,
                };
                l.outbox_append(CONTACTS, &guid, "save", &json(&save)?)?;
            }
        }
        lay_contact_save(l, &contact, details.as_ref())?;
        Ok(guid.clone())
    })
}

fn lay_contact_save(
    l: &mut Library,
    contact: &Contact,
    details: Option<&ContactDetails>,
) -> quince_core::Result<()> {
    l.put_contact(contact)?;
    if let Some(d) = details {
        l.put_contact_details(&[(contact.guid.clone(), d.clone())], false)?;
    }
    Ok(())
}

/// A card deleted here: gone at once, deleted at Apple behind.
pub fn record_contact_delete(library: &mut Library, contact: &Contact) -> quince_core::Result<()> {
    library.atomically(|l| {
        let d = ContactDelete {
            contact: contact.clone(),
        };
        l.outbox_append(CONTACTS, &contact.guid, "delete", &json(&d)?)?;
        l.forget_contact(&contact.guid)
    })
}

/// A card's addresses edited: shown at once, sent behind — after the card itself, when it was
/// made here and waits too, since the queue keeps the order changes were made in. Answers
/// the list as it now shows.
pub fn record_contact_address(
    library: &mut Library,
    guid: &str,
    base: Vec<PostalAddress>,
    edits: Vec<AddressEdit>,
) -> quince_core::Result<Vec<PostalAddress>> {
    library.atomically(|l| {
        let current = l.contact_addresses(guid)?;
        let after = postal::laid_over(&current, &base, &edits).map_err(io)?;
        let name = l
            .contacts(None)?
            .into_iter()
            .find(|c| c.guid == guid)
            .map(|c| c.display_name())
            .unwrap_or_default();
        let change = ContactAddress { base, edits, name };
        l.outbox_append(CONTACTS, guid, "address", &json(&change)?)?;
        l.set_contact_addresses(guid, &after)?;
        Ok(after)
    })
}

fn lay_contact_address(l: &mut Library, guid: &str, a: &ContactAddress) -> quince_core::Result<()> {
    // A card deleted here, or gone at Apple, has nowhere to show them.
    if !l.contacts(None)?.iter().any(|c| c.guid == guid) {
        return Ok(());
    }
    let current = l.contact_addresses(guid)?;
    if let Ok(after) = postal::laid_over(&current, &a.base, &a.edits) {
        l.set_contact_addresses(guid, &after)?;
    }
    Ok(())
}

/// Lay one waiting change to a card over the library. Kinds this module doesn't lay (a merge,
/// which the desktop lays itself; a list change, which the library shows from Apple's lists)
/// are left alone.
pub fn lay(l: &mut Library, p: &Pending) -> quince_core::Result<()> {
    match p.kind.as_str() {
        "save" => {
            if let Ok(s) = serde_json::from_str::<ContactSave>(&p.payload) {
                lay_contact_save(l, &s.contact, s.details.as_ref())?;
            }
        }
        "delete" => l.forget_contact(&p.record)?,
        "address" => {
            if let Ok(a) = serde_json::from_str::<ContactAddress>(&p.payload) {
                lay_contact_address(l, &p.record, &a)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Lay every change still waiting for Contacts over what a sync pass just stored, in the order
/// they were made.
pub fn replay(library: &mut Library) -> quince_core::Result<()> {
    for p in library.outbox_pending(CONTACTS)? {
        lay(library, &p)?;
    }
    Ok(())
}

// --- Queueing the phone's state: its rows are the change already shown, so nothing is laid.
// Each is asked again on every pass while the row waits; asked for exactly what already waits,
// it queues nothing.

/// The newest change of `kind` still waiting for `record`, wherever it stands in the queue,
/// with its payload read; and whether it is the last word on the record (nothing of another
/// kind after it), the only place a change can be amended without moving it past another.
fn newest<T: serde::de::DeserializeOwned>(
    l: &Library,
    record: &str,
    kind: &str,
) -> quince_core::Result<Option<(Pending, T, bool)>> {
    let Some(p) = l.outbox_waiting(CONTACTS, record, kind)?.pop() else {
        return Ok(None);
    };
    let last = last_word(l, CONTACTS, record, kind)?.is_some_and(|w| w.id == p.id);
    Ok(serde_json::from_str::<T>(&p.payload)
        .ok()
        .map(|t| (p, t, last)))
}

/// A card as it is on the phone now (`create` for one made there, under the id the phone gave
/// it), against the card as last read from Apple (`base`). Whether anything was queued.
///
/// What waits already is followed, never resent from the base: unsent and the last word, it is
/// rewritten to the card as it is now (one save, from the first base); on its way, or behind
/// another change to the card, this one goes after it, made from what it sends — so a field
/// changed and then put back is put back at Apple too.
pub fn queue_phone_card(
    l: &mut Library,
    card: &Card,
    base: Option<&Card>,
    create: bool,
) -> quince_core::Result<bool> {
    let guid = card.contact.guid.clone();
    let save = match newest::<ContactSave>(l, &guid, "save")? {
        Some((_, before, _))
            if before.contact == card.contact && before.details.as_ref() == Some(&card.details) =>
        {
            return Ok(false);
        }
        Some((p, before, true)) if p.tries == 0 => {
            let save = ContactSave {
                contact: card.contact.clone(),
                details: Some(card.details.clone()),
                ..before
            };
            if l.outbox_amend(p.id, &json(&save)?)? {
                return Ok(true);
            }
            save
        }
        Some((_, before, _)) => ContactSave {
            contact: card.contact.clone(),
            base: Some(before.contact),
            create: false,
            details: Some(card.details.clone()),
            base_details: before.details,
        },
        None => ContactSave {
            contact: card.contact.clone(),
            base: base.filter(|_| !create).map(|b| b.contact.clone()),
            create,
            details: Some(card.details.clone()),
            base_details: base.filter(|_| !create).map(|b| b.details.clone()),
        },
    };
    l.outbox_append(CONTACTS, &guid, "save", &json(&save)?)?;
    Ok(true)
}

/// A card deleted on the phone, as it last was. Queued once.
pub fn queue_phone_delete(l: &mut Library, contact: &Contact) -> quince_core::Result<bool> {
    if last_word(l, CONTACTS, &contact.guid, "delete")?.is_some() {
        return Ok(false);
    }
    let d = ContactDelete {
        contact: contact.clone(),
    };
    l.outbox_append(CONTACTS, &contact.guid, "delete", &json(&d)?)?;
    Ok(true)
}

/// A card's addresses as they are on the phone now, against the addresses as last read from
/// Apple: sent as the edits that make one the other ([`address_edits`]), part by part, so a
/// part changed on another device meanwhile stays. `name` is who the card is, for a notice.
pub fn queue_phone_addresses(
    l: &mut Library,
    guid: &str,
    base: &[PostalAddress],
    now: &[PostalAddress],
    name: &str,
) -> quince_core::Result<bool> {
    let sent_by =
        |a: &ContactAddress| postal::apply_edits(&a.base, &a.edits).map(|(after, _)| after);
    // The phone's list as a send leaves it — parts trimmed, a country's code from its name, an
    // emptied address gone — so it compares equal with what a waiting edit already sends.
    let now = postal::apply_edits(base, &address_edits(base, now))
        .map_err(io)?
        .0;
    let now = now.as_slice();
    let change = match newest::<ContactAddress>(l, guid, "address")? {
        Some((_, before, _)) if sent_by(&before).ok().as_deref() == Some(now) => return Ok(false),
        Some((p, before, true)) if p.tries == 0 => {
            let edits = address_edits(&before.base, now);
            if edits.is_empty() {
                // Put back as it was before anything went: nothing to send.
                return l.outbox_withdraw(p.id);
            }
            let change = ContactAddress {
                edits,
                name: name.to_string(),
                ..before
            };
            if l.outbox_amend(p.id, &json(&change)?)? {
                return Ok(true);
            }
            change
        }
        Some((_, before, _)) => {
            let from = sent_by(&before).map_err(io)?;
            ContactAddress {
                edits: address_edits(&from, now),
                base: from,
                name: name.to_string(),
            }
        }
        None => ContactAddress {
            base: base.to_vec(),
            edits: address_edits(base, now),
            name: name.to_string(),
        },
    };
    if change.edits.is_empty() {
        return Ok(false);
    }
    l.outbox_append(CONTACTS, guid, "address", &json(&change)?)?;
    Ok(true)
}

/// The lists a card is in on the phone now (`now`), against those it was in as last read
/// (`base`), by the lists' ids.
pub fn queue_phone_lists(
    l: &mut Library,
    guid: &str,
    base: &[String],
    now: &[String],
    name: &str,
) -> quince_core::Result<bool> {
    let set = |v: &[String]| v.iter().cloned().collect::<HashSet<_>>();
    let change = match newest::<CardLists>(l, guid, CARD_LISTS)? {
        Some((_, before, _)) if set(&before.now) == set(now) => return Ok(false),
        Some((p, before, true)) if p.tries == 0 => {
            if set(&before.base) == set(now) {
                return l.outbox_withdraw(p.id);
            }
            let change = CardLists {
                now: now.to_vec(),
                name: name.to_string(),
                ..before
            };
            if l.outbox_amend(p.id, &json(&change)?)? {
                return Ok(true);
            }
            change
        }
        Some((_, before, _)) => CardLists {
            base: before.now,
            now: now.to_vec(),
            name: name.to_string(),
        },
        None if set(base) == set(now) => return Ok(false),
        None => CardLists {
            base: base.to_vec(),
            now: now.to_vec(),
            name: name.to_string(),
        },
    };
    l.outbox_append(CONTACTS, guid, CARD_LISTS, &json(&change)?)?;
    Ok(true)
}

/// A list made (`create`, under the id the phone gave it) or renamed on the phone.
pub fn queue_phone_list(
    l: &mut Library,
    guid: &str,
    name: &str,
    create: bool,
) -> quince_core::Result<bool> {
    let save = match newest::<ListSave>(l, guid, LIST_SAVE)? {
        Some((_, before, _)) if before.name == name => return Ok(false),
        Some((p, before, true)) if p.tries == 0 => {
            let save = ListSave {
                name: name.to_string(),
                create: before.create,
            };
            if l.outbox_amend(p.id, &json(&save)?)? {
                return Ok(true);
            }
            save
        }
        _ => ListSave {
            name: name.to_string(),
            create,
        },
    };
    l.outbox_append(CONTACTS, guid, LIST_SAVE, &json(&save)?)?;
    Ok(true)
}

/// A list deleted on the phone. Queued once.
pub fn queue_phone_list_delete(
    l: &mut Library,
    guid: &str,
    name: &str,
) -> quince_core::Result<bool> {
    if last_word(l, CONTACTS, guid, LIST_DELETE)?.is_some() {
        return Ok(false);
    }
    let d = ListDelete {
        name: name.to_string(),
    };
    l.outbox_append(CONTACTS, guid, LIST_DELETE, &json(&d)?)?;
    Ok(true)
}

/// The edits that make `base` into `now`, as a form would make them: each address in both
/// changed part by part (and its label), those `now` has beyond `base` added, those `base` has
/// beyond `now` taken out — from the last, so each index still names what it named. Applied
/// to `base` ([`postal::apply_edits`]) they give `now`.
pub fn address_edits(base: &[PostalAddress], now: &[PostalAddress]) -> Vec<AddressEdit> {
    use quince_core::contacts::ADDRESS_FIELDS;
    let mut edits = Vec::new();
    for (index, (b, n)) in base.iter().zip(now).enumerate() {
        for field in ADDRESS_FIELDS {
            let (was, is) = (
                b.field(field).and_then(Option::as_ref),
                n.field(field).and_then(Option::as_ref),
            );
            // A country changed by name takes its code along (`apply_edits`): its code is
            // never sent as well, or the code the name brings would be written over.
            if field == "countryCode" && b.country != n.country {
                continue;
            }
            if was != is {
                edits.push(AddressEdit::SetField {
                    index,
                    field: field.to_string(),
                    value: is.cloned(),
                });
            }
        }
        if b.label != n.label {
            edits.push(AddressEdit::SetLabel {
                index,
                label: n.label.clone(),
            });
        }
    }
    for address in now.iter().skip(base.len()) {
        edits.push(AddressEdit::Add {
            address: address.clone(),
        });
    }
    for index in (now.len()..base.len()).rev() {
        edits.push(AddressEdit::Remove { index });
    }
    edits
}

// --- Sending.

/// The address book a run of queued contact changes is sent from: read once for the run, not
/// once for each change — every read is the whole book, every card in it. A card (or list) the
/// run has written since is read again before it is written again (the copy here is from
/// before); so is the book when the last write's answer left no tokens to carry on with (see
/// [`quince_icloud::contacts::Ledger`]); and a write Apple says was made against an old book
/// is sent once more from a fresh read.
#[derive(Default)]
pub struct Book {
    read: Option<Arc<BookRead>>,
    written: HashSet<String>,
}

/// One read of the book: each card as Apple wrote it, by id, and the lists.
pub struct BookRead {
    pub cards: HashMap<String, serde_json::Value>,
    pub lists: Vec<ContactList>,
}

impl Book {
    /// The book to send a change to `records` from, read now if the run has none it can use —
    /// none read yet, one of `records` written since, or no live tokens left to write with;
    /// and whether it was read just now. A read hands its tokens to `ledger`.
    pub async fn for_records(
        book: &Mutex<Self>,
        ledger: &Ledger,
        http: &reqwest::Client,
        root: &str,
        params: &[(String, String)],
        records: &[&str],
    ) -> Result<(Arc<BookRead>, bool), Error> {
        let kept = {
            let b = book.lock().unwrap_or_else(PoisonError::into_inner);
            b.read
                .clone()
                .filter(|_| !records.iter().any(|r| b.written.contains(*r)) && ledger.has_tokens())
        };
        if let Some(read) = kept {
            return Ok((read, false));
        }
        let (cards, lists, tokens) =
            quince_icloud::contacts::cards_and_lists_with_tokens(http, root, params.to_vec())
                .await?;
        ledger.hold(tokens);
        let cards = cards
            .into_iter()
            .filter_map(|card| {
                let id = card.get("contactId")?.as_str()?.to_string();
                Some((id, card))
            })
            .collect();
        let mut b = book.lock().unwrap_or_else(PoisonError::into_inner);
        let read = Arc::new(BookRead { cards, lists });
        b.read = Some(Arc::clone(&read));
        b.written.clear();
        Ok((read, true))
    }

    /// [`Self::for_records`] for one record.
    pub async fn for_record(
        book: &Mutex<Self>,
        ledger: &Ledger,
        http: &reqwest::Client,
        root: &str,
        params: &[(String, String)],
        record: &str,
    ) -> Result<(Arc<BookRead>, bool), Error> {
        Self::for_records(book, ledger, http, root, params, &[record]).await
    }

    /// `record` written: its copy in the read is old now.
    pub fn wrote(book: &Mutex<Self>, record: &str) {
        book.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .written
            .insert(record.to_string());
    }

    /// The read let go of: written past it in ways it cannot track (a merge).
    pub fn forget(book: &Mutex<Self>) {
        book.lock().unwrap_or_else(PoisonError::into_inner).read = None;
    }
}

/// One queued change to Contacts sent. `book` and `ledger` are the run's: one read of the book
/// for all its changes, and the tokens each write carries.
pub async fn send(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
    book: &Mutex<Book>,
    ledger: &Ledger,
) -> Result<(), Error> {
    use quince_icloud::contacts as c;
    match p.kind.as_str() {
        "address" => {
            let a: ContactAddress = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            Book::wrote(book, &p.record);
            // It reads the card and its tokens itself: the run's tokens are spent by its write.
            let edited = c::address::edit(http, root, params, &p.record, &a.base, &a.edits).await;
            ledger.spend();
            return edited.map(|_| ());
        }
        CARD_LISTS => return send_card_lists(http, root, params, &p, book, ledger).await,
        LIST_SAVE | LIST_DELETE => return send_list(http, root, params, &p, book, ledger).await,
        "save" | "delete" => {}
        other => {
            return Err(Error::Refused(format!(
                "Skykeep does not know how to send “{other}”."
            )))
        }
    }
    let (read, fresh) = Book::for_record(book, ledger, http, root, &params, &p.record).await?;
    let sent = send_from(http, root, params.clone(), &p, &read, ledger).await;
    if matches!(sent, Err(Error::Conflict)) && !fresh {
        // Made against a book read earlier in the run, which Apple says is old now: from a
        // fresh read, once.
        Book::forget(book);
        let (read, _) = Book::for_record(book, ledger, http, root, &params, &p.record).await?;
        let again = send_from(http, root, params, &p, &read, ledger).await;
        Book::wrote(book, &p.record);
        return again;
    }
    Book::wrote(book, &p.record);
    sent
}

/// One queued card change sent, from `read`: Apple's card as it was read, with the tokens
/// `ledger` holds.
async fn send_from(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: &Pending,
    read: &BookRead,
    ledger: &Ledger,
) -> Result<(), Error> {
    use quince_icloud::contacts as c;
    let card = read.cards.get(&p.record);
    if p.kind == "delete" {
        // Gone already — deleted elsewhere, or by an earlier send of this one. Otherwise
        // against the version read: changed since, Apple refuses it as a conflict.
        let Some(card) = card else {
            return Ok(());
        };
        return c::merge::delete_card(http, root, params, ledger, card).await;
    }
    let s: ContactSave = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
    if let Some(details) = &s.details {
        let whole = |contact: &Contact, details: &ContactDetails| Card {
            contact: contact.clone(),
            details: details.clone(),
        };
        let mine = whole(&s.contact, details);
        if s.create {
            // Already made — an earlier send of this one — is done.
            if card.is_some() {
                return Ok(());
            }
            return c::create_card_named(http, root, params, ledger, &p.record, &mine).await;
        }
        // A base from before the form showed these has none: compared with Apple's own, so
        // only what this form changed is written.
        let base_details = match &s.base_details {
            Some(d) => Some(d.clone()),
            None => card.map(|v| {
                c::parse_cards(&serde_json::json!({ "contacts": [v] }))
                    .pop()
                    .map(|c| c.details)
                    .unwrap_or_default()
            }),
        };
        let base = s
            .base
            .as_ref()
            .map(|b| whole(b, &base_details.clone().unwrap_or_default()));
        return c::update_card(http, root, params, ledger, &mine, card, base.as_ref()).await;
    }
    if s.create {
        if card.is_some() {
            return Ok(());
        }
        return c::create_named(http, root, params, ledger, &p.record, &s.contact).await;
    }
    c::update_from(
        http,
        root,
        params,
        ledger,
        &s.contact,
        card,
        s.base.as_ref(),
    )
    .await
}

/// A card's lists sent: the card added to each list it joined and taken out of each it left,
/// each laid over the list as Apple has it now. A list Apple no longer has (deleted on another
/// device, or a list made here that Apple turned down) has nothing to change.
async fn send_card_lists(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: &Pending,
    book: &Mutex<Book>,
    ledger: &Ledger,
) -> Result<(), Error> {
    let change: CardLists = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
    let joined: Vec<&String> = change
        .now
        .iter()
        .filter(|g| !change.base.contains(g))
        .collect();
    let left: Vec<&String> = change
        .base
        .iter()
        .filter(|g| !change.now.contains(g))
        .collect();
    let touched: Vec<&str> = joined.iter().chain(&left).map(|g| g.as_str()).collect();
    if touched.is_empty() {
        return Ok(());
    }
    let (read, _) = Book::for_records(book, ledger, http, root, &params, &touched).await?;
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    for guid in touched {
        let Some(mut list) = read.lists.iter().find(|l| same(&l.guid, guid)).cloned() else {
            continue;
        };
        let had = list.members.clone();
        if joined.iter().any(|g| same(g, guid)) {
            if !list.members.iter().any(|m| same(m, &p.record)) {
                list.members.push(p.record.clone());
            }
        } else {
            list.members.retain(|m| !same(m, &p.record));
        }
        if list.members == had {
            continue;
        }
        let written =
            quince_icloud::contacts::update_list(http, root, params.clone(), ledger, &list).await;
        Book::wrote(book, &list.guid);
        written?;
    }
    Ok(())
}

/// A list made, renamed or deleted here, sent against the list as Apple has it now. Made: once
/// (already there, it is only named as asked). Renamed or deleted but gone at Apple: done.
async fn send_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: &Pending,
    book: &Mutex<Book>,
    ledger: &Ledger,
) -> Result<(), Error> {
    use quince_icloud::contacts as c;
    let (read, _) = Book::for_record(book, ledger, http, root, &params, &p.record).await?;
    let there = read
        .lists
        .iter()
        .find(|l| l.guid.eq_ignore_ascii_case(&p.record))
        .cloned();
    let written = if p.kind == LIST_DELETE {
        match there {
            None => return Ok(()),
            Some(list) => c::delete_list(http, root, params, ledger, &list.guid).await,
        }
    } else {
        let s: ListSave = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
        match there {
            Some(list) if list.name == s.name.trim() => return Ok(()),
            Some(mut list) => {
                list.name = s.name.trim().to_string();
                c::update_list(http, root, params, ledger, &list).await
            }
            None if s.create => {
                c::create_list_named(http, root, params, ledger, &p.record, &s.name, &[]).await
            }
            None => return Ok(()),
        }
    };
    Book::wrote(book, &p.record);
    written
}

#[cfg(test)]
mod tests;
