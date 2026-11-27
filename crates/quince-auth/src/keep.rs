//! Keeping the session alive before a sync pass, the same on the desktop and the phone.
//!
//! Apple's cookies lapse after an idle spell long before the session does, and the session
//! token lapses in days, long before this device's trust does. A pass that went ahead on a
//! lapsed session failed with "needs a fresh sign-in", and on the phone, where nothing else
//! ever woke the session, every background sync after that failed the same way, silently, for
//! good. So every pass asks [`Client::ensure_session`] first: the heartbeat (`/validate`),
//! and when Apple says the session lapsed, the resume that takes it up again from the tokens
//! or the kept password, never asking for a code.
//!
//! It never hammers Apple:
//!
//! * a session found alive is trusted for [`ALIVE_FOR`] without asking again, so the passes
//!   of every service starting together cost one request between them;
//! * a session Apple has ended (it wants the password, or a code) is not asked about again
//!   until a sign-in brings a new one: each pass after it is answered from here, at once;
//! * signing back in goes through the sign-in's own cooldown (`Error::CoolingDown`, kept on
//!   disk per account), which this never resets.

use crate::Client;
use std::time::{Duration, Instant};

/// How long a session found alive is taken to stay so without asking Apple again.
pub const ALIVE_FOR: Duration = Duration::from_secs(60);

/// Renewals after a refusal are at least this far apart: a refusal within it waits for the next
/// look, so a service that keeps refusing never has Apple asked to renew every pass.
pub const RENEW_GAP: Duration = Duration::from_secs(120);

/// Where the session stands before a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// Alive, or taken up again: passes go ahead.
    Alive,
    /// Apple couldn't be reached, was busy, or asked to wait: nothing is known to have changed
    /// about the session. Passes are tried again later.
    Away,
    /// Apple wants a code before it trusts this device again. None has been asked for: that is
    /// left to the person, at the code screen.
    Code,
    /// Apple ended the session, and only the password brings it back.
    SignIn,
    /// Nobody is signed in.
    SignedOut,
}

impl Kept {
    /// Apple has ended the session: only the person, signing in again, can bring it back.
    #[must_use]
    pub fn ended(self) -> bool {
        matches!(self, Kept::Code | Kept::SignIn)
    }
}

/// What this run remembers between passes (see the module's notes).
#[derive(Debug, Default)]
pub(crate) struct Keeping {
    /// When the session was last found alive.
    alive_at: Option<Instant>,
    /// The session Apple ended, by its token, and what it wanted: not asked about again.
    ended: Option<(Option<String>, Kept)>,
    /// A pass was refused since the last look: the next look renews the session (see
    /// [`Client::session_refused`]).
    doubt: bool,
    /// Renewals so far this run: a pass says which it started under, so a refusal from before
    /// the latest renewal (the old cookies, still in flight) is not taken for one after it.
    generation: u64,
    /// When renewing was last tried, whatever Apple answered: tries are at least
    /// [`RENEW_GAP`] apart, so a busy or rate-limiting Apple is not asked again every look.
    tried_at: Option<Instant>,
    /// The session token the doubt is about: a new session (signed in since) is not doubted
    /// for what the old one met.
    token: Option<String>,
}

impl Client {
    /// Make sure the session is alive before a pass uses it: the heartbeat, and the resume
    /// when Apple says it lapsed. See the module's notes for how often Apple is asked.
    pub async fn ensure_session(&mut self) -> Kept {
        if self.session.webservices.is_none() {
            return Kept::SignedOut;
        }
        let token = self.session.session_token.clone();
        if let Some((was, kept)) = &self.keeping.ended {
            // The code entered since (it no longer waits on one) is a session taken up again.
            let code_given = *kept == Kept::Code && !self.awaiting_code();
            if *was == token && !code_given {
                return *kept;
            }
            // Signed in since: a new session, asked about afresh.
            self.keeping.ended = None;
        }
        // A pass was refused: `/validate` can go on vouching for a session every service
        // refuses (26–27 Nov 2026: a window failed every sync for a day, saying nothing), so it is
        // renewed, not looked at: the tokens traded for fresh cookies (`accountLogin`), at most
        // once every two minutes. Apple's answer decides: taken, the session is alive; Apple out
        // of reach, the doubt stands for the next look; the token refused, Apple has ended the
        // session, and it is taken up as any lapse is (the kept password, quietly), or said to
        // need a sign-in. A service refusing never decides it.
        // Signed in since the doubt: a new session, renewed by nothing the old one met. Passes
        // still in flight under the old cookies count as before it.
        if self.keeping.token != token {
            // The first look of the run only notes the session; a change from one to another
            // is a sign-in since.
            if self.keeping.token.is_some() {
                self.keeping.doubt = false;
                self.keeping.generation += 1;
            }
            self.keeping.token = token.clone();
        }
        if self.keeping.doubt
            && !self
                .keeping
                .tried_at
                .is_some_and(|at| at.elapsed() < RENEW_GAP)
        {
            self.keeping.tried_at = Some(Instant::now());
            let renewed = self.renew().await;
            let kept = self.looked(renewed);
            if kept != Kept::Away {
                self.keeping.doubt = false;
            }
            if kept == Kept::Alive {
                self.keeping.generation += 1;
            }
            self.keeping.token = self.session.session_token.clone();
            return self.settle(kept);
        }
        if self
            .keeping
            .alive_at
            .is_some_and(|at| at.elapsed() < ALIVE_FOR)
        {
            return Kept::Alive;
        }
        let looked = self.heartbeat().await;
        let kept = self.looked(looked);
        self.settle(kept)
    }

    /// What Apple's answer to a look at the session says about it.
    fn looked(&self, answer: crate::Result<bool>) -> Kept {
        match answer {
            Ok(true) => Kept::Alive,
            Ok(false) if self.awaiting_code() => Kept::Code,
            Ok(false) => Kept::SignIn,
            // Apple saying no to the account itself (see `Error::ends_session`).
            Err(e) if e.ends_session() => Kept::SignIn,
            // Offline, Apple busy or asking to wait, the sign-in's own cooldown, an answer not
            // understood, a keyring that would not answer: none says whether Apple still holds
            // the session, and none is a reason to ask again sooner than the next pass.
            Err(_) => Kept::Away,
        }
    }

    /// Remembers what a look found (see the module's notes), and answers it.
    fn settle(&mut self, kept: Kept) -> Kept {
        match kept {
            Kept::Alive => self.keeping.alive_at = Some(Instant::now()),
            Kept::Code | Kept::SignIn => {
                self.keeping.alive_at = None;
                // By the token it ends with: the resume drops it, so a sign-in brings another.
                self.keeping.ended = Some((self.session.session_token.clone(), kept));
            }
            Kept::Away | Kept::SignedOut => self.keeping.alive_at = None,
        }
        kept
    }

    /// Which renewal passes start under now: given back with [`Self::session_refused`].
    #[must_use]
    pub fn session_generation(&self) -> u64 {
        self.keeping.generation
    }

    /// A pass started under `generation` was answered "sign in again": the next
    /// [`Self::ensure_session`] renews the session rather than trusting `/validate`. A pass from
    /// before the latest renewal is not counted: it was refused for the old cookies.
    pub fn session_refused(&mut self, generation: u64) {
        if generation < self.keeping.generation {
            return;
        }
        self.keeping.alive_at = None;
        self.keeping.doubt = true;
    }

    /// A request or pass was just refused: [`Self::session_refused`] as now.
    pub fn session_doubted(&mut self) {
        self.session_refused(self.keeping.generation);
    }
}
