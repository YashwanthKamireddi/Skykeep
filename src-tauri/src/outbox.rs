//! Changes go to Apple behind the window, never in front of it. A command records the change
//! (quince_core::outbox) and lays it over the library in one transaction, then returns: every
//! view shows it at once, as a native app's own store would, with or without a network. This
//! worker sends what is queued, in the order it was made, one service at a time:
//!
//! - taken: off the queue; one sync pass afterwards brings Apple's own version of the record;
//! - Apple can't be reached (offline, signed out, a server fault): kept, and tried again with a
//!   growing wait, so a change made offline lands when the connection comes back — even after
//!   a restart, since the queue is in the library;
//! - refused for good (deleted elsewhere, edited in the same place, turned down): kept in the
//!   library marked refused, with what was typed, for the person to send again or let go;
//!   Apple's version read back over ours, and the window told why — `quince://outbox-refused`.
//!
//! Sent exactly once in effect: every change is safe to send twice. A reminder or card made
//! here carries a name chosen before it is queued, and a create finding that name at Apple
//! already does nothing; edits are merges against what Apple holds (a note's text against the
//! base it was typed over, a reminder's fields, a card's fields against the form's base), so
//! one that already landed lands again as no change.
//!
//! A sync pass that lands while changes wait would put Apple's older version over them, so
//! every pass lays the waiting changes over what it read (see [`replay`]).

use crate::{cloudkit_context, Auth, Data};
use quince_core::calendar::Event;
use quince_core::contacts::{Card, Contact, ContactDetails, PostalAddress};
use quince_core::outbox::Pending;
#[cfg(test)]
use quince_core::reminders::Reminder;
use quince_core::Library;
use quince_icloud::calendar::{EventRepeat, Scope};
use quince_icloud::contacts::address::{self as postal, AddressEdit};
#[cfg(test)]
use quince_icloud::reminders::ReminderEdit;
use quince_icloud::Error;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
#[cfg(test)]
use tokio::sync::Mutex;
use tokio::sync::Notify;

static WAKE: Notify = Notify::const_new();

/// Something was queued: send it now.
pub fn kick() {
    WAKE.notify_one();
}

#[cfg(test)]
use quince_icloud::ckdocument::Run;
/// Notes' changes: recorded, laid, sent and refreshed by the code the phone's engine runs too
/// (crates/quince-sync).
pub use quince_sync::notes::{
    record_note_create, record_note_delete, record_note_move, record_note_pin, record_note_save,
    NOTES,
};
#[cfg(test)]
use quince_sync::notes::{send as send_note, NoteDelete, NoteMove, NoteSave};
/// Photos' flag changes (Favorite, Hide, Delete, Recover, Delete for good): recorded, laid and
/// sent by the code the phone's engine runs too (crates/quince-sync).
pub use quince_sync::photos::PHOTOS;
#[cfg(test)]
use quince_sync::queue::{drain, transient};
use quince_sync::queue::{drain_as, json, last_word, unreadable, Outcome, Report};
#[cfg(test)]
use quince_sync::reminders::{
    record_reminder_advance, replay as replay_reminders, send as send_reminder, Advance, Done, Edit,
};
/// Reminders' changes: recorded, laid, sent and refreshed by the code the phone's engine runs
/// too (crates/quince-sync).
pub use quince_sync::reminders::{
    record_reminder_create, record_reminder_delete, record_reminder_edit, record_reminder_tick,
    Create, REMINDERS,
};
pub const CONTACTS: &str = "contacts";
pub const CALENDAR: &str = "calendar";

/// Where a calendar account's queue is named: one each, so a server that is down holds up only
/// its own changes, never iCloud's or another account's.
const ACCOUNT_QUEUE: &str = "caldav:";

/// The queue of the calendar account `id`.
pub fn account_queue(id: &str) -> String {
    format!("{ACCOUNT_QUEUE}{id}")
}

/// The queue a change to this calendar waits in.
fn queue_of(calendar_guid: &str) -> String {
    quince_core::dav::account_of(calendar_guid).map_or_else(|| CALENDAR.to_string(), account_queue)
}

/// What a queue is to the window, which knows a change as the Calendar's whichever account it
/// goes to.
fn service_shown(service: &str) -> &str {
    if service.starts_with(ACCOUNT_QUEUE) {
        CALENDAR
    } else {
        service
    }
}

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

/// An event made or saved here: as the person left it, and as it was before (`None` for a new
/// one) — what a conflict is merged against, and where a move is made from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventSave {
    pub event: Event,
    pub base: Option<Event>,
    pub create: bool,
    /// Alerts set to exactly these; `None` leaves Apple's.
    pub alerts: Option<Vec<i64>>,
    /// Whether the repeat is set (to `repeat`, `None` for never) or left as Apple holds it.
    pub repeat_changed: bool,
    pub repeat: Option<EventRepeat>,
    /// For one occurrence of a repeating event: which events the change is for.
    pub scope: Option<Scope>,
}

impl EventSave {
    /// A later save of the same event, not yet sent, laid over this one: one save, from the
    /// event as this one began from it.
    fn then(self, later: Self) -> Self {
        Self {
            event: later.event,
            base: self.base,
            create: self.create,
            alerts: later.alerts.or(self.alerts),
            repeat_changed: self.repeat_changed || later.repeat_changed,
            repeat: if later.repeat_changed {
                later.repeat
            } else {
                self.repeat
            },
            scope: later.scope,
        }
    }
}

/// An event deleted here — with its scope, for one of a repeating series.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventDelete {
    pub event: Event,
    pub scope: Option<Scope>,
}

// --- Recording: the command's half. Each records the change and lays it over the library as
// one transaction, so a crash leaves both or neither.

/// A card saved here — made (no id yet: one is given now) or edited. Answers its id. The
/// shape a save had before the form showed the rest of the name and work: kept for the tests
/// that queue one, as a window from before would have.
#[cfg(test)]
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
        let after = postal::laid_over(&current, &base, &edits)
            .map_err(|e| quince_core::Error::Io(std::io::Error::other(e.to_string())))?;
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

/// An event made or saved here: shown at once, sent behind. Saves not yet sent and still the
/// last word on the event, for the same events of a series, are one save; one already on its
/// way is followed by the next, made from what it sends.
pub fn record_event_save(library: &mut Library, save: EventSave) -> quince_core::Result<()> {
    let guid = save.event.guid.clone();
    let queue = queue_of(&save.event.calendar_guid);
    library.atomically(|l| {
        let waiting = last_word(l, &queue, &guid, "save")?.and_then(|p| {
            serde_json::from_str::<EventSave>(&p.payload)
                .ok()
                .map(|s| (p, s))
        });
        let (save, whole) = match waiting {
            Some((p, before)) if p.tries == 0 && before.scope == save.scope => {
                let merged = before.then(save);
                if !l.outbox_amend(p.id, &json(&merged)?)? {
                    l.outbox_append(&queue, &guid, "save", &json(&merged)?)?;
                }
                // Over the one it joins, which the library already shows: what the form
                // holds now is the whole of it, a field put back as it was included.
                (merged, true)
            }
            Some((_, before)) => {
                let after = EventSave {
                    base: Some(before.event),
                    create: false,
                    ..save
                };
                l.outbox_append(&queue, &guid, "save", &json(&after)?)?;
                (after, false)
            }
            None => {
                l.outbox_append(&queue, &guid, "save", &json(&save)?)?;
                (save, false)
            }
        };
        lay_event(l, &save, !whole)
    })
}

/// A save laid over the library. `merging`, as the send will settle it: a field this save left
/// as its base had it shows the library's — another device's change, from a pass that landed
/// after the save was made — rather than the older value the save carried. Not merging, the
/// save is shown whole.
fn lay_event(l: &mut Library, s: &EventSave, merging: bool) -> quince_core::Result<()> {
    if !l
        .calendars()?
        .iter()
        .any(|c| c.guid == s.event.calendar_guid)
    {
        return Ok(());
    }
    let current = l.event(&s.event.guid)?;
    let mut event = match (&s.base, &current) {
        (Some(base), Some(current)) if merging => {
            quince_icloud::calendar::merge_edit(base, &s.event, current)
        }
        _ => s.event.clone(),
    };
    // Apple's tag for it stays what the library last heard of Apple's own version.
    event.etag = current.and_then(|e| e.etag);
    l.put_events(&[event])
}

/// What a save of event `guid` is made from: the event as the form opened it, when it says —
/// not the library's copy at save time, which a pass may have brought another device's change
/// into while the form was open. A create, or a window that sends none, has the library's.
pub fn save_base(guid: &str, opened: Option<Event>, stored: Option<Event>) -> Option<Event> {
    match opened {
        Some(opened) if opened.guid == guid => Some(opened),
        _ => stored,
    }
}

/// The tag a save is sent with: its base's, the version it was made from — never the library's
/// newest. Sent against a newer one, Apple took a save made from an older version whole, and
/// another device's change to a field it left alone was lost; against its base's, Apple answers
/// that it has changed, and the save is merged with what Apple holds.
fn sent_etag(s: &EventSave) -> Option<String> {
    match &s.base {
        Some(base) => base.etag.clone(),
        None => s.event.etag.clone(),
    }
}

/// An event deleted here — the one day, those after it, or the whole series.
pub fn record_event_delete(
    library: &mut Library,
    event: &Event,
    scope: Option<Scope>,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let d = EventDelete {
            event: event.clone(),
            scope,
        };
        l.outbox_append(
            &queue_of(&event.calendar_guid),
            &event.guid,
            "delete",
            &json(&d)?,
        )?;
        lay_event_delete(l, &d)
    })
}

fn lay_event_delete(l: &mut Library, d: &EventDelete) -> quince_core::Result<()> {
    use quince_icloud::calendar::series_of;
    // What goes at Apple goes here: the whole series for all of it, this day and those after
    // for this-and-future — not just the one day.
    let series = series_of(&d.event.guid).to_string();
    let gone: Vec<String> = match d.scope {
        Some(Scope::All | Scope::Future) => {
            let mut gone = Vec::new();
            for (g, _) in l.events_starting_between(i64::MIN / 2, i64::MAX / 2)? {
                if series_of(&g) != series {
                    continue;
                }
                let keep = d.scope == Some(Scope::Future)
                    && l.event(&g)?
                        .is_some_and(|e| e.starts_at < d.event.starts_at);
                if !keep {
                    gone.push(g);
                }
            }
            gone
        }
        _ => vec![d.event.guid.clone()],
    };
    l.forget_events(&gone)
}

