//! Calendar's changes made here: recorded, laid over the library and sent, by the code the
//! desktop's outbox worker and the phone's engine both run.
//!
//! The desktop records a change and lays it over its library in one transaction
//! ([`record_event_save`], [`record_event_delete`]); each pass lays what still waits back over
//! what it read ([`replay`]). The phone's change is already shown in Android's own Calendar, so
//! it is only queued ([`queue_phone_save`], [`queue_phone_delete`]), as the state its row is in
//! now, on every pass until Apple has it.
//!
//! Sending ([`send`]) needs nothing but the change itself: an edit goes against the version it
//! was made from and, refused as changed since, is merged with Apple's copy as it is now
//! (`save_event_merging`, which finds the event by its id wherever it is, however far from
//! today); a delete goes against the version it was made from and, refused, against Apple's
//! newest. Nothing depends on which days were last read.

use crate::library_lock::LibraryLock;
use crate::queue::{json, last_word, unreadable};
use quince_core::calendar::Event;
use quince_core::outbox::Pending;
use quince_core::Library;
use quince_icloud::calendar::{EventRepeat, Scope};
use quince_icloud::Error;
use serde::{Deserialize, Serialize};

pub const CALENDAR: &str = "calendar";

/// Where a calendar account's queue is named: one each, so a server that is down holds up only
/// its own changes, never iCloud's or another account's.
pub const ACCOUNT_QUEUE: &str = "caldav:";

/// The queue of the calendar account `id`.
pub fn account_queue(id: &str) -> String {
    format!("{ACCOUNT_QUEUE}{id}")
}

/// The queue a change to this calendar waits in.
pub fn queue_of(calendar_guid: &str) -> String {
    quince_core::dav::account_of(calendar_guid).map_or_else(|| CALENDAR.to_string(), account_queue)
}

/// What a queue is to the person, who knows a change as the Calendar's whichever account it
/// goes to.
pub fn service_shown(service: &str) -> &str {
    if service.starts_with(ACCOUNT_QUEUE) {
        CALENDAR
    } else {
        service
    }
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

// --- Recording: the desktop's commands. Each records the change and lays it over the library
// as one transaction, so a crash leaves both or neither.

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

/// Lay one waiting change to an event over the library, whichever account's queue it is in.
pub fn lay(l: &mut Library, p: &Pending) -> quince_core::Result<()> {
    match p.kind.as_str() {
        "save" => {
            if let Ok(s) = serde_json::from_str::<EventSave>(&p.payload) {
                lay_event(l, &s, true)?;
            }
        }
        "delete" => {
            if let Ok(d) = serde_json::from_str::<EventDelete>(&p.payload) {
                lay_event_delete(l, &d)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Lay every change still waiting in `queue` over what a sync pass just stored, in the order
/// they were made.
pub fn replay(library: &mut Library, queue: &str) -> quince_core::Result<()> {
    for p in library.outbox_pending(queue)? {
        lay(library, &p)?;
    }
    Ok(())
}

/// Every change still waiting in `queue` but `except`, laid over the library: what Apple
/// answered for one just sent is stored, and what still waits behind it stays as made.
pub fn replay_except(library: &mut Library, queue: &str, except: i64) -> quince_core::Result<()> {
    for p in library.outbox_pending(queue)? {
        if p.id != except {
            lay(library, &p)?;
        }
    }
    Ok(())
}

// --- Queueing the phone's state: its rows are the change already shown, so nothing is laid.

/// An event as it is on the phone now (`save.create` for one made there, under the id the phone
/// gave it). Whether anything was queued: nothing while what waits already says exactly this.
/// What waits is followed as on the desktop: unsent and the last word, for the same events of a
/// series, it becomes one save; otherwise this goes behind it, made from what it sends.
pub fn queue_phone_save(l: &mut Library, save: EventSave) -> quince_core::Result<bool> {
    let guid = save.event.guid.clone();
    let newest = l.outbox_waiting(CALENDAR, &guid, "save")?.pop();
    let last = match &newest {
        Some(p) => last_word(l, CALENDAR, &guid, "save")?.is_some_and(|w| w.id == p.id),
        None => false,
    };
    let waiting = newest.and_then(|p| {
        serde_json::from_str::<EventSave>(&p.payload)
            .ok()
            .map(|s| (p, s))
    });
    let save = match waiting {
        Some((_, before))
            if before.event == save.event
                && before.scope == save.scope
                && (!save.repeat_changed || before.repeat == save.repeat) =>
        {
            return Ok(false);
        }
        Some((p, before)) if last && p.tries == 0 && before.scope == save.scope => {
            let merged = before.then(save);
            if l.outbox_amend(p.id, &json(&merged)?)? {
                return Ok(true);
            }
            merged
        }
        Some((_, before)) => EventSave {
            base: Some(before.event),
            create: false,
            ..save
        },
        None => save,
    };
    l.outbox_append(CALENDAR, &guid, "save", &json(&save)?)?;
    Ok(true)
}

/// An event deleted on the phone, as it last was. Queued once.
pub fn queue_phone_delete(
    l: &mut Library,
    event: &Event,
    scope: Option<Scope>,
) -> quince_core::Result<bool> {
    if last_word(l, CALENDAR, &event.guid, "delete")?.is_some() {
        return Ok(false);
    }
    let d = EventDelete {
        event: event.clone(),
        scope,
    };
    l.outbox_append(CALENDAR, &event.guid, "delete", &json(&d)?)?;
    Ok(true)
}

// --- Sending.

/// One queued change to an iCloud calendar sent. Answers Apple's own version of what it saved
/// (none for a delete), for the caller to keep: the desktop stores it in its library, the phone
/// writes it into the row as the row's new base.
pub async fn send(
    library: &LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: &Pending,
) -> Result<Vec<Event>, Error> {
    use quince_icloud::calendar::CalendarClient;
    use quince_icloud::sync::{calendar_ctag, save_event_merging};
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
            save_event_merging(
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
            .await
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
                        return Ok(Vec::new());
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
                        .map(|()| Vec::new())
                }
                other => other.map(|()| Vec::new()),
            }
        }
        other => Err(Error::Refused(format!(
            "Skykeep does not know how to send “{other}”."
        ))),
    }
}

#[cfg(test)]
mod tests;
