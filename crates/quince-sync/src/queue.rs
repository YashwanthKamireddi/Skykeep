//! The queue of changes made here: each recorded and shown at once, sent behind, oldest first.
//! A change Apple can't be reached for waits, and everything after it with it, in order; one
//! Apple refuses is held as refused and said; one sent comes off the queue. Shared by the
//! desktop's worker and the phone's engine.

use crate::library_lock::LibraryLock;
use futures_util::FutureExt;
use quince_core::outbox::Pending;
use quince_core::Library;
use quince_icloud::Error;
use serde::Serialize;
use tokio::sync::Mutex;

/// Where the library names whose it is: the Apple Account it was made for.
pub const ACCOUNT_KEY: &str = "account";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing was waiting: the pass sent nothing and changed nothing.
    Idle,
    Clear,
    Later,
}

/// How many times an answer this app does not understand (or a conflict a merge did not
/// settle) is retried before the change is held as refused — a fault that will never clear
/// held every change queued after it forever, each shown as made.
pub const PROTOCOL_TRIES: i64 = 5;

/// Whether Apple could not be reached (keep it) rather than said no (hold it as refused).
/// `tries` counts this one.
pub fn transient(e: &Error, tries: i64) -> bool {
    match e {
        Error::Network(_)
        | Error::ReauthRequired
        | Error::StaleCursor
        | Error::Busy
        | Error::NeedsDeviceApproval
        | Error::Io(_)
        | Error::Storage(_) => true,
        Error::Protocol(_) | Error::Conflict => tries < PROTOCOL_TRIES,
        _ => false,
    }
}

impl Outcome {
    pub fn is_clear(&self) -> bool {
        matches!(self, Outcome::Clear)
    }
}

/// What one pass over the queue came to.
pub struct Report {
    pub outcome: Outcome,
    pub sent: bool,
    /// Each refused change: its record and Apple's reason.
    pub refused: Vec<(String, String)>,
}

/// [`drain_as`] without asking whose the library is: the tests' pretend Apple.
pub async fn drain<F, Fut>(
    library: &LibraryLock,
    service: &str,
    pass: &Mutex<()>,
    send: F,
) -> Report
where
    F: Fn(Pending) -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    drain_checked(library, service, pass, None, send).await
}

/// Send `service`'s queue with the session of `account` (as the library names accounts), for
/// as long as the library is still that account's: asked as each change is taken, under the
/// same lock, so a sign-in of another account while the queue was going — which swaps the
/// library under it — never has its changes sent with the session captured before.
pub async fn drain_as<F, Fut>(
    library: &LibraryLock,
    service: &str,
    pass: &Mutex<()>,
    account: &str,
    send: F,
) -> Report
where
    F: Fn(Pending) -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    drain_checked(library, service, pass, Some(account), send).await
}

/// Send what is waiting for `service`, oldest first, with `send`. Stops at the first change
/// Apple can't be reached for, leaving it and everything after it queued, in order — and,
/// given an `account`, at the first change taken once the library is no longer its.
pub async fn drain_checked<F, Fut>(
    library: &LibraryLock,
    service: &str,
    pass: &Mutex<()>,
    account: Option<&str>,
    send: F,
) -> Report
where
    F: Fn(Pending) -> Fut,
    Fut: std::future::Future<Output = Result<(), Error>>,
{
    let mut report = Report {
        outcome: Outcome::Clear,
        sent: false,
        refused: Vec::new(),
    };
    loop {
        // No sync pass between sending and taking it off the queue: one fetched just before
        // the write and stored just after would put the old state back with nothing waiting
        // to lay over it.
        let _pass = pass.lock().await;
        let next = {
            let mut l = library.lock().await;
            let theirs =
                account.is_none_or(|a| l.state(ACCOUNT_KEY).ok().flatten().as_deref() == Some(a));
            if theirs {
                l.outbox_take(service)
            } else {
                // Another account's library now: what is left waits for its own session.
                report.outcome = Outcome::Later;
                return report;
            }
        };
        let p = match next {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(_) => {
                report.outcome = Outcome::Later;
                return report;
            }
        };
        let (id, tries, record) = (p.id, p.tries, p.record.clone());
        // A panic while sending one change is that change refused, said with the rest — never
        // the whole queue stopped behind it, retried for ever, its edit laid back over the
        // screen each time (an empty note's formatting once did this).
        let sent = match std::panic::AssertUnwindSafe(send(p)).catch_unwind().await {
            Ok(sent) => sent,
            Err(_) => {
                crate::quiet!("outbox: sending a {service} change panicked; held as refused");
                Err(Error::Refused(
                    "Skykeep hit a problem sending this change, so nothing was changed in iCloud. Your text is safe here."
                        .into(),
                ))
            }
        };
        match sent {
            Ok(()) => {
                let _ = library.lock().await.outbox_done(id);
                report.sent = true;
            }
            Err(e) if transient(&e, tries) => {
                let _ = library.lock().await.outbox_retry(id, &e.to_string());
                report.outcome = Outcome::Later;
                return report;
            }
            Err(e) => {
                let _ = library.lock().await.outbox_refuse(id, &e.to_string());
                report.refused.push((record, e.to_string()));
            }
        }
    }
    report
}