/// Lay one waiting change over the library.
fn lay(l: &mut Library, p: &Pending) -> quince_core::Result<()> {
    if p.service == REMINDERS {
        return quince_sync::reminders::lay(l, p);
    }
    if p.service == NOTES {
        return quince_sync::notes::lay(l, p);
    }
    if p.service == PHOTOS {
        return quince_sync::photos::lay(l, p);
    }
    match (service_shown(&p.service), p.kind.as_str()) {
        (CONTACTS, "save") => {
            if let Ok(s) = serde_json::from_str::<ContactSave>(&p.payload) {
                lay_contact_save(l, &s.contact, s.details.as_ref())?;
            }
        }
        (CONTACTS, "merge") => {
            if let Ok(m) = serde_json::from_str::<crate::contacts_merge::ContactMerge>(&p.payload) {
                crate::contacts_merge::lay(l, &m)?;
            }
        }
        (CONTACTS, "delete") => l.forget_contact(&p.record)?,
        (CONTACTS, "address") => {
            if let Ok(a) = serde_json::from_str::<ContactAddress>(&p.payload) {
                lay_contact_address(l, &p.record, &a)?;
            }
        }
        (CALENDAR, "save") => {
            if let Ok(s) = serde_json::from_str::<EventSave>(&p.payload) {
                lay_event(l, &s, true)?;
            }
        }
        (CALENDAR, "delete") => {
            if let Ok(d) = serde_json::from_str::<EventDelete>(&p.payload) {
                lay_event_delete(l, &d)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Lay every change still waiting for `service` over what a sync pass just stored, in the
/// order they were made.
pub fn replay(library: &mut Library, service: &str) -> quince_core::Result<()> {
    if service == PHOTOS {
        // Photos' own, which also puts back a photo set aside that no delete waits for now.
        return quince_sync::photos::replay(library);
    }
    for p in library.outbox_pending(service)? {
        lay(library, &p)?;
    }
    Ok(())
}

/// Every change still waiting for `service` but `except`, laid over the library: what Apple
/// answered for one just sent is stored, and what still waits behind it stays as made.
pub fn replay_except(library: &mut Library, service: &str, except: i64) -> quince_core::Result<()> {
    for p in library.outbox_pending(service)? {
        if p.id != except {
            lay(library, &p)?;
        }
    }
    Ok(())
}

// --- Sending: the worker's half.

/// Runs for as long as the app does. Starts by sending whatever an earlier run left.
pub fn start(app: AppHandle) {
    watch_network();
    tauri::async_runtime::spawn(async move {
        let mut wait = Duration::ZERO;
        loop {
            tokio::select! {
                () = WAKE.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
            // Reminders and Notes changes wait in the queue while Advanced Data Protection keeps
            // their keys until the next approval — sent then, never refused meanwhile.
            let waiting = {
                let auth = app.state::<Auth>();
                let client = auth.client.lock().await;
                crate::data_access::waiting(&client)
            };
            let outcomes = [
                (
                    REMINDERS,
                    if held(REMINDERS, &waiting) {
                        Outcome::Idle
                    } else {
                        drain_reminders(&app).await
                    },
                ),
                (
                    NOTES,
                    if held(NOTES, &waiting) {
                        Outcome::Idle
                    } else {
                        drain_notes(&app).await
                    },
                ),
                (
                    PHOTOS,
                    if held(PHOTOS, &waiting) {
                        Outcome::Idle
                    } else {
                        drain_photos(&app).await
                    },
                ),
                (CONTACTS, drain_contacts(&app).await),
                (
                    CALENDAR,
                    either(drain_calendar(&app).await, drain_accounts(&app).await),
                ),
            ];
            // Said once, after the pass, and only to the services it touched. Told before and
            // after every pass with no service named, each kept-mounted screen read its whole
            // list again twice whatever had moved: a Notes save re-read Reminders and Calendar.
            // Nothing waiting is nothing to say, so an idle pass every thirty seconds is quiet.
            if let Some(changed) = changed(&outcomes) {
                let _ = app.emit(CHANGED, changed);
            }
            wait = if outcomes.iter().any(|(_, o)| matches!(o, Outcome::Later)) {
                // Apple out of reach: wait longer each time, up to two minutes.
                (wait * 2).clamp(Duration::from_secs(4), Duration::from_secs(120))
            } else {
                Duration::from_secs(30)
            };
        }
    });
}

/// Whether `service`'s queued changes wait for Advanced Data Protection's next approval:
/// its key is not held, so sending would only be refused. Kept, in order, for then.
fn held(service: &str, waiting: &[&'static str]) -> bool {
    waiting.contains(&service)
}

/// Told to the window whenever what waits may have changed.
pub const CHANGED: &str = "quince://outbox-changed";

/// What [`CHANGED`] carries after a pass: the services it sent for, or found waiting. A screen
/// reads its rows again only when its own service is named.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Changed {
    pub services: Vec<&'static str>,
}

/// What a pass over every service comes to for the window: the services that had anything
/// waiting, or `None` when none did — nothing was sent, so nothing on screen moved.
fn changed(outcomes: &[(&'static str, Outcome)]) -> Option<Changed> {
    let services: Vec<&'static str> = outcomes
        .iter()
        .filter(|(_, o)| !matches!(o, Outcome::Idle))
        .map(|(service, _)| *service)
        .collect();
    (!services.is_empty()).then_some(Changed { services })
}

#[derive(Clone, serde::Serialize)]
struct RefusedEvent {
    service: &'static str,
    record: String,
    message: String,
}

fn tell_refused(app: &AppHandle, service: &'static str, report: &Report) {
    for (record, message) in &report.refused {
        let _ = app.emit(
            "quince://outbox-refused",
            RefusedEvent {
                service,
                record: record.clone(),
                message: message.clone(),
            },
        );
    }
}

/// Whether anything waits for `service` — the answer when Apple can't be asked yet.
async fn waiting(data: &Data, service: &str) -> Outcome {
    if data
        .0
        .lock()
        .await
        .outbox_pending(service)
        .map(|p| p.is_empty())
        .unwrap_or(true)
    {
        Outcome::Clear
    } else {
        Outcome::Later
    }
}

async fn drain_reminders(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, REMINDERS).await.is_clear() {
        return Outcome::Idle;
    }
    let session = if crate::same_account(app).await {
        cloudkit_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        // Not signed in, or the session is being renewed: nothing can go yet.
        return Outcome::Later;
    };
    let Some(account) = crate::asked_as(&params) else {
        return Outcome::Later;
    };
    let report = drain_as(&data.0, REMINDERS, &crate::REMINDERS_PASS, &account, |p| {
        quince_sync::reminders::send(&data.0, &http, &root, params.clone(), p)
    })
    .await;
    tell_refused(app, REMINDERS, &report);
    // Apple's own version of what was sent — its new change tags, and the truth over anything
    // it refused.
    if report.sent || !report.refused.is_empty() {
        let _ = crate::refresh_reminders(&http, &root, params, &data, false).await;
        crate::alerts::changed();
        let _ = app.emit("quince://reminders-synced", ());
    }
    report.outcome
}

async fn drain_notes(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, NOTES).await.is_clear() {
        return Outcome::Idle;
    }
    let session = if crate::same_account(app).await {
        cloudkit_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        return Outcome::Later;
    };
    let Some(account) = crate::asked_as(&params) else {
        return Outcome::Later;
    };
    let report = drain_as(&data.0, NOTES, &crate::NOTES_PASS, &account, |p| {
        quince_sync::notes::send(&data.0, &http, &root, params.clone(), p)
    })
    .await;
    tell_refused(app, NOTES, &report);
    if report.sent || !report.refused.is_empty() {
        let _ = crate::refresh_notes(&http, &root, params, &data).await;
    }
    report.outcome
}

async fn drain_photos(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, PHOTOS).await.is_clear() {
        return Outcome::Idle;
    }
    let session = if crate::same_account(app).await {
        cloudkit_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        return Outcome::Later;
    };
    let Some(account) = crate::asked_as(&params) else {
        return Outcome::Later;
    };
    // Held under the photos pass as each is sent, so no pass lands between Apple taking a
    // change and its coming off the queue. Apple's answers are kept as each is taken (new
    // versions, the date a deleted photo goes), so no pass is needed after.
    let report = drain_as(&data.0, PHOTOS, &crate::PHOTOS_PASS, &account, |p| {
        quince_sync::photos::send(&data.0, &http, &root, params.clone(), p)
    })
    .await;
    tell_refused(app, PHOTOS, &report);
    report.outcome
}

async fn drain_contacts(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, CONTACTS).await.is_clear() {
        return Outcome::Idle;
    }
    let session = if crate::same_account(app).await {
        crate::contacts_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        return Outcome::Later;
    };
    // A merge waits a moment before it goes, so it can still be undone.
    if let Some(wait) = crate::contacts_merge::held(&data).await {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(wait).await;
            kick();
        });
        return Outcome::Clear;
    }
    let Some(account) = crate::asked_as(&params) else {
        return Outcome::Later;
    };
    // The book read once for the run, not once for each change in it — while Apple's answers
    // keep the run in tokens. The ledger holds the tokens the next write carries: the read's,
    // then each write's answer's, never one an earlier write spent.
    let book = std::sync::Mutex::new(Book::default());
    let ledger = quince_icloud::contacts::Ledger::new();
    let report = drain_as(&data.0, CONTACTS, &crate::CONTACTS_PASS, &account, |p| {
        send_contact(&data.0, &http, &root, params.clone(), p, &book, &ledger)
    })
    .await;
    tell_refused(app, CONTACTS, &report);
    if report.sent || !report.refused.is_empty() {
        let _one = crate::CONTACTS_PASS.lock().await;
        let _ = crate::resync_contacts(&http, &root, params, &data).await;
    }
    report.outcome
}

/// The address book a run of queued contact changes is sent from: read once for the run, not
/// once for each change — every read is the whole book, every card in it. A card the run has
/// written since is read again before it is written again (the copy here is from before); so
/// is the book when the last write's answer left no tokens to carry on with (see
/// [`quince_icloud::contacts::Ledger`]); and a write Apple says was made against an old book
/// is sent once more from a fresh read.
#[derive(Default)]
struct Book {
    read: Option<std::sync::Arc<BookRead>>,
    written: std::collections::HashSet<String>,
}

/// One read of the book: each card as Apple wrote it, by id.
struct BookRead {
    cards: std::collections::HashMap<String, serde_json::Value>,
}

impl Book {
    /// The book to send a change to `record` from, read now if the run has none it can use —
    /// none read yet, `record` written since, or no live tokens left to write with; and
    /// whether it was read just now. A read hands its tokens to `ledger`.
    async fn for_record(
        book: &std::sync::Mutex<Self>,
        ledger: &quince_icloud::contacts::Ledger,
        http: &reqwest::Client,
        root: &str,
        params: &[(String, String)],
        record: &str,
    ) -> Result<(std::sync::Arc<BookRead>, bool), Error> {
        let kept = {
            let b = book
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            b.read
                .clone()
                .filter(|_| !b.written.contains(record) && ledger.has_tokens())
        };
        if let Some(read) = kept {
            return Ok((read, false));
        }
        let (cards, tokens) =
            quince_icloud::contacts::cards_with_tokens(http, root, params.to_vec()).await?;
        ledger.hold(tokens);
        let cards = cards
            .into_iter()
            .filter_map(|card| {
                let id = card.get("contactId")?.as_str()?.to_string();
                Some((id, card))
            })
            .collect();
        let mut b = book
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let read = std::sync::Arc::new(BookRead { cards });
        b.read = Some(std::sync::Arc::clone(&read));
        b.written.clear();
        Ok((read, true))
    }

    /// `record` written: its copy in the read is old now.
    fn wrote(book: &std::sync::Mutex<Self>, record: &str) {
        book.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .written
            .insert(record.to_string());
    }

    /// The read let go of: written past it in ways it cannot track (a merge).
    fn forget(book: &std::sync::Mutex<Self>) {
        book.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .read = None;
    }
}

async fn send_contact(
    library: &crate::library_lock::LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
    book: &std::sync::Mutex<Book>,
    ledger: &quince_icloud::contacts::Ledger,
) -> Result<(), Error> {
    use quince_icloud::contacts as c;
    if p.kind == "merge" {
        // A merge writes and deletes cards of its own, read by itself.
        Book::forget(book);
        return crate::contacts_merge::send(library, http, root, params, ledger, &p).await;
    }
    if p.kind == "address" {
        let a: ContactAddress = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
        Book::wrote(book, &p.record);
        // It reads the card and its tokens itself: the run's tokens are spent by its write.
        let edited = c::address::edit(http, root, params, &p.record, &a.base, &a.edits).await;
        ledger.spend();
        return edited.map(|_| ());
    }
    let (read, fresh) = Book::for_record(book, ledger, http, root, &params, &p.record).await?;
    let sent = send_contact_from(http, root, params.clone(), &p, &read, ledger).await;
    if matches!(sent, Err(Error::Conflict)) && !fresh {
        // Made against a book read earlier in the run, which Apple says is old now: from a
        // fresh read, once.
        Book::forget(book);
        let (read, _) = Book::for_record(book, ledger, http, root, &params, &p.record).await?;
        let again = send_contact_from(http, root, params, &p, &read, ledger).await;
        Book::wrote(book, &p.record);
        return again;
    }
    Book::wrote(book, &p.record);
    sent
}

/// One queued card change sent, from `read`: Apple's card as it was read, with the tokens
/// `ledger` holds.
async fn send_contact_from(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: &Pending,
    read: &BookRead,
    ledger: &quince_icloud::contacts::Ledger,
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

/// What two queues of one service come to together.
fn either(a: Outcome, b: Outcome) -> Outcome {
    match (a, b) {
        (Outcome::Later, _) | (_, Outcome::Later) => Outcome::Later,
        (Outcome::Clear, _) | (_, Outcome::Clear) => Outcome::Clear,
        _ => Outcome::Idle,
    }
}

/// Each calendar account's queue, on its own server.
async fn drain_accounts(app: &AppHandle) -> Outcome {
    let data = app.state::<Data>();
    let (accounts, owner) = {
        let l = data.0.lock().await;
        (
            l.dav_accounts().unwrap_or_default(),
            l.state(crate::ACCOUNT_KEY).ok().flatten(),
        )
    };
    let mut outcome = Outcome::Idle;
    for account in accounts {
        let queue = account_queue(&account.id);
        if waiting(&data, &queue).await.is_clear() {
            continue;
        }
        // A refused password asked again looks like someone guessing it: kept for the sign-in.
        let ready = !crate::dav::waiting_for_sign_in(&account) && crate::same_account(app).await;
        let remote = ready
            .then(|| crate::dav::remote_of(app, &account).ok())
            .flatten();
        let (Some(remote), Some(owner)) = (remote, owner.as_deref()) else {
            outcome = either(outcome, Outcome::Later);
            continue;
        };
        let report = drain_as(&data.0, &queue, &crate::dav::PASS, owner, |p| {
            crate::dav::send(app, &data.0, &remote, &account, p)
        })
        .await;
        tell_refused(app, CALENDAR, &report);
        if report.sent || !report.refused.is_empty() {
            let _ = app.emit("quince://calendar-synced", ());
        }
        outcome = either(outcome, report.outcome);
    }
    outcome
}

async fn drain_calendar(app: &AppHandle) -> Outcome {
    let auth = app.state::<Auth>();
    let data = app.state::<Data>();
    if waiting(&data, CALENDAR).await.is_clear() {
        return Outcome::Idle;
    }
    let session = if crate::same_account(app).await {
        crate::calendar_context(&auth).await
    } else {
        Err(String::new())
    };
    let Ok((http, root, params)) = session else {
        return Outcome::Later;
    };
    let Some(account) = crate::asked_as(&params) else {
        return Outcome::Later;
    };
    let report = drain_as(&data.0, CALENDAR, &crate::CALENDAR_PASS, &account, |p| {
        send_event(&data.0, &http, &root, params.clone(), p)
    })
    .await;
    tell_refused(app, CALENDAR, &report);
    if report.sent || !report.refused.is_empty() {
        let _ = app.emit("quince://calendar-synced", ());
    }
    report.outcome
}

async fn send_event(
    library: &crate::library_lock::LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
) -> Result<(), Error> {
    use quince_icloud::calendar::CalendarClient;
    use quince_icloud::sync::{calendar_ctag, save_event_merging, store_events};
    match p.kind.as_str() {
        "save" => {
            let s: EventSave = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            let (event, ctag, from) = {
                let l = library.lock().await;
                let mut event = s.event.clone();
                event.etag = sent_etag(&s);
                let ctag = calendar_ctag(&l, &event.calendar_guid);
                // Saved to another calendar than it was in: a move, from there.
                let from = s
                    .base
                    .as_ref()
                    .map(|b| b.calendar_guid.clone())
                    .filter(|old| *old != event.calendar_guid)
                    .map(|old| {
                        let tag = calendar_ctag(&l, &old);
                        (old, tag)
                    });
                (event, ctag, from)
            };
            // A create sent again — its answer lost — merges against itself: no change.
            let base = s.base.clone().or_else(|| s.create.then(|| s.event.clone()));
            let kept = save_event_merging(
                http,
                root,
                params,
                ctag.as_deref(),
                &event,
                base.as_ref(),
                s.alerts.as_deref(),
                s.repeat_changed.then_some(s.repeat.as_ref()),
                s.scope,
                from.as_ref()
                    .map(|(old, tag)| (old.as_str(), tag.as_deref())),
            )
            .await?;
            // What Apple holds is stored — its tag the next write is made against — and what
            // still waits behind it laid back over.
            let mut l = library.lock().await;
            l.atomically(|l| {
                store_events(l, kept)?;
                replay_except(l, CALENDAR, p.id)?;
                Ok::<_, Error>(())
            })
        }
        "delete" => {
            let d: EventDelete = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            let mut event = d.event.clone();
            let ctag = calendar_ctag(&*library.lock().await, &event.calendar_guid);
            let client = CalendarClient::new(http, root, params);
            match client
                .delete_event_scoped(&event, ctag.as_deref(), d.scope)
                .await
            {
                Err(Error::Conflict) => {
                    // Changed at Apple since it was read: deleted all the same, as asked, against
                    // Apple's newest version — or done already, if it is gone.
                    let Some(theirs) = client.current_event(&event).await? else {
                        return Ok(());
                    };
                    event.etag = theirs.etag;
                    let ctag = client
                        .current_ctag(&event.calendar_guid)
                        .await
                        .ok()
                        .flatten();
                    client
                        .delete_event_scoped(&event, ctag.as_deref(), d.scope)
                        .await
                }
                other => other,
            }
        }
        other => Err(Error::Refused(format!(
            "Skykeep does not know how to send “{other}”."
        ))),
    }
}

// --- Waking the sender when the network returns.

/// Whether a change in connectivity is the network coming back: online now, and not known to
/// be online before. `was` is updated to now.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn came_online(was: &mut Option<bool>, online: bool) -> bool {
    let back = online && *was != Some(true);
    *was = Some(online);
    back
}

/// NetworkManager's `NMState`: 60 is reaching the local site, 70 the internet. Apple may be
/// reachable at either (a captive portal clears to 70), so both count.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn nm_online(state: u32) -> bool {
    state >= 60
}

