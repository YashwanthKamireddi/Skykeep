//! Advanced Data Protection approvals, followed for the window (see `quince_auth::pcs`).
//!
//! An account with Advanced Data Protection and "Access iCloud Data on the Web" on lets this
//! session read Notes, Reminders, Photos and iCloud Drive only once the person approves it on
//! one of their devices, for about an hour at a time, and every key Apple then releases shows a
//! notice on their devices. So this is never background work:
//!
//! * Apple is asked only when the person acts — signing in, or pressing Sync Now (the approval
//!   card's button, or an explicit refresh). Every protected service is asked for in that one
//!   go, back to back, so one approval covers them all.
//! * A background pass (the timer, a push, a window coming back) asks Apple nothing about it:
//!   it reads where things stand from what is held here ([`Client::data_access_standing`]), and
//!   the protected services wait — not fail — until the next approval. Calendar and Contacts,
//!   which Apple does not protect this way, carry on as usual.
//! * Keys are held only until they run out, by the time recorded when they arrived, so passes
//!   stop asking for a protected service before Apple would refuse it.
//! * While an approval is on its way the client is taken only for each step, so nothing waits
//!   behind it; once it is given, what was waiting is read and the changes queued meanwhile go.

use crate::Auth;
use quince_auth::pcs::{self, DataAccess};
use quince_auth::Client;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

/// Told to every window when where the approval stands changes.
pub const EVENT: &str = "quince://data-access";

/// Where the approval stands, as the window shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Access {
    pub state: DataAccess,
    /// The services waiting on it, by the app's names; empty when `state` is `open`.
    pub services: Vec<&'static str>,
    /// Why the last step failed, in words, when it did.
    pub problem: Option<String>,
}

impl Access {
    const OPEN: Access = Access {
        state: DataAccess::Open,
        services: Vec::new(),
        problem: None,
    };
}

static NOW: std::sync::Mutex<Access> = std::sync::Mutex::new(Access::OPEN);
/// An approval is being followed: a second follower would only double the requests.
static FOLLOWING: AtomicBool = AtomicBool::new(false);
/// Services a pass found refused for want of their key, for the next step to count as missing.
static REFUSED: std::sync::Mutex<BTreeSet<&'static str>> = std::sync::Mutex::new(BTreeSet::new());

/// Who is asking, which decides whether Apple is asked at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The person: signing in, or pressing Sync Now. Apple is asked, and the approval put on
    /// their devices when it is needed.
    Person,
    /// Anything else — the timer, a push, a window opening or coming back. Nothing is asked of
    /// Apple; where things stand is read from what is held here.
    Background,
}

/// Whether a step for `ask` may send anything to Apple.
fn asks_apple(ask: Ask) -> bool {
    ask == Ask::Person
}

/// Where it stands now.
pub fn current() -> Access {
    NOW.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The protected services a pass should leave alone now: their keys are not held, so asking
/// Apple for them would only be refused. Read from what is held here; asks Apple nothing.
pub fn waiting(client: &Client) -> Vec<&'static str> {
    client.data_access_missing()
}

/// A new sign-in, or a sign-out: nothing from the session before carries over.
pub fn forget(app: &tauri::AppHandle) {
    REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    set(app, Access::OPEN);
}

/// A pass found `service` refused by Advanced Data Protection: a key Apple let go early.
pub fn refused(service: &'static str) {
    REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(service);
}

fn set(app: &tauri::AppHandle, next: Access) {
    let changed = {
        let mut now = NOW
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = *now != next;
        *now = next.clone();
        changed
    };
    if changed {
        app.emit(EVENT, next).ok();
    }
}

/// Take one step and tell the window. For the person, Apple is asked — every missing service
/// in one go — and an approval on its way is followed; for anything else, only what is held
/// here is read.
pub async fn check(app: &tauri::AppHandle, client: &Arc<Mutex<Client>>, ask: Ask) {
    let (step, missing) = {
        let mut guard = client.lock().await;
        if guard.session().webservices.is_none() {
            return;
        }
        for service in std::mem::take(
            &mut *REFUSED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ) {
            guard.data_access_lapsed(service);
        }
        let step = if asks_apple(ask) {
            guard.data_access(true).await
        } else {
            Ok(guard.data_access_standing())
        };
        (step, guard.data_access_missing())
    };
    // An approval being followed is the follower's to settle; a background look at what is
    // held here (nothing yet) must not say "needed" over "waiting".
    if ask == Ask::Background && FOLLOWING.load(Ordering::SeqCst) {
        return;
    }
    settle(app, client, step, missing, ask);
}

/// After a pass: when Advanced Data Protection kept a service out (a key let go early), say
/// where things stand — from what is held here, asking Apple nothing.
pub async fn after_pass(app: &tauri::AppHandle, client: &Arc<Mutex<Client>>) {
    let any = !REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_empty();
    if any {
        check(app, client, Ask::Background).await;
    }
}

