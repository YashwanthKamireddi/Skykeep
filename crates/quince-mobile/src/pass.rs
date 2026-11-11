//! What every sync pass on the phone goes through, as the desktop's do: the session kept alive
//! first ([`Account::ensure_session`]), then the shared pass (`quince_sync::pass`) with whose
//! library it is, the daily whole walk, Advanced Data Protection's pause, a panic kept to the
//! pass, and the failure recorded. And where Advanced Data Protection stands, with the approval
//! asked for on the person's own devices, as the desktop's `data_access.rs` asks for it.

use crate::{failed, runtime, Account, EngineError};
use quince_auth::pcs::{self, DataAccess};
use quince_icloud::photos::Links;
use quince_sync::library_lock::LibraryLock;
use quince_sync::pass::{self, Outcome, Report};
use std::collections::HashMap;
use std::sync::Arc;

/// Where the session stands before a pass.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Alive, or taken up again by itself: passes go ahead.
    Alive,
    /// Apple couldn't be reached or asked to wait: tried again later. Not a sign-out.
    Away,
    /// Apple wants a code before it trusts this phone again: the person, in the app.
    Code,
    /// Apple ended the session: only signing in again brings it back.
    SignIn,
    /// Nobody is signed in.
    SignedOut,
}

/// Where Advanced Data Protection's approval stands, as Home shows it.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataState {
    /// Nothing waits on an approval.
    Open,
    /// The approval is on the person's devices, or given and the keys on their way.
    Waiting,
    /// An approval is needed and none is on its way: the person asks for it.
    Needed,
    /// Apple couldn't put the approval on any of their devices.
    NotSent,
    /// "Access iCloud Data on the Web" is off: nothing opens until it is on.
    WebAccessOff,
}

/// Where the approval stands, and what waits on it.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct DataAccessNow {
    pub state: DataState,
    /// The services held until it is given, by the app's names (`notes`, `reminders`,
    /// `photos`, `drive`); empty when nothing waits.
    pub services: Vec<String>,
    /// Why the last step failed, in words, when it did.
    pub problem: Option<String>,
}

fn data_state(d: DataAccess) -> DataState {
    match d {
        DataAccess::Open => DataState::Open,
        DataAccess::Waiting => DataState::Waiting,
        DataAccess::Needed => DataState::Needed,
        DataAccess::NotSent => DataState::NotSent,
        DataAccess::WebAccessOff => DataState::WebAccessOff,
    }
}

/// What one pass needs from the session, read in one look at it: the account it stores for is
/// the one its requests ask as (an account switched in between can never mix the two).
pub(crate) struct Apple {
    pub http: reqwest::Client,
    pub params: Vec<(String, String)>,
    pub cloudkit: Option<String>,
    pub account: String,
    pub waiting: Vec<&'static str>,
}

/// What the phone does with what a pass found beside its rows.
struct Phone<'a>(&'a Account);

impl pass::Hooks for Phone<'_> {
    fn links(&self, links: HashMap<String, Links>) {
        crate::photos::remember(links);
    }
    // A key Apple let go of early: the service waits for the next approval from here on,
    // and is never retried before it.
    fn refused(&self, service: &'static str) {
        self.0.client().data_access_lapsed(service);
    }
}

impl Account {
    /// [`Apple`], in one look at the session; `what` names the app for "sign in" wording.
    pub(crate) fn apple(&self, what: &str) -> Result<Apple, EngineError> {
        let c = self.client();
        let params = c.service_params();
        let account = quince_sync::queue::asked_as(&params)
            .ok_or_else(|| failed(format!("Sign in to see your {what}.")))?;
        Ok(Apple {
            http: c.http().clone(),
            cloudkit: c.session().service_url("ckdatabasews"),
            waiting: c.data_access_missing(),
            params,
            account,
        })
    }

    /// One pass of `service` through the shared driver, under its pass lock.
    pub(crate) async fn pass(
        &self,
        service: &'static str,
        apple: &Apple,
        library: &LibraryLock,
        lock: &tokio::sync::Mutex<()>,
    ) -> Report {
        let hooks = Phone(self);
        let ctx = pass::Ctx {
            http: apple.http.clone(),
            params: apple.params.clone(),
            cloudkit: apple.cloudkit.clone(),
            account: Some(apple.account.clone()),
            waiting: apple.waiting.clone(),
            hooks: &hooks,
        };
        let report = pass::run(service, &ctx, library, lock).await;
        // Apple answered "sign in again" to the pass itself: the next pass asks about the
        // session rather than trusting the last look.
        if report.outcome == Outcome::Failed
            && report.detail.as_deref() == Some(&quince_icloud::Error::ReauthRequired.to_string())
        {
            self.client().session_doubted();
        }
        report
    }

    /// The account's library for `apple`'s account.
    pub(crate) fn library_of(&self, apple: &Apple) -> Result<Arc<LibraryLock>, EngineError> {
        self.library_for(&apple.account)
    }
}