/// connman's `State`: "ready" is connected, "online" connected and checked.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn connman_online(state: &str) -> bool {
    matches!(state, "ready" | "online")
}

/// Send what waits the moment the network comes back, rather than on the next backoff tick:
/// NetworkManager's `StateChanged`, or connman's `State`, on the system bus. Where neither
/// runs, the window's `online` event does the same ([`outbox_wake`]).
fn watch_network() {
    #[cfg(target_os = "linux")]
    tauri::async_runtime::spawn(async {
        use futures_util::StreamExt;
        let Ok(bus) = zbus::Connection::system().await else {
            return;
        };
        let proxy = |dest: &'static str, path: &'static str, iface: &'static str| {
            let bus = bus.clone();
            async move {
                zbus::proxy::Builder::<zbus::Proxy<'static>>::new(&bus)
                    .destination(dest)
                    .ok()?
                    .path(path)
                    .ok()?
                    .interface(iface)
                    .ok()?
                    .cache_properties(zbus::proxy::CacheProperties::No)
                    .build()
                    .await
                    .ok()
            }
        };
        let nm = async {
            let Some(nm) = proxy(
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
            )
            .await
            else {
                return;
            };
            let Ok(mut changes) = nm.receive_signal("StateChanged").await else {
                return;
            };
            let mut was = None;
            while let Some(signal) = changes.next().await {
                if let Ok(state) = signal.body().deserialize::<u32>() {
                    if came_online(&mut was, nm_online(state)) {
                        kick();
                        crate::push::wake();
                    }
                }
            }
        };
        let connman = async {
            let Some(cm) = proxy("net.connman", "/", "net.connman.Manager").await else {
                return;
            };
            let Ok(mut changes) = cm.receive_signal("PropertyChanged").await else {
                return;
            };
            let mut was = None;
            while let Some(signal) = changes.next().await {
                let Ok((name, value)) = signal
                    .body()
                    .deserialize::<(String, zbus::zvariant::OwnedValue)>()
                else {
                    continue;
                };
                if name != "State" {
                    continue;
                }
                if let Ok(state) = String::try_from(value) {
                    if came_online(&mut was, connman_online(&state)) {
                        kick();
                        crate::push::wake();
                    }
                }
            }
        };
        tokio::join!(nm, connman);
    });
}

