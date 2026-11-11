//! Calendar on the phone, sent by the desktop's own code. Android's Calendar is the store: an
//! event changed there (its row marked dirty) is handed over here as it is now, against the
//! event as last read from Apple, queued in the engine's outbox and sent with
//! `quince_sync::calendar::send` — by its id, against the version it was made from, merged with
//! Apple's copy when that changed since — so an event far from today is edited and deleted as
//! one inside the span the phone reads. What Apple kept comes back for the row's new base.

use crate::contacts::{SendState, Settled};
use crate::{event_of, failed, runtime, Account, ChangeKind, EngineError, Ev, Repeat};
use quince_icloud::calendar::{is_occurrence, CalendarClient, EventRepeat, Scope};
use quince_sync::calendar::{self as shared, EventSave, CALENDAR};
use std::collections::HashMap;
use tokio::sync::Mutex;

static PASS: Mutex<()> = Mutex::const_new(());

/// One event changed on the phone (or one occurrence of a repeating event, whose id is
/// `<series>__<start>`): as it is now, and as it was last read from Apple (`base`).
#[derive(uniffi::Record)]
pub struct PhoneEvent {
    pub kind: ChangeKind,
    pub guid: String,
    pub event: Option<Ev>,
    pub base: Option<Ev>,
    /// A new event's repeat, when the phone gave it one iCloud can keep.
    pub repeat: Option<Repeat>,
    /// Whether `base` says what the event's link was. A base kept before links were has none to
    /// say: the link is read from Apple before the change is queued, so it isn't lost.
    pub base_has_url: bool,
}

/// What sending came to: where each change is, and Apple's own version of each event it saved,
/// for the phone's rows to take as their base however far from today they are.
#[derive(uniffi::Record)]
pub struct CalendarSent {
    pub settled: Vec<Settled>,
    pub kept: Vec<Ev>,
}

pub(crate) fn ev_of(e: quince_core::calendar::Event) -> Ev {
    Ev {
        guid: e.guid,
        calendar_guid: e.calendar_guid,
        title: e.title,
        location: e.location,
        starts_at: e.starts_at,
        ends_at: e.ends_at,
        all_day: e.all_day,
        timezone: e.timezone,
        notes: e.notes,
        url: e.url,
        etag: e.etag,
    }
}

fn repeat_of(r: &Repeat) -> EventRepeat {
    EventRepeat {
        freq: r.freq.clone(),
        interval: r.interval,
        count: r.count,
        week_days: r.week_days.clone(),
        position: r.position.clone(),
        month_days: r.month_days.clone(),
        months: r.months.clone(),
        until: r.until.clone(),
    }
}

/// One occurrence of a repeating event is changed alone, as "This Event" on an iPhone.
fn scope_of(guid: &str) -> Option<Scope> {
    is_occurrence(guid).then_some(Scope::This)
}

/// The change as the desktop's outbox holds one. `base` is the phone's base with its link
/// filled in.
fn save_of(c: &PhoneEvent, base: Option<&Ev>) -> Result<EventSave, String> {
    let ev = c
        .event
        .as_ref()
        .ok_or_else(|| "This event has nothing in it.".to_string())?;
    if c.kind == ChangeKind::Create {
        let repeat = c.repeat.as_ref().map(repeat_of);
        // New: no version of Apple's to be made against.
        let mut event = event_of(ev, &c.guid, None);
        event.etag = None;
        return Ok(EventSave {
            event,
            base: None,
            create: true,
            alerts: None,
            repeat_changed: repeat.is_some(),
            repeat,
            scope: None,
        });
    }
    let base = base.map(|b| event_of(b, &c.guid, b.etag.clone()));
    let mut event = event_of(ev, &c.guid, base.as_ref().and_then(|b| b.etag.clone()));
    if let Some(b) = &base {
        // The link isn't on the phone: as the base had it, so it is left as Apple has it.
        event.url.clone_from(&b.url);
        // Apple moves a whole series, never one occurrence alone: it stays in its calendar.
        if is_occurrence(&c.guid) {
            event.calendar_guid.clone_from(&b.calendar_guid);
        }
    }
    Ok(EventSave {
        event,
        base,
        create: false,
        alerts: None,
        repeat_changed: false,
        repeat: None,
        scope: scope_of(&c.guid),
    })
}

