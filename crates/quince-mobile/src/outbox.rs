//! Changes Apple turned down, kept for the person, as the desktop keeps them: what was typed,
//! Apple's reason, and the choice to send it again or let it go. Every service's refusals are
//! here — Contacts and Calendar as much as Notes and Reminders — so nothing made on the phone is
//! dropped without the person seeing it.

use crate::{failed, runtime, Account, EngineError};
use quince_core::Library;

/// A change Apple refused, kept.
#[derive(uniffi::Record, Debug, PartialEq, Eq)]
pub struct NotSent {
    pub id: i64,
    /// `contacts`, `calendar`, `reminders` or `notes`.
    pub service: String,
    pub kind: String,
    /// What was typed: the person's name, the event's or reminder's title, the note's text.
    pub what: String,
    pub reason: String,
    /// Unix seconds.
    pub made_at: i64,
}

fn not_sent(l: &Library) -> quince_core::Result<Vec<NotSent>> {
    Ok(l.outbox_refused()?
        .into_iter()
        .map(|r| NotSent {
            id: r.change.id,
            service: quince_sync::calendar::service_shown(&r.change.service).to_string(),
            what: quince_sync::queue::content_of(&r.change),
            kind: r.change.kind,
            reason: r.reason,
            made_at: r.made_at,
        })
        .collect())
}

/// A refused change queued again, at the back. Notes, Reminders and Photos live in this
/// library, so the change is laid over it again as it goes, as on the desktop; Contacts and
/// Calendar live in Android's own providers and are never written into the library.
fn resend(l: &mut Library, id: i64) -> quince_core::Result<bool> {
    use quince_sync::{notes, photos, reminders};
    l.atomically(|l| {
        if !l.outbox_resend(id)? {
            return Ok(false);
        }
        if let Some(p) = l.outbox_all_pending()?.pop() {
            if p.service == reminders::REMINDERS {
                reminders::lay(l, &p)?;
            } else if p.service == notes::NOTES {
                notes::lay(l, &p)?;
            } else if p.service == photos::PHOTOS {
                photos::lay(l, &p)?;
            }
        }
        Ok(true)
    })
}

/// A refused change let go of. A photo a refused delete-for-good set aside is put back, as on
/// the desktop.
fn discard(l: &mut Library, id: i64) -> quince_core::Result<()> {
    use quince_sync::photos;
    l.atomically(|l| {
        let photo = l
            .outbox_refused_one(id)?
            .is_some_and(|r| r.change.service == photos::PHOTOS);
        l.outbox_discard(id)?;
        if photo {
            photos::replay(l)?;
        }
        Ok(())
    })
}

#[uniffi::export]
impl Account {
    /// Every change Apple turned down, oldest first.
    pub fn outbox_refused(&self) -> Result<Vec<NotSent>, EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async { not_sent(&*lock.lock().await).map_err(failed) })
    }

    /// A refused change sent again on its service's next sync. The service, so the app can
    /// start that sync; none when it was no longer there to send.
    pub fn outbox_resend(&self, id: i64) -> Result<Option<String>, EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async {
            let mut l = lock.lock().await;
            let service = l
                .outbox_refused_one(id)
                .map_err(failed)?
                .map(|r| quince_sync::calendar::service_shown(&r.change.service).to_string());
            Ok(resend(&mut l, id)
                .map_err(failed)?
                .then_some(service)
                .flatten())
        })
    }

    /// A refused change let go of: Apple's version stands, and comes back on the next sync.
    pub fn outbox_discard(&self, id: i64) -> Result<(), EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async { discard(&mut *lock.lock().await, id).map_err(failed) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_sync::contacts::{queue_phone_card, CONTACTS};

    /// A refused change made on the phone is listed with what was typed and why; sent again it
    /// is back in the queue, and nothing is laid into the library for a service the phone keeps
    /// in its own provider; let go of, it is gone.
    #[test]
    fn a_refused_phone_change_is_shown_sent_again_or_let_go() {
        let mut l = Library::in_memory().unwrap();
        let card = quince_core::contacts::Card {
            contact: quince_core::contacts::Contact {
                guid: "X".into(),
                first_name: Some("Asha".into()),
                ..crate::contact_of(&crate::contacts::blank(), "X")
            },
            details: Default::default(),
        };
        queue_phone_card(&mut l, &card, None, true).unwrap();
        let p = l.outbox_take(CONTACTS).unwrap().unwrap();
        l.outbox_refuse(p.id, "Turned down").unwrap();
        let shown = not_sent(&l).unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(
            (
                shown[0].service.as_str(),
                shown[0].what.as_str(),
                shown[0].reason.as_str()
            ),
            ("contacts", "Asha", "Turned down")
        );

        assert!(resend(&mut l, shown[0].id).unwrap());
        assert!(not_sent(&l).unwrap().is_empty());
        assert_eq!(l.outbox_pending(CONTACTS).unwrap().len(), 1, "queued again");
        assert!(l.contacts(None).unwrap().is_empty(), "nothing laid");

        let p = l.outbox_take(CONTACTS).unwrap().unwrap();
        l.outbox_refuse(p.id, "Turned down").unwrap();
        let id = not_sent(&l).unwrap()[0].id;
        discard(&mut l, id).unwrap();
        assert!(not_sent(&l).unwrap().is_empty());
        assert!(l.outbox_pending(CONTACTS).unwrap().is_empty());
    }
}