/// The window saw the network come back (its `online` event): send what waits now.
#[tauri::command]
pub fn outbox_wake() {
    kick();
    // And look for what changed elsewhere while offline (push.rs).
    crate::push::wake();
}

// --- What the window asks.

/// A change waiting to go.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingView {
    service: String,
    record: String,
    kind: String,
}

/// A change Apple turned down, with what it held, for the person to keep.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefusedView {
    id: i64,
    service: String,
    record: String,
    kind: String,
    reason: String,
    made_at: i64,
    /// What was typed, in words: the note's text, the reminder's title, the person's name.
    content: String,
}

#[derive(Serialize)]
pub struct Status {
    waiting: Vec<WaitingView>,
    refused: Vec<RefusedView>,
}

/// The words a refused change held.
fn content_of(p: &Pending) -> String {
    let v: serde_json::Value = serde_json::from_str(&p.payload).unwrap_or_default();
    let s = |ptr: &str| v.pointer(ptr).and_then(|x| x.as_str()).map(str::to_string);
    match (service_shown(&p.service), p.kind.as_str()) {
        (NOTES, "delete") => s("/title"),
        (NOTES, "create" | "save") => s("/body"),
        (REMINDERS, "create") => s("/title"),
        (REMINDERS, "edit") => s("/title").or_else(|| s("/notes")),
        (REMINDERS, "delete") => s("/reminder/title"),
        (CALENDAR, _) => s("/event/title"),
        (CONTACTS, "address") => s("/name"),
        (CONTACTS, _) => {
            let name: Vec<String> = [
                "/contact/firstName",
                "/contact/lastName",
                "/contact/company",
            ]
            .iter()
            .filter_map(|ptr| s(ptr))
            .filter(|x| !x.trim().is_empty())
            .collect();
            Some(name.join(" "))
        }
        _ => None,
    }
    .unwrap_or_default()
}

pub fn status(library: &Library) -> quince_core::Result<Status> {
    Ok(Status {
        waiting: library
            .outbox_all_pending()?
            .into_iter()
            .map(|p| WaitingView {
                service: service_shown(&p.service).to_string(),
                record: p.record,
                kind: p.kind,
            })
            .collect(),
        refused: library
            .outbox_refused()?
            .into_iter()
            .map(|r| RefusedView {
                id: r.change.id,
                content: content_of(&r.change),
                service: service_shown(&r.change.service).to_string(),
                record: r.change.record,
                kind: r.change.kind,
                reason: r.reason,
                made_at: r.made_at,
            })
            .collect(),
    })
}

/// What waits for Apple and what Apple turned down.
#[tauri::command]
pub async fn outbox_status(data: State<'_, Data>) -> Result<Status, String> {
    status(&*data.0.lock().await).map_err(|e| e.to_string())
}

/// A refused change, sent again. It is laid over the library again as it goes.
#[tauri::command]
pub async fn outbox_resend(id: i64, app: AppHandle, data: State<'_, Data>) -> Result<(), String> {
    {
        let mut l = data.0.lock().await;
        l.atomically(|l| {
            if l.outbox_resend(id)? {
                if let Some(service) = l
                    .outbox_all_pending()?
                    .into_iter()
                    .next_back()
                    .map(|p| p.service)
                {
                    replay(l, &service)?;
                }
            }
            Ok::<_, quince_core::Error>(())
        })
        .map_err(|e| e.to_string())?;
    }
    let _ = app.emit(CHANGED, ());
    kick();
    Ok(())
}