/// What one step came to, told; an approval on its way followed until it lands.
fn settle(
    app: &tauri::AppHandle,
    client: &Arc<Mutex<Client>>,
    step: Result<DataAccess, quince_auth::Error>,
    waiting_on: Vec<&'static str>,
    ask: Ask,
) {
    let was = current();
    match step {
        Ok(DataAccess::Open) => opened(app, client, &was),
        Ok(DataAccess::Waiting) => {
            set(
                app,
                Access {
                    state: DataAccess::Waiting,
                    services: waiting_on.clone(),
                    problem: None,
                },
            );
            follow(app.clone(), Arc::clone(client), waiting_on);
        }
        Ok(state) => set(
            app,
            Access {
                state,
                services: waiting_on,
                problem: None,
            },
        ),
        // Not reached, or Apple busy: nothing is known to have changed. Said to the person who
        // asked; nothing else asks.
        Err(e @ (quince_auth::Error::Network(_) | quince_auth::Error::Unavailable(_))) => {
            if ask == Ask::Person {
                set(
                    app,
                    Access {
                        problem: Some(e.to_string()),
                        ..was
                    },
                );
            }
        }
        Err(e) => {
            quiet!("data access: {e:?}");
            set(
                app,
                Access {
                    state: DataAccess::Needed,
                    services: waiting_on,
                    problem: Some(e.to_string()),
                },
            );
        }
    }
}

/// Every key is here (or none was ever needed): said, and what was waiting on it read now —
/// and the changes made here meanwhile sent.
fn opened(app: &tauri::AppHandle, client: &Arc<Mutex<Client>>, was: &Access) {
    set(app, Access::OPEN);
    if was.state == DataAccess::Open || was.services.is_empty() {
        return;
    }
    let services = was.services.clone();
    let app = app.clone();
    let client = Arc::clone(client);
    tauri::async_runtime::spawn(async move {
        crate::outbox::kick();
        let data = app.state::<crate::Data>();
        crate::refreshed_some(&app, &client, &data.0, &services).await;
        crate::alerts::changed();
        let mut status = crate::refresh::status(&data.0).await;
        status.changed = Some(Vec::new());
        app.emit(crate::SYNCED, status).ok();
    });
}

/// Follow an approval the person asked for, one step at a time on [`pcs::APPROVAL_SCHEDULE`],
/// taking the client only for each step. Each step asks only whether it was given, and fetches
/// the keys once it is: no second prompt.
fn follow(app: tauri::AppHandle, client: Arc<Mutex<Client>>, waiting_on: Vec<&'static str>) {
    if FOLLOWING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let stepping = Arc::clone(&client);
        let landed = pcs::follow(pcs::APPROVAL_SCHEDULE, || {
            let client = Arc::clone(&stepping);
            async move {
                let mut guard = client.lock().await;
                if guard.session().webservices.is_none() {
                    // Signed out while it was on its way: nothing left to follow.
                    return Ok(DataAccess::Open);
                }
                match guard.data_access(false).await {
                    // A dropped connection or a busy Apple is waited out like any other step;
                    // the schedule still bounds how long.
                    Err(quince_auth::Error::Network(_) | quince_auth::Error::Unavailable(_)) => {
                        Ok(DataAccess::Waiting)
                    }
                    other => other,
                }
            }
        })
        .await;
        FOLLOWING.store(false, Ordering::SeqCst);
        let still = client.lock().await.data_access_missing();
        let waiting_on = if still.is_empty() { waiting_on } else { still };
        settle(&app, &client, landed, waiting_on, Ask::Person);
    });
}

/// Where the approval stands, for a window opening.
#[tauri::command]
pub async fn data_access_status() -> Result<Access, String> {
    Ok(current())
}

/// The person pressed Sync Now: ask Apple — the approval on their devices when it is needed,
/// every protected service in one go — and follow it.
#[tauri::command]
pub async fn data_access_ask(
    app: tauri::AppHandle,
    auth: tauri::State<'_, Auth>,
) -> Result<Access, String> {
    check(&app, &auth.client, Ask::Person).await;
    Ok(current())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_person_ever_has_apple_asked() {
        assert!(asks_apple(Ask::Person));
        assert!(
            !asks_apple(Ask::Background),
            "no background pass sends Apple anything about keys"
        );
    }

    #[test]
    fn the_window_reads_it_in_its_own_words() {
        let told = serde_json::to_value(Access {
            state: DataAccess::Waiting,
            services: vec!["notes", "drive"],
            problem: None,
        })
        .unwrap();
        assert_eq!(
            told,
            serde_json::json!({ "state": "waiting", "services": ["notes", "drive"], "problem": null })
        );
        assert_eq!(
            serde_json::to_value(DataAccess::WebAccessOff).unwrap(),
            "webAccessOff"
        );
        assert_eq!(
            serde_json::to_value(DataAccess::NotSent).unwrap(),
            "notSent"
        );
    }
}