/// A pass's report as the app takes it: stored or waiting on an approval is a sync that went
/// (true when it waits), a failure is an error with Apple's reason, and a service not offered
/// says so in `offered`.
pub(crate) fn settled(report: &Report, offered: &str) -> Result<bool, EngineError> {
    match report.outcome {
        Outcome::Ok | Outcome::Preparing => Ok(false),
        Outcome::Paused => Ok(true),
        Outcome::Unavailable => Err(failed(offered)),
        Outcome::Failed => {
            Err(failed(report.detail.clone().unwrap_or_else(|| {
                format!("{} couldn't be read.", report.label)
            })))
        }
    }
}

#[uniffi::export]
impl Account {
    /// Make sure the session is alive before a pass: the heartbeat, and the resume when Apple
    /// says it lapsed (no code is ever asked for from here). Called before every background
    /// pass; Apple is asked at most once a minute however many passes start together, and not
    /// at all once it has ended the session, until a sign-in.
    pub fn ensure_session(&self) -> SessionState {
        let mut c = self.client();
        match runtime().block_on(c.ensure_session()) {
            quince_auth::Kept::Alive => SessionState::Alive,
            quince_auth::Kept::Away => SessionState::Away,
            quince_auth::Kept::Code => SessionState::Code,
            quince_auth::Kept::SignIn => SessionState::SignIn,
            quince_auth::Kept::SignedOut => SessionState::SignedOut,
        }
    }

    /// Where Advanced Data Protection's approval stands, from what the phone holds alone: no
    /// request, and nothing shown on anyone's devices. What Home and every pass go by.
    pub fn data_access_standing(&self) -> DataAccessNow {
        let c = self.client();
        DataAccessNow {
            state: data_state(c.data_access_standing()),
            services: c
                .data_access_missing()
                .into_iter()
                .map(str::to_string)
                .collect(),
            problem: None,
        }
    }

    /// The person asked: Apple is asked to let this session read the protected services, the
    /// approval put on their devices when one is needed — every service in one go, so one
    /// approval covers them all. Answers where it got to; `Waiting` is followed with
    /// [`Account::data_access_follow`].
    pub fn data_access_ask(&self) -> DataAccessNow {
        let mut c = self.client();
        let step = runtime().block_on(c.data_access(true));
        let services = c
            .data_access_missing()
            .into_iter()
            .map(str::to_string)
            .collect();
        match step {
            Ok(state) => DataAccessNow {
                state: data_state(state),
                services,
                problem: None,
            },
            Err(e) => DataAccessNow {
                state: data_state(c.data_access_standing()),
                services,
                problem: Some(e.to_string()),
            },
        }
    }

    /// An approval on its way, followed until it lands or the wait runs out (five minutes, on
    /// `pcs::APPROVAL_SCHEDULE`, never hammering): each step only asks whether it was given,
    /// and fetches the keys once it is. The session is held only for each step.
    pub fn data_access_follow(&self) -> DataAccessNow {
        // `pcs::follow`'s loop, with the session taken (a blocking lock here) only for each step.
        let mut n = 0;
        let landed = loop {
            let Some(wait) = (pcs::APPROVAL_SCHEDULE)(n) else {
                // Run out: the person is asked to try again rather than waited on for ever.
                break Ok(DataAccess::Needed);
            };
            n += 1;
            std::thread::sleep(wait);
            let step = {
                let mut c = self.client();
                if c.session().webservices.is_none() {
                    // Signed out while it was on its way: nothing left to follow.
                    break Ok(DataAccess::Open);
                }
                runtime().block_on(c.data_access(false))
            };
            match step {
                Ok(DataAccess::Waiting) => {}
                // A dropped connection or a busy Apple is waited out like any other step; the
                // schedule still bounds how long.
                Err(quince_auth::Error::Network(_) | quince_auth::Error::Unavailable(_)) => {}
                other => break other,
            }
        };
        let c = self.client();
        let services = c
            .data_access_missing()
            .into_iter()
            .map(str::to_string)
            .collect();
        match landed {
            Ok(state) => DataAccessNow {
                state: data_state(state),
                services,
                problem: None,
            },
            Err(e) => DataAccessNow {
                state: DataState::Needed,
                services,
                problem: Some(e.to_string()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of Apple's answers has the app's word for it, so Home can say each plainly.
    #[test]
    fn every_standing_has_its_word() {
        for (d, s) in [
            (DataAccess::Open, DataState::Open),
            (DataAccess::Waiting, DataState::Waiting),
            (DataAccess::Needed, DataState::Needed),
            (DataAccess::NotSent, DataState::NotSent),
            (DataAccess::WebAccessOff, DataState::WebAccessOff),
        ] {
            assert_eq!(data_state(d), s);
        }
    }

    /// A waiting service is a sync that went, said as waiting; a failure carries Apple's reason;
    /// a service not offered says so.
    #[test]
    fn a_report_settles_as_the_app_takes_it() {
        let mut r = pass::unavailable("notes");
        assert!(settled(&r, "Apple hasn't offered Notes for this account.").is_err());
        r.outcome = Outcome::Paused;
        assert!(settled(&r, "").unwrap(), "waiting");
        r.outcome = Outcome::Ok;
        assert!(!settled(&r, "").unwrap());
        r.outcome = Outcome::Failed;
        r.detail = Some("Apple’s servers are busy".into());
        let e = settled(&r, "").unwrap_err();
        assert!(e.to_string().contains("busy"));
    }
}