/// A refused change let go of: Apple's version stands.
#[tauri::command]
pub async fn outbox_discard(id: i64, app: AppHandle, data: State<'_, Data>) -> Result<(), String> {
    let account = data
        .0
        .lock()
        .await
        .atomically(|l| {
            let queue = l
                .outbox_refused()?
                .into_iter()
                .find(|r| r.change.id == id)
                .map(|r| r.change.service);
            l.outbox_discard(id)?;
            // A photo a refused delete-for-good had set aside is back already, unless the
            // refusal was a fault in sending: let go of, it is put back here either way.
            if queue.as_deref() == Some(PHOTOS) {
                quince_sync::photos::replay(l)?;
            }
            // A pass finds nothing new to lay over a calendar whose tag has not moved, so what
            // was given up on is taken back off it here.
            let account = queue
                .as_deref()
                .and_then(|q| q.strip_prefix(ACCOUNT_QUEUE))
                .map(ToString::to_string);
            if let Some(account) = &account {
                crate::dav::lay_account(l, account)?;
            }
            Ok::<_, quince_core::Error>(account)
        })
        .map_err(|e| e.to_string())?;
    if account.is_some() {
        let _ = app.emit("quince://calendar-synced", ());
    }
    let _ = app.emit(CHANGED, ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pass names only the services it sent for or found waiting, once; an idle pass says
    /// nothing at all.
    #[test]
    fn a_pass_tells_only_the_services_it_touched() {
        assert_eq!(
            changed(&[
                (REMINDERS, Outcome::Idle),
                (NOTES, Outcome::Clear),
                (CONTACTS, Outcome::Idle),
                (CALENDAR, Outcome::Later),
            ]),
            Some(Changed {
                services: vec![NOTES, CALENDAR]
            })
        );
        assert_eq!(
            changed(&[
                (REMINDERS, Outcome::Idle),
                (NOTES, Outcome::Idle),
                (CONTACTS, Outcome::Idle),
                (CALENDAR, Outcome::Idle),
            ]),
            None
        );
        // The window reads it as `{ services: [...] }`.
        assert_eq!(
            serde_json::to_string(&Changed {
                services: vec![REMINDERS]
            })
            .unwrap(),
            r#"{"services":["reminders"]}"#
        );
    }
    use quince_core::reminders::ReminderList;
    use std::sync::{Arc, Mutex as StdMutex};

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        let list = |g: &str| ReminderList {
            guid: g.into(),
            title: g.into(),
            enabled: true,
            ..Default::default()
        };
        lib.put_reminder_lists(&[list("L"), list("M")]).unwrap();
        let r = |g: &str| Reminder {
            guid: g.into(),
            list_guid: "L".into(),
            title: g.into(),
            etag: Some("t".into()),
            ..Default::default()
        };
        lib.put_reminders(&[r("a"), r("b"), r("c")]).unwrap();
        lib
    }
    fn queue(lib: &mut Library, g: &str, done: bool) {
        lib.set_reminder_completed(g, done.then_some(1)).unwrap();
        let payload = serde_json::to_string(&Done {
            done,
            at: 1,
            base: Default::default(),
        })
        .unwrap();
        lib.outbox_push(REMINDERS, g, "done", &payload).unwrap();
    }
    fn note(lib: &mut Library, guid: &str, body: &str) {
        lib.put_notes(&[quince_core::notes::Note {
            guid: guid.into(),
            folder_guid: None,
            title: body.into(),
            snippet: None,
            body: Some(body.into()),
            locked: false,
            pinned: false,
            created_at: None,
            modified_at: None,
            editable: true,
            etag: None,
            runs: None,
        }])
        .unwrap();
    }
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

    #[test]
    fn sends_in_order_keeps_what_cannot_reach_apple_and_holds_refusals() {
        tauri::async_runtime::block_on(async {
            let lib = crate::library_lock::LibraryLock::new(library(), None);
            let pass = Mutex::new(());
            {
                let mut l = lib.lock().await;
                queue(&mut l, "a", true);
                queue(&mut l, "b", true);
                queue(&mut l, "c", true);
            }
            // Apple turns "b" down; everything goes through in the order it was made.
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let s2 = seen.clone();
            let report = drain(&lib, REMINDERS, &pass, move |p| {
                s2.lock().unwrap().push(p.record.clone());
                async move {
                    if p.record == "b" {
                        Err(Error::Refused("changed on another device".into()))
                    } else {
                        Ok(())
                    }
                }
            })
            .await;
            assert_eq!(*seen.lock().unwrap(), vec!["a", "b", "c"]);
            assert!(report.sent && report.outcome.is_clear());
            assert_eq!(
                report.refused,
                vec![("b".to_string(), "changed on another device".to_string())]
            );
            assert!(lib
                .lock()
                .await
                .outbox_pending(REMINDERS)
                .unwrap()
                .is_empty());
            // Refused is kept, never dropped.
            let refused = lib.lock().await.outbox_refused().unwrap();
            assert_eq!(refused.len(), 1);
            assert_eq!(refused[0].change.record, "b");

            // Offline: the first stays, with all after it, in order, for next time.
            {
                let mut l = lib.lock().await;
                queue(&mut l, "a", false);
                queue(&mut l, "c", false);
            }
            let report = drain(&lib, REMINDERS, &pass, |_p| async {
                Err(Error::Protocol("CloudKit refused the change (503)".into()))
            })
            .await;
            assert!(!report.outcome.is_clear() && !report.sent);
            let left = lib.lock().await.outbox_pending(REMINDERS).unwrap();
            assert_eq!(
                left.iter().map(|p| p.record.as_str()).collect::<Vec<_>>(),
                vec!["a", "c"]
            );
            assert_eq!(left[0].tries, 1);

            // A sync pass that read Apple's older copy is laid over with what is still waiting.
            {
                let mut l = lib.lock().await;
                l.set_reminder_completed("a", Some(5)).unwrap();
                replay_reminders(&mut l).unwrap();
                assert_eq!(l.reminder("a").unwrap().unwrap().completed_at, None);
            }

            // Back online: both go.
            let report = drain(&lib, REMINDERS, &pass, |_p| async { Ok(()) }).await;
            assert!(report.sent && report.outcome.is_clear());
            assert!(lib
                .lock()
                .await
                .outbox_pending(REMINDERS)
                .unwrap()
                .is_empty());
        });
    }

    /// Offline, a reminder is made, edited twice and another deleted: each shows at once, a
    /// pass reading Apple's older state is laid over with them all, and they go in the order
    /// they were made, the two edits as one.
    #[test]
    fn reminder_changes_show_at_once_survive_a_pass_and_go_in_order() {
        let mut lib = library();
        let made = record_reminder_create(
            &mut lib,
            &Create {
                list_guid: "L".into(),
                title: " Buy milk ".into(),
                due_at: Some(100),
                all_day: Some(true),
                flagged: None,
            },
        )
        .unwrap();
        assert!(made.starts_with("Reminder/"), "named here, before Apple");
        let shown = lib.reminder(&made).unwrap().unwrap();
        assert_eq!(
            (shown.title.as_str(), shown.due_at, shown.all_day),
            ("Buy milk", Some(100), true)
        );

        let a = lib.reminder("a").unwrap().unwrap();
        record_reminder_edit(
            &mut lib,
            &a,
            &ReminderEdit {
                title: Some("A renamed".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let a = lib.reminder("a").unwrap().unwrap();
        record_reminder_edit(
            &mut lib,
            &a,
            &ReminderEdit {
                notes: Some(None),
                list_guid: Some("M".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let b = lib.reminder("b").unwrap().unwrap();
        record_reminder_delete(&mut lib, &b).unwrap();

        let a = lib.reminder("a").unwrap().unwrap();
        assert_eq!((a.title.as_str(), a.list_guid.as_str()), ("A renamed", "M"));
        assert!(lib.reminder("b").unwrap().is_none(), "deleted at once");
        assert!(a.etag.is_some(), "Apple's tag kept for the write");

        let p = lib.outbox_pending(REMINDERS).unwrap();
        assert_eq!(
            p.iter()
                .map(|x| (x.record.as_str(), x.kind.as_str()))
                .collect::<Vec<_>>(),
            vec![(made.as_str(), "create"), ("a", "edit"), ("b", "delete")]
        );
        let edit: Edit = serde_json::from_str(&p[1].payload).unwrap();
        assert_eq!(edit.title.as_deref(), Some("A renamed"));
        assert_eq!(edit.notes, Some(None), "clear kept apart from leave");
        assert_eq!(edit.from_list, "L", "moved from where it first was");

        // A full pass reads Apple's state from before any of it.
        lib.retain_reminders(&["a".into(), "b".into(), "c".into()])
            .unwrap();
        lib.put_reminders(&[
            Reminder {
                guid: "a".into(),
                list_guid: "L".into(),
                title: "a".into(),
                etag: Some("t2".into()),
                ..Default::default()
            },
            Reminder {
                guid: "b".into(),
                list_guid: "L".into(),
                title: "b".into(),
                ..Default::default()
            },
        ])
        .unwrap();
        replay_reminders(&mut lib).unwrap();
        assert!(lib.reminder(&made).unwrap().is_some());
        let a = lib.reminder("a").unwrap().unwrap();
        assert_eq!(
            (a.title.as_str(), a.list_guid.as_str(), a.etag.as_deref()),
            ("A renamed", "M", Some("t2"))
        );
        assert!(lib.reminder("b").unwrap().is_none());
    }

    /// An edit already taken to be sent is followed, never rewritten.
    #[test]
    fn an_edit_on_its_way_is_followed_not_rewritten() {
        let mut lib = library();
        let a = lib.reminder("a").unwrap().unwrap();
        let edit = |t: &str| ReminderEdit {
            title: Some(t.into()),
            ..Default::default()
        };
        record_reminder_edit(&mut lib, &a, &edit("one")).unwrap();
        lib.outbox_take(REMINDERS).unwrap();
        record_reminder_edit(&mut lib, &a, &edit("two")).unwrap();
        let p = lib.outbox_pending(REMINDERS).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(
            serde_json::from_str::<Edit>(&p[0].payload)
                .unwrap()
                .title
                .as_deref(),
            Some("one")
        );
        assert_eq!(
            serde_json::from_str::<Edit>(&p[1].payload)
                .unwrap()
                .title
                .as_deref(),
            Some("two")
        );
    }

    /// Opening a note is not an edit. His "Attachments" note (text made generic) queued a save
    /// on being opened: the same text, Apple's runs in pieces against the editor's merged.
    /// That save — and one typed then undone — is never queued; a real edit is.
    #[test]
    fn a_save_that_changes_nothing_is_never_queued() {
        let plain = |length| Run {
            length,
            ..Run::default()
        };
        let at = |id: &str| Run {
            attachment: Some(id.into()),
            ..plain(1)
        };
        let text = "Attachments\n\u{FFFC}\n\u{FFFC}\nA few words to try it out \n\u{FFFC}";
        let apple = vec![
            plain(10),
            plain(1),
            plain(1),
            at("A1"),
            plain(1),
            at("A2"),
            plain(1),
            plain(2),
            plain(10),
            plain(8),
            plain(5),
            plain(1),
            plain(1),
            at("A3"),
        ];
        let editor = vec![plain(12), at("A1"), plain(1), at("A2"), plain(28), at("A3")];
        let mut lib = Library::in_memory().unwrap();
        note(&mut lib, "N", text);
        lib.set_note_runs("N", &serde_json::to_string(&apple).unwrap())
            .unwrap();

        // As the window sent it, with the base it read — and with none, from the library.
        record_note_save(
            &mut lib,
            "N",
            text,
            Some(text.into()),
            Some(editor.clone()),
            Some(apple.clone()),
        )
        .unwrap();
        record_note_save(&mut lib, "N", text, None, Some(editor.clone()), None).unwrap();
        record_note_save(&mut lib, "N", text, None, None, None).unwrap();
        assert!(
            lib.outbox_pending(NOTES).unwrap().is_empty(),
            "opening a note queued a save"
        );
        let kept: Vec<Run> = serde_json::from_str(&lib.note_runs("N").unwrap().unwrap()).unwrap();
        assert_eq!(kept, apple, "the library's runs were rewritten");

        // Typed, then undone before it went: the change is taken back, not sent.
        let typed = text.replace("out ", "out! ");
        let mut mine = editor.clone();
        mine[4].length += 1;
        record_note_save(
            &mut lib,
            "N",
            &typed,
            Some(text.into()),
            Some(mine.clone()),
            Some(apple.clone()),
        )
        .unwrap();
        assert_eq!(
            lib.outbox_pending(NOTES).unwrap().len(),
            1,
            "a real edit is queued"
        );
        record_note_save(
            &mut lib,
            "N",
            text,
            Some(text.into()),
            Some(editor.clone()),
            Some(apple.clone()),
        )
        .unwrap();
        assert!(
            lib.outbox_pending(NOTES).unwrap().is_empty(),
            "an undone edit was still sent"
        );

        // A restyle with the same text is an edit.
        let mut bold = editor.clone();
        bold.splice(
            4..5,
            [
                plain(1),
                Run {
                    bold: true,
                    ..plain(1)
                },
                plain(26),
            ],
        );
        record_note_save(
            &mut lib,
            "N",
            text,
            Some(text.into()),
            Some(bold),
            Some(apple),
        )
        .unwrap();
        assert_eq!(
            lib.outbox_pending(NOTES).unwrap().len(),
            1,
            "a restyle is queued"
        );
    }

    /// Note saves: shown at once; several unsent are one, from the first base; one on its way
    /// is followed by the next typed over what it sends; a pass is laid over.
    #[test]
    fn note_saves_keep_their_base_and_survive_a_pass() {
        let mut lib = Library::in_memory().unwrap();
        note(&mut lib, "N", "Apple's text");
        record_note_save(&mut lib, "N", "Apple's text, mine", None, None, None).unwrap();
        record_note_save(
            &mut lib,
            "N",
            "Apple's text, mine more",
            Some("ignored".into()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("Apple's text, mine more")
        );
        let p = lib.outbox_pending(NOTES).unwrap();
        assert_eq!(p.len(), 1, "one save");
        let s: NoteSave = serde_json::from_str(&p[0].payload).unwrap();
        assert_eq!(
            s.base.as_deref(),
            Some("Apple's text"),
            "from what was typed over first"
        );

        lib.outbox_take(NOTES).unwrap();
        record_note_save(
            &mut lib,
            "N",
            "Apple's text, mine more and more",
            None,
            None,
            None,
        )
        .unwrap();
        let p = lib.outbox_pending(NOTES).unwrap();
        let s: NoteSave = serde_json::from_str(&p[1].payload).unwrap();
        assert_eq!(s.base.as_deref(), Some("Apple's text, mine more"));

        // A pass brings Apple's older text: the waiting typing is laid back over it.
        note(&mut lib, "N", "Apple's text");
        replay(&mut lib, NOTES).unwrap();
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("Apple's text, mine more and more")
        );
    }

    /// A note as a pass stores it: Apple's title and time edited.
    fn titled(lib: &mut Library, guid: &str, title: &str, body: &str, modified: i64) {
        lib.put_notes(&[quince_core::notes::Note {
            guid: guid.into(),
            folder_guid: None,
            title: title.into(),
            snippet: None,
            body: Some(body.into()),
            locked: false,
            pinned: false,
            created_at: None,
            modified_at: Some(modified),
            editable: true,
            etag: None,
            runs: None,
        }])
        .unwrap();
    }

    /// A save keeps the title Apple holds — the phone's — while the title line is untouched,
    /// before it is sent and across a pass; an edit of the title line takes the new one.
    #[test]
    fn a_save_keeps_apples_title_until_the_title_line_changes() {
        let mut lib = Library::in_memory().unwrap();
        let camp = "\u{1F3D5}\u{1F3D5}";
        let text = format!("{camp}\nTent\nStove");
        titled(&mut lib, "N", camp, &text, 100);
        record_note_save(&mut lib, "N", &format!("{text} and lamp"), None, None, None).unwrap();
        assert_eq!(lib.note("N").unwrap().unwrap().title, camp);
        titled(&mut lib, "N", camp, &text, 100);
        replay(&mut lib, NOTES).unwrap();
        assert_eq!(
            lib.note("N").unwrap().unwrap().title,
            camp,
            "laid back over a pass"
        );
        record_note_save(&mut lib, "N", "Camping\nTent\nStove", None, None, None).unwrap();
        assert_eq!(lib.note("N").unwrap().unwrap().title, "Camping");
    }

    /// A note edited here keeps its time edited while the save waits: a pass bringing Apple's
    /// older time does not drop it back down the list until the save is sent. A save that
    /// changes nothing moves nothing.
    #[test]
    fn a_waiting_save_keeps_its_time_edited_across_a_pass() {
        let mut lib = Library::in_memory().unwrap();
        titled(&mut lib, "N", "Groceries", "Groceries\nMilk", 100);
        titled(&mut lib, "M", "Errands", "Errands\nBank", 200);
        record_note_save(&mut lib, "M", "Errands\nBank", None, None, None).unwrap();
        assert_eq!(
            lib.note("M").unwrap().unwrap().modified_at,
            Some(200),
            "nothing changed, nothing moved"
        );
        record_note_save(&mut lib, "N", "Groceries\nMilk\nEggs", None, None, None).unwrap();
        let edited = lib.note("N").unwrap().unwrap().modified_at.unwrap();
        assert!(edited > 200);
        // A pass before the send: Apple still has the old text and the old time.
        titled(&mut lib, "N", "Groceries", "Groceries\nMilk", 100);
        replay(&mut lib, NOTES).unwrap();
        let n = lib.note("N").unwrap().unwrap();
        assert_eq!(n.modified_at, Some(edited));
        assert_eq!(
            lib.note_body("N").unwrap().as_deref(),
            Some("Groceries\nMilk\nEggs")
        );
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
        replay(&mut lib, CONTACTS).unwrap();
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
        replay(&mut lib, CONTACTS).unwrap();
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
        replay(&mut lib, CONTACTS).unwrap();
        replay(&mut lib, CONTACTS).unwrap();
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
        replay(&mut lib, CONTACTS).unwrap();
        assert!(lib.contact_addresses("X").unwrap().is_empty());
    }

    /// Sent twice — the app died after Apple took it, before the queue heard — a change does
    /// what it did once: the create finds its own name and makes nothing more.
    #[test]
    fn a_create_replayed_after_a_crash_is_made_once() {
        tauri::async_runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let made = {
                let mut l = Library::open(dir.path()).unwrap();
                l.put_reminder_lists(&[ReminderList {
                    guid: "L".into(),
                    title: "L".into(),
                    enabled: true,
                    ..Default::default()
                }])
                .unwrap();
                let made = record_reminder_create(
                    &mut l,
                    &Create {
                        list_guid: "L".into(),
                        title: "Once".into(),
                        due_at: None,
                        all_day: None,
                        flagged: None,
                    },
                )
                .unwrap();
                // Taken and sent; the process dies before it is marked done.
                l.outbox_take(REMINDERS).unwrap();
                made
            };
            let lib =
                crate::library_lock::LibraryLock::new(Library::open(dir.path()).unwrap(), None);
            let pass = Mutex::new(());
            // Apple, as a set of names: a create of a name it holds makes nothing.
            let apple = Arc::new(StdMutex::new(std::collections::HashSet::from([
                made.clone()
            ])));
            let creates = Arc::new(StdMutex::new(0));
            let (a2, c2) = (apple.clone(), creates.clone());
            let report = drain(&lib, REMINDERS, &pass, move |p| {
                let (apple, creates) = (a2.clone(), c2.clone());
                async move {
                    if apple.lock().unwrap().insert(p.record.clone()) {
                        *creates.lock().unwrap() += 1;
                    }
                    Ok(())
                }
            })
            .await;
            assert!(report.sent);
            assert_eq!(
                *creates.lock().unwrap(),
                0,
                "already at Apple: not made again"
            );
            assert_eq!(apple.lock().unwrap().len(), 1);
            assert!(lib
                .lock()
                .await
                .outbox_pending(REMINDERS)
                .unwrap()
                .is_empty());
        });
    }

    // --- The kinds added for everyday offline edits.

    fn folders(lib: &mut Library) {
        use quince_core::notes::{NoteFolder, TRASH_FOLDER};
        let f = |g: &str| NoteFolder {
            guid: g.into(),
            title: g.into(),
            sort_order: 0,
            parent_guid: None,
        };
        lib.put_note_folders(&[
            f(quince_icloud::notes::DEFAULT_FOLDER),
            f(TRASH_FOLDER),
            f("F"),
        ])
        .unwrap();
    }

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

    /// Drain with a pretend Apple: every change is taken, unless `refuse` names its kind.
    async fn send_all(
        lib: &crate::library_lock::LibraryLock,
        service: &str,
        refuse: &'static str,
    ) -> Report {
        let pass = Mutex::new(());
        drain(lib, service, &pass, move |p| async move {
            if p.kind == refuse {
                Err(Error::Refused("turned down".into()))
            } else {
                Ok(())
            }
        })
        .await
    }

    /// The last tick before End Repeat: completed here at once, never moved past its end; and
    /// one with time left still moves on.
    #[test]
    fn the_last_tick_before_end_repeat_completes_it() {
        let mut lib = library();
        let due = 4_102_444_800; // 2100-01-01 00:00 UTC
        let mut daily = lib.reminder("a").unwrap().unwrap();
        daily.due_at = Some(due);
        daily.recur_frequency = Some(0);
        daily.recur_interval = Some(1);
        daily.timezone = Some("UTC".into());
        daily.raw = Some(
            r#"{"fields":{"RecurrenceRuleIDs":{"type":"STRING_LIST","value":["R1"]}}}"#.into(),
        );
        lib.put_reminders(std::slice::from_ref(&daily)).unwrap();
        let rule = |end: i64| quince_core::reminders::ReminderPart {
            name: "RecurrenceRule/R1".into(),
            reminder_guid: Some("a".into()),
            kind: "RecurrenceRule".into(),
            raw: format!(r#"{{"fields":{{"EndDate":{{"type":"TIMESTAMP","value":{end}}}}}}}"#),
        };
        // Ends that evening: tomorrow is past it.
        lib.put_reminder_parts(&[rule((due + 12 * 3600) * 1000)], &[], true)
            .unwrap();
        record_reminder_advance(&mut lib, &daily, 1_000).unwrap();
        let after = lib.reminder("a").unwrap().unwrap();
        assert_eq!(after.due_at, Some(due), "not moved past its end");
        assert_eq!(
            after.completed_at,
            Some(1_000),
            "completed, as on the phone"
        );

        // Ends two days on: moved on, open.
        let mut lib = library();
        lib.put_reminders(std::slice::from_ref(&daily)).unwrap();
        lib.put_reminder_parts(&[rule((due + 2 * 86_400) * 1000)], &[], true)
            .unwrap();
        record_reminder_advance(&mut lib, &daily, 1_000).unwrap();
        let after = lib.reminder("a").unwrap().unwrap();
        assert_eq!(
            (after.due_at, after.completed_at),
            (Some(due + 86_400), None)
        );
    }

    /// Weekdays, as the phone wrote "Repeating 1": ticked on a Friday, it moves on to Monday.
    #[test]
    fn a_weekdays_tick_on_friday_moves_on_to_monday() {
        let mut lib = library();
        let friday = 1_792_771_200; // Fri 23 Oct 2026, 00:00 UTC
        let mut r = lib.reminder("a").unwrap().unwrap();
        r.due_at = Some(friday);
        r.recur_frequency = Some(1);
        r.recur_interval = Some(1);
        r.timezone = Some("UTC".into());
        r.raw = Some(r#"{"fields":{"RecurrenceRuleIDs":{"type":"STRING_LIST","value":["EC08DD7D-132C-412C-AFDD-064AAECBA6A9"]}}}"#.into());
        lib.put_reminders(std::slice::from_ref(&r)).unwrap();
        lib.put_reminder_parts(
            &[quince_core::reminders::ReminderPart {
                name: "RecurrenceRule/EC08DD7D-132C-412C-AFDD-064AAECBA6A9".into(),
                reminder_guid: Some("a".into()),
                kind: "RecurrenceRule".into(),
                raw: r#"{"fields":{"DaysOfTheWeek":{"type":"BYTES","value":"W3siZGF5T2ZUaGVXZWVrIjoyLCJ3ZWVrTnVtYmVyIjowfSx7ImRheU9mVGhlV2VlayI6Mywid2Vla051bWJlciI6MH0seyJkYXlPZlRoZVdlZWsiOjQsIndlZWtOdW1iZXIiOjB9LHsiZGF5T2ZUaGVXZWVrIjo1LCJ3ZWVrTnVtYmVyIjowfSx7ImRheU9mVGhlV2VlayI6Niwid2Vla051bWJlciI6MH1d"},"Frequency":{"type":"NUMBER_INT64","value":1}}}"#.into(),
            }],
            &[],
            true,
        )
        .unwrap();
        record_reminder_advance(&mut lib, &r, friday - 60).unwrap();
        assert_eq!(
            lib.reminder("a").unwrap().unwrap().due_at,
            Some(friday + 3 * 86_400),
            "Monday, not next Friday"
        );
    }

    /// A repeating reminder ticked moves on to its next date at once, open; a pass from before
    /// the tick is laid over once, never twice; two ticks are two days, sent in order.
    #[test]
    fn a_repeating_tick_moves_on_once_and_two_ticks_are_two_days() {
        let mut lib = library();
        let day = 86_400;
        let due = 4_102_444_800; // 2100-01-01, so "next" is one step on
        let mut daily = lib.reminder("a").unwrap().unwrap();
        daily.due_at = Some(due);
        daily.recur_frequency = Some(0);
        daily.recur_interval = Some(1);
        daily.timezone = Some("UTC".into());
        lib.put_reminders(std::slice::from_ref(&daily)).unwrap();

        record_reminder_advance(&mut lib, &daily, 1_000).unwrap();
        let after = lib.reminder("a").unwrap().unwrap();
        assert_eq!(after.due_at, Some(due + day), "on to tomorrow, at once");
        assert!(after.completed_at.is_none(), "and open again");
        // Laid again (a replay with nothing new from Apple): not moved on twice.
        replay_reminders(&mut lib).unwrap();
        assert_eq!(lib.reminder("a").unwrap().unwrap().due_at, Some(due + day));

        // Ticked again, deliberately: a second day.
        record_reminder_advance(&mut lib, &after, 1_001).unwrap();
        assert_eq!(
            lib.reminder("a").unwrap().unwrap().due_at,
            Some(due + 2 * day)
        );
        let p = lib.outbox_pending(REMINDERS).unwrap();
        let froms: Vec<Option<i64>> = p
            .iter()
            .map(|x| {
                serde_json::from_str::<Advance>(&x.payload)
                    .unwrap()
                    .from_due
            })
            .collect();
        assert_eq!(
            froms,
            vec![Some(due), Some(due + day)],
            "each from its own day, in order"
        );

        // A pass reads Apple's copy from before either: both are laid back, once each.
        lib.put_reminders(&[
            daily.clone(),
            lib.reminder("b").unwrap().unwrap(),
            lib.reminder("c").unwrap().unwrap(),
        ])
        .unwrap();
        replay_reminders(&mut lib).unwrap();
        assert_eq!(
            lib.reminder("a").unwrap().unwrap().due_at,
            Some(due + 2 * day)
        );
        // Apple has taken the first: its copy is one day on; laid over, still two, not three.
        let mut moved = daily.clone();
        moved.due_at = Some(due + day);
        lib.put_reminder_local(&moved).unwrap();
        let first = lib.outbox_pending(REMINDERS).unwrap()[0].id;
        lib.outbox_done(first).unwrap();
        replay_reminders(&mut lib).unwrap();
        assert_eq!(
            lib.reminder("a").unwrap().unwrap().due_at,
            Some(due + 2 * day)
        );
    }

    /// Notes made, moved, pinned and deleted offline: each shows at once, a pass from before
    /// them all is laid over with them in order, and they go in the order they were made.
    #[test]
    fn note_changes_show_at_once_survive_a_pass_and_go_in_order() {
        let mut lib = Library::in_memory().unwrap();
        folders(&mut lib);
        note(&mut lib, "N", "Old note");
        let made = record_note_create(&mut lib, Some("F".into()), "Packing\npassport").unwrap();
        assert_eq!(made.len(), 36, "named here, as Apple names one: {made}");
        assert_eq!(made, made.to_uppercase());
        let shown = lib.note(&made).unwrap().unwrap();
        assert_eq!(
            (shown.title.as_str(), shown.folder_guid.as_deref()),
            ("Packing", Some("F"))
        );

        record_note_pin(&mut lib, &made, true).unwrap();
        record_note_move(&mut lib, "N", Some("F".into())).unwrap();
        record_note_move(&mut lib, "N", None).unwrap();
        let n = lib.note("N").unwrap().unwrap();
        record_note_delete(&mut lib, &n).unwrap();
        assert!(lib.note(&made).unwrap().unwrap().pinned);
        assert_eq!(
            lib.note("N").unwrap().unwrap().folder_guid.as_deref(),
            Some(quince_core::notes::TRASH_FOLDER),
            "to Recently Deleted"
        );
        assert_eq!(
            kinds(&lib, NOTES),
            vec![
                (made.clone(), "create".into()),
                (made.clone(), "pin".into()),
                ("N".into(), "move".into()),
                ("N".into(), "delete".into()),
            ],
            "two moves not yet sent are one, the last"
        );
        let mv: NoteMove =
            serde_json::from_str(&lib.outbox_pending(NOTES).unwrap()[2].payload).unwrap();
        assert_eq!(mv.folder_guid, None);

        // A full pass from before any of it: the new note is not there, N where it was.
        lib.retain_notes(&["N".into()]).unwrap();
        note(&mut lib, "N", "Old note");
        replay(&mut lib, NOTES).unwrap();
        let back = lib.note(&made).unwrap().unwrap();
        assert!(back.pinned && back.folder_guid.as_deref() == Some("F"));
        assert_eq!(
            lib.note("N").unwrap().unwrap().folder_guid.as_deref(),
            Some(quince_core::notes::TRASH_FOLDER)
        );

        // Deleted from Recently Deleted: gone here at once.
        let trashed = lib.note("N").unwrap().unwrap();
        record_note_delete(&mut lib, &trashed).unwrap();
        assert!(lib.note("N").unwrap().is_none());
        let last: NoteDelete =
            serde_json::from_str(&lib.outbox_pending(NOTES).unwrap().last().unwrap().payload)
                .unwrap();
        assert_eq!(
            last.from_folder.as_deref(),
            Some(quince_core::notes::TRASH_FOLDER),
            "for good"
        );
    }

    /// Recorded, then the app dies: reopened, every new kind is there in order, laid over the
    /// library still, and each goes once — the create already at Apple is not made again.
    #[test]
    fn new_kinds_survive_a_crash_between_record_and_send_and_go_once() {
        tauri::async_runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let (made, card) = {
                let mut l = Library::open(dir.path()).unwrap();
                folders(&mut l);
                calendars(&mut l);
                l.put_contacts(&[person("X", "Asha")]).unwrap();
                l.put_events(&[event("E", "Dentist", 1_000_000)]).unwrap();
                let made = record_note_create(&mut l, None, "Offline thought").unwrap();
                let x = l.contacts(None).unwrap().remove(0);
                record_contact_delete(&mut l, &x).unwrap();
                let e = l.event("E").unwrap().unwrap();
                record_event_delete(&mut l, &e, None).unwrap();
                // Taken and sent; the process dies before it is marked done.
                l.outbox_take(NOTES).unwrap();
                (made, x.guid)
            };
            let lib =
                crate::library_lock::LibraryLock::new(Library::open(dir.path()).unwrap(), None);
            {
                let l = lib.lock().await;
                assert!(
                    l.note(&made).unwrap().is_some(),
                    "still shown after the restart"
                );
                assert!(l.contacts(None).unwrap().is_empty());
                assert!(l.event("E").unwrap().is_none());
                assert_eq!(
                    l.outbox_pending(NOTES).unwrap()[0].tries,
                    1,
                    "counted as tried"
                );
            }
            // Apple, as a set of names: what it holds already is not made again.
            let apple = Arc::new(StdMutex::new(std::collections::HashSet::from([
                made.clone()
            ])));
            let made_again = Arc::new(StdMutex::new(Vec::new()));
            let pass = Mutex::new(());
            for service in [NOTES, CONTACTS, CALENDAR] {
                let (a, m) = (apple.clone(), made_again.clone());
                let report = drain(&lib, service, &pass, move |p| {
                    let (a, m) = (a.clone(), m.clone());
                    async move {
                        if p.kind == "create" && a.lock().unwrap().insert(p.record.clone()) {
                            m.lock().unwrap().push(p.record);
                        }
                        Ok(())
                    }
                })
                .await;
                assert!(report.sent && report.outcome.is_clear());
            }
            assert!(
                made_again.lock().unwrap().is_empty(),
                "already at Apple: made once"
            );
            assert_eq!(
                lib.lock().await.outbox_count().unwrap(),
                0,
                "each sent once"
            );
            let _ = card;
        });
    }

    /// One Apple turns down is kept — never dropped — with what it held, and the rest go.
    #[test]
    fn a_refused_change_of_each_new_kind_is_kept_with_what_it_held() {
        tauri::async_runtime::block_on(async {
            let mut l = Library::in_memory().unwrap();
            folders(&mut l);
            calendars(&mut l);
            note(&mut l, "N", "Shopping");
            l.put_contacts(&[person("X", "Asha")]).unwrap();
            l.put_events(&[event("E", "Dentist", 1_000_000)]).unwrap();
            let n = l.note("N").unwrap().unwrap();
            record_note_delete(&mut l, &n).unwrap();
            record_note_create(&mut l, None, "Kept words").unwrap();
            let x = l.contacts(None).unwrap().remove(0);
            record_contact_delete(&mut l, &x).unwrap();
            let mut e = l.event("E").unwrap().unwrap();
            e.title = "Dentist, moved".into();
            let base = l.event("E").unwrap();
            record_event_save(
                &mut l,
                EventSave {
                    event: e.clone(),
                    base,
                    create: false,
                    alerts: None,
                    repeat_changed: false,
                    repeat: None,
                    scope: None,
                },
            )
            .unwrap();
            let lib = crate::library_lock::LibraryLock::new(l, None);
            for (service, kind) in [(NOTES, "create"), (CONTACTS, "delete"), (CALENDAR, "save")] {
                let report = send_all(&lib, service, kind).await;
                assert_eq!(report.refused.len(), 1, "{service} {kind}");
                assert!(report.outcome.is_clear(), "the rest went");
            }
            let l = lib.lock().await;
            let st = status(&l).unwrap();
            assert!(st.waiting.is_empty());
            let held: Vec<(String, String, String)> = st
                .refused
                .into_iter()
                .map(|r| (r.service, r.kind, r.content))
                .collect();
            assert_eq!(
                held,
                vec![
                    (NOTES.into(), "create".into(), "Kept words".into()),
                    (CONTACTS.into(), "delete".into(), "Asha".into()),
                    (CALENDAR.into(), "save".into(), "Dentist, moved".into()),
                ]
            );
        });
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
        replay(&mut lib, CONTACTS).unwrap();
        assert_eq!(
            names(&lib),
            vec!["Y".to_string()],
            "the pass does not bring it back"
        );
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

    /// The sender wakes when the network comes back — once per return, not on every report of
    /// "still online" — and when told to by the window.
    #[test]
    fn the_sender_wakes_when_the_network_comes_back() {
        let mut was = None;
        assert!(came_online(&mut was, nm_online(70)), "first word: online");
        assert!(
            !came_online(&mut was, nm_online(70)),
            "still online: no new wake"
        );
        assert!(!came_online(&mut was, nm_online(20)), "gone offline");
        assert!(!came_online(&mut was, nm_online(40)), "connecting");
        assert!(came_online(&mut was, nm_online(60)), "back");
        let mut was = None;
        assert!(!came_online(&mut was, connman_online("idle")));
        assert!(came_online(&mut was, connman_online("ready")));
        assert!(!came_online(&mut was, connman_online("online")));
        assert!(!came_online(&mut was, connman_online("offline")));
        assert!(came_online(&mut was, connman_online("online")));

        // `outbox_wake` (the window's `online` event) and every trigger above end in a kick,
        // which wakes a sender waiting out its backoff at once.
        tauri::async_runtime::block_on(async {
            outbox_wake();
            tokio::time::timeout(Duration::from_secs(1), WAKE.notified())
                .await
                .expect("the sender is woken");
        });
    }

    #[test]
    fn an_edit_round_trips_clear_and_leave() {
        let e = Edit {
            notes: Some(None),
            due_at: Some(Some(5)),
            parent_guid: None,
            from_list: "L".into(),
            ..Default::default()
        };
        let back: Edit = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    /// A tick, a pin or a move for a record the library does not hold is dropped (ticks, pins)
    /// or refused (moves) only once a pass has said what the library holds: in a library made
    /// again after damage, before its first pass, the record is not known yet, and the change
    /// waits — never quietly "sent" and lost.
    #[tokio::test]
    async fn a_change_for_a_record_not_known_yet_waits_for_a_pass() {
        let lib = crate::library_lock::LibraryLock::new(Library::in_memory().unwrap(), None);
        let http = reqwest::Client::new();
        // Nothing listens here: any request at all fails as a network error, not Busy.
        let nowhere = "http://127.0.0.1:9";
        let pending = |service: &str, kind: &str, payload: String| Pending {
            id: 1,
            service: service.into(),
            record: "GONE".into(),
            kind: kind.into(),
            payload,
            tries: 0,
        };
        let tick = || {
            let d = Done {
                done: true,
                at: 1,
                base: Default::default(),
            };
            pending(REMINDERS, "done", serde_json::to_string(&d).unwrap())
        };
        let pin = || pending(NOTES, "pin", r#"{"pinned":true}"#.into());
        let moved = || pending(NOTES, "move", r#"{"folder_guid":null}"#.into());

        let waits = |r: Result<(), Error>| matches!(r, Err(e) if transient(&e, 99));
        assert!(waits(
            send_reminder(&lib, &http, nowhere, Vec::new(), tick()).await
        ));
        assert!(waits(
            send_note(&lib, &http, nowhere, Vec::new(), pin()).await
        ));
        assert!(waits(
            send_note(&lib, &http, nowhere, Vec::new(), moved()).await
        ));

        // A pass in: the record is gone, so a tick or a pin has nothing left to do, and a move
        // says why it did not happen.
        {
            let l = lib.lock().await;
            l.mark_synced(REMINDERS, Some("t")).unwrap();
            l.mark_synced(NOTES, Some("t")).unwrap();
        }
        assert!(send_reminder(&lib, &http, nowhere, Vec::new(), tick())
            .await
            .is_ok());
        assert!(send_note(&lib, &http, nowhere, Vec::new(), pin())
            .await
            .is_ok());
        assert!(matches!(
            send_note(&lib, &http, nowhere, Vec::new(), moved()).await,
            Err(Error::Refused(_))
        ));
    }

    /// The sender sends a library's changes only while the library is the account's whose
    /// session it holds: asked as each change is taken. Another account's library — swapped in
    /// by a sign-in while the queue was going — keeps every change, unsent.
    #[tokio::test]
    async fn changes_go_only_with_their_own_accounts_session() {
        let mut l = library();
        queue(&mut l, "R1", true);
        queue(&mut l, "R2", true);
        l.set_state(crate::ACCOUNT_KEY, "b").unwrap();
        let lib = crate::library_lock::LibraryLock::new(l, None);
        let sent = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pass = Mutex::new(());
        let count = std::sync::Arc::clone(&sent);
        let report = drain_as(&lib, REMINDERS, &pass, "a", move |_p| {
            let count = std::sync::Arc::clone(&count);
            async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        })
        .await;
        assert!(matches!(report.outcome, Outcome::Later));
        assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(lib.lock().await.outbox_pending(REMINDERS).unwrap().len(), 2);

        // Swapped part way: the first goes with its own account, the rest wait.
        lib.lock().await.set_state(crate::ACCOUNT_KEY, "a").unwrap();
        let lib = std::sync::Arc::new(lib);
        let (inner, count) = (std::sync::Arc::clone(&lib), std::sync::Arc::clone(&sent));
        let report = drain_as(&lib, REMINDERS, &pass, "a", move |_p| {
            let (lib, count) = (std::sync::Arc::clone(&inner), std::sync::Arc::clone(&count));
            async move {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                lib.lock().await.set_state(crate::ACCOUNT_KEY, "b").unwrap();
                Ok(())
            }
        })
        .await;
        assert!(matches!(report.outcome, Outcome::Later));
        assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(lib.lock().await.outbox_pending(REMINDERS).unwrap().len(), 1);
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

        let mut l = library();
        for (id, name) in [("A", "Ada"), ("B", "Bea"), ("C", "Cy")] {
            let was = person(id, id);
            l.put_contacts(std::slice::from_ref(&was)).unwrap();
            record_contact_save(&mut l, &person(id, name), Some(was)).unwrap();
        }
        // Behind a save of A that is on its way: a second edit of A, sent after it.
        l.outbox_take(CONTACTS).unwrap();
        record_contact_save(&mut l, &person("A", "Ada L"), Some(person("A", "Ada"))).unwrap();
        l.set_state(crate::ACCOUNT_KEY, "me").unwrap();
        let lib = crate::library_lock::LibraryLock::new(l, None);
        let http = reqwest::Client::new();
        let book = std::sync::Mutex::new(Book::default());
        let ledger = quince_icloud::contacts::Ledger::new();
        let root = server.uri();
        let report = drain_as(&lib, CONTACTS, &Mutex::new(()), "me", |p| {
            send_contact(&lib, &http, &root, Vec::new(), p, &book, &ledger)
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
                let body =
                    serde_json::json!({ "Event": [apple_json(&self.0.event(), &self.0.tag())] });
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

        async fn send_waiting(lib: &crate::library_lock::LibraryLock, root: &str) {
            let http = reqwest::Client::new();
            let report = drain(lib, CALENDAR, &Mutex::new(()), |p| {
                send_event(lib, &http, root, Vec::new(), p)
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
    }
}

#[cfg(test)]
mod held_tests {
    use super::*;

    #[test]
    fn changes_wait_for_the_next_approval_and_go_with_it() {
        let waiting = ["notes", "reminders", "photos", "drive"];
        assert!(held(NOTES, &waiting) && held(REMINDERS, &waiting));
        assert!(
            !held(CONTACTS, &waiting) && !held(CALENDAR, &waiting),
            "never held"
        );
        assert!(!held(NOTES, &[]), "approved: sent");
    }

    /// Contacts waiting in the queue go one after another, each write moving the book's sync
    /// token on. Against an Apple that refuses a spent token, every one lands — whether Apple
    /// answers a write with its new tokens or not — and none is sent with the token an
    /// earlier one spent.
    #[test]
    fn queued_contact_writes_never_send_a_spent_token() {
        use crate::contacts_merge::tests::{apple_stub, person};
        use serde_json::json;
        for answers_tokens in [false, true] {
            tauri::async_runtime::block_on(async {
                let server = wiremock::MockServer::start().await;
                let apple = apple_stub::Apple(std::sync::Arc::new(std::sync::Mutex::new(
                    apple_stub::Book {
                        cards: vec![
                            json!({ "contactId": "A", "etag": "ea", "firstName": "Zoya" }),
                            json!({ "contactId": "B", "etag": "eb", "firstName": "Ravi" }),
                            json!({ "contactId": "C", "etag": "ec", "firstName": "Old" }),
                        ],
                        answers_tokens,
                        ..Default::default()
                    },
                )));
                wiremock::Mock::given(wiremock::matchers::any())
                    .respond_with(apple.clone())
                    .mount(&server)
                    .await;
                let mut l = Library::in_memory().unwrap();
                let (a, b, c) = (
                    person("A", "Zoya", "+91 90000 00001").contact,
                    person("B", "Ravi", "+91 90000 00002").contact,
                    person("C", "Old", "+91 90000 00003").contact,
                );
                l.put_contacts(&[a.clone(), b.clone(), c.clone()]).unwrap();
                let mut a2 = a.clone();
                a2.first_name = Some("Zoya R".into());
                let mut b2 = b.clone();
                b2.first_name = Some("Ravi K".into());
                record_contact_save(&mut l, &a2, Some(a)).unwrap();
                record_contact_save(&mut l, &b2, Some(b)).unwrap();
                record_contact_delete(&mut l, &c).unwrap();
                let lib = crate::library_lock::LibraryLock::new(l, None);
                let http = reqwest::Client::new();
                let ledger = quince_icloud::contacts::Ledger::new();
                let pass = Mutex::new(());
                let uri = server.uri();
                let book = std::sync::Mutex::new(Book::default());
                let report = drain(&lib, CONTACTS, &pass, |p| {
                    send_contact(&lib, &http, &uri, vec![], p, &book, &ledger)
                })
                .await;
                assert!(lib
                    .lock()
                    .await
                    .outbox_pending(CONTACTS)
                    .unwrap()
                    .is_empty());
                let book = apple.0.lock().unwrap();
                assert!(report.refused.is_empty(), "{:?}", report.refused);
                assert!(
                    !book.writes.iter().any(|w| w.starts_with("refused")),
                    "answers_tokens {answers_tokens}: {:?}",
                    book.writes
                );
                assert_eq!(book.writes.len(), 3, "{:?}", book.writes);
                let first = |id: &str| {
                    book.cards
                        .iter()
                        .find(|c| c["contactId"] == id)
                        .map(|c| c["firstName"].clone())
                };
                assert_eq!(first("A"), Some(json!("Zoya R")));
                assert_eq!(first("B"), Some(json!("Ravi K")));
                assert_eq!(first("C"), None);
            });
        }
    }
}