/// A queued change's record missing from the library: gone — deleted here, or on another
/// device, and a pass has said so — once the library has had a pass of `service`. Before its
/// first (a library made again after damage, its records not yet brought back) the record is
/// not known yet, not gone: the change waits for that pass, rather than being dropped as done
/// or refused as deleted elsewhere.
pub async fn known_gone(library: &LibraryLock, service: &str) -> Result<(), Error> {
    if library.lock().await.last_synced(service)?.is_some() {
        Ok(())
    } else {
        Err(Error::Busy)
    }
}

pub fn unreadable(e: &serde_json::Error) -> Error {
    Error::Refused(format!("A queued change could not be read: {e}"))
}

pub fn json<T: Serialize>(v: &T) -> quince_core::Result<String> {
    serde_json::to_string(v)
        .map_err(|e| quince_core::Error::Io(std::io::Error::other(e.to_string())))
}

/// The newest change waiting for `record` in `service`, if it is still the last word on that
/// record (nothing of another kind queued after it) and of `kind`.
pub fn last_word(
    library: &Library,
    service: &str,
    record: &str,
    kind: &str,
) -> quince_core::Result<Option<Pending>> {
    Ok(library
        .outbox_pending(service)?
        .into_iter()
        .rfind(|p| p.record == record)
        .filter(|p| p.kind == kind))
}

/// An account as the library names it.
pub fn account_tag(dsid: &str) -> String {
    use sha2::Digest;
    format!(
        "{:x}",
        sha2::Sha256::digest(format!("quince-account:{dsid}"))
    )
}

/// The account a request's parameters ask as (its `dsid`), as the library names accounts.
pub fn asked_as(params: &[(String, String)]) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k == "dsid")
        .map(|(_, dsid)| account_tag(dsid))
}

/// The words a queued change held, for a person deciding what to do with it: the note's text,
/// the reminder's title, the event's title, the person's or list's name.
pub fn content_of(p: &Pending) -> String {
    use crate::calendar::{service_shown, CALENDAR};
    use crate::contacts::{CARD_LISTS, CONTACTS, LIST_DELETE, LIST_SAVE};
    use crate::notes::NOTES;
    use crate::reminders::REMINDERS;
    let v: serde_json::Value = serde_json::from_str(&p.payload).unwrap_or_default();
    let s = |ptr: &str| v.pointer(ptr).and_then(|x| x.as_str()).map(str::to_string);
    match (service_shown(&p.service), p.kind.as_str()) {
        (NOTES, "delete") => s("/title"),
        (NOTES, "create" | "save") => s("/body"),
        (REMINDERS, "create") => s("/title"),
        (REMINDERS, "edit") => s("/title").or_else(|| s("/notes")),
        (REMINDERS, "delete") => s("/reminder/title"),
        (CALENDAR, _) => s("/event/title"),
        (CONTACTS, "address" | CARD_LISTS | LIST_SAVE | LIST_DELETE) => s("/name"),
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