#[uniffi::export]
impl Account {
    /// The phone's changes to events sent to iCloud, as the desktop sends its own: each queued
    /// in the engine's outbox, then sent by id, merged with what changed elsewhere. Where each
    /// is now, in the order given, and what Apple kept of each saved.
    pub fn calendar_send(&self, changes: Vec<PhoneEvent>) -> Result<CalendarSent, EngineError> {
        let (lock, account) = self.library()?;
        let service = self.service("calendar");
        let mut unqueued: HashMap<String, String> = HashMap::new();
        let mut later: Vec<String> = Vec::new();
        // A base kept before links were: Apple's link read first, by the event's id. Not read
        // now (offline), the change waits for the next pass rather than lose the link.
        let mut bases: HashMap<String, Ev> = HashMap::new();
        for c in &changes {
            let Some(base) = c.base.as_ref() else {
                continue;
            };
            if c.base_has_url || c.kind != ChangeKind::Update {
                continue;
            }
            let found = service.as_ref().map(|(http, root, params)| {
                let client = CalendarClient::new(http, root, params.clone());
                runtime().block_on(client.current_event(&event_of(base, &c.guid, None)))
            });
            match found {
                Some(Ok(theirs)) => {
                    let mut filled = base.clone();
                    filled.url = theirs.and_then(|t| t.url);
                    bases.insert(c.guid.clone(), filled);
                }
                _ => later.push(c.guid.clone()),
            }
        }
        runtime().block_on(async {
            let mut l = lock.lock().await;
            l.atomically(|l| {
                for c in &changes {
                    if later.contains(&c.guid) {
                        continue;
                    }
                    let queued = match c.kind {
                        ChangeKind::Delete => {
                            let was = c.base.as_ref().or(c.event.as_ref());
                            match was {
                                Some(e) => {
                                    let event = event_of(e, &c.guid, e.etag.clone());
                                    shared::queue_phone_delete(l, &event, scope_of(&c.guid))
                                        .map(|_| ())
                                        .map_err(|e| e.to_string())
                                }
                                None => Err("This event can't be found to delete.".into()),
                            }
                        }
                        ChangeKind::Create | ChangeKind::Update => {
                            let base = bases.get(&c.guid).or(c.base.as_ref());
                            save_of(c, base).and_then(|save| {
                                shared::queue_phone_save(l, save)
                                    .map(|_| ())
                                    .map_err(|e| e.to_string())
                            })
                        }
                    };
                    if let Err(why) = queued {
                        unqueued.insert(c.guid.clone(), why);
                    }
                }
                Ok::<_, quince_core::Error>(())
            })
            .map_err(failed)
        })?;
        let kept = std::sync::Mutex::new(Vec::new());
        let report = match &service {
            None => quince_sync::queue::Report {
                outcome: quince_sync::queue::Outcome::Later,
                sent: false,
                refused: Vec::new(),
            },
            Some((http, root, params)) => runtime().block_on(async {
                quince_sync::queue::drain_as(&lock, CALENDAR, &PASS, &account, |p| {
                    let (kept, lock) = (&kept, &lock);
                    async move {
                        let back = shared::send(lock, http, root, params.clone(), &p).await?;
                        kept.lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .extend(back);
                        Ok(())
                    }
                })
                .await
            }),
        };
        for (_, why) in &report.refused {
            crate::log::error(&crate::log::redact(why));
        }
        let pending = runtime()
            .block_on(async { lock.lock().await.outbox_pending(CALENDAR) })
            .map_err(failed)?;
        let settled = changes
            .iter()
            .map(|c| {
                let guid = c.guid.clone();
                let state = if let Some(reason) = unqueued.get(&guid) {
                    SendState::Refused {
                        reason: reason.clone(),
                    }
                } else if later.contains(&guid) || pending.iter().any(|p| p.record == guid) {
                    SendState::Waiting
                } else if let Some((_, reason)) =
                    report.refused.iter().rev().find(|(r, _)| *r == guid)
                {
                    SendState::Refused {
                        reason: reason.clone(),
                    }
                } else {
                    SendState::Sent
                };
                Settled { guid, state }
            })
            .collect();
        let kept = kept
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .into_iter()
            .map(ev_of)
            .collect();
        Ok(CalendarSent { settled, kept })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(guid: &str, calendar: &str, title: &str) -> Ev {
        Ev {
            guid: guid.into(),
            calendar_guid: calendar.into(),
            title: title.into(),
            location: None,
            starts_at: 1_900_000_000,
            ends_at: 1_900_003_600,
            all_day: false,
            timezone: Some("UTC".into()),
            notes: None,
            url: Some("https://example.com/meet".into()),
            etag: Some("v1".into()),
        }
    }

    /// An edit from the phone is the desktop's save: against the base's tag, the link the phone
    /// can't show kept as the base had it, one occurrence changed alone and never moved out of
    /// its series' calendar.
    #[test]
    fn a_phone_edit_is_the_desktops_save() {
        let base = ev("S__1900000000", "work", "Standup");
        let mut now = ev("S__1900000000", "home", "Standup, moved");
        now.url = None;
        now.etag = None;
        let c = PhoneEvent {
            kind: ChangeKind::Update,
            guid: base.guid.clone(),
            event: Some(now),
            base: Some(base.clone()),
            repeat: None,
            base_has_url: true,
        };
        let s = save_of(&c, c.base.as_ref()).unwrap();
        assert_eq!(s.event.etag.as_deref(), Some("v1"));
        assert_eq!(s.event.url, base.url);
        assert_eq!(s.event.calendar_guid, "work");
        assert_eq!(s.scope, Some(Scope::This));
        assert!(!s.create && !s.repeat_changed);

        let made = PhoneEvent {
            kind: ChangeKind::Create,
            guid: "NEW".into(),
            event: Some(ev("", "work", "Lunch")),
            base: None,
            repeat: None,
            base_has_url: true,
        };
        let s = save_of(&made, None).unwrap();
        assert!(s.create && s.base.is_none() && s.event.etag.is_none());
        assert_eq!(s.event.guid, "NEW");
    }
}
