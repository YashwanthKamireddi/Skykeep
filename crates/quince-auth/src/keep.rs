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

/// Tries at renewing after a refusal are at least this far apart, whatever Apple answered: the
/// first comes at once, and a service that keeps refusing for its own reasons (or a busy Apple)
/// costs at most one `accountLogin` in this long. A request the person is waiting on may try
/// sooner ([`Client::session_refused_now`]).
pub const RENEW_GAP: Duration = Duration::from_secs(600);

/// The gap for a request the person is waiting on: short, but never every request.
pub const RENEW_GAP_NOW: Duration = Duration::from_secs(30);

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
    /// A pass or request was refused since the last look: the next look renews the session
    /// (see [`Client::session_refused`]).
    doubt: bool,
    /// The refusal was a request the person is waiting on: tried without waiting out the gap.
    urgent: bool,
    /// Times the cookies have been traded afresh this run (any `accountLogin` Apple took): a
    /// pass or request refused under an earlier count is sent again once it has moved.
    generation: u64,
    /// When renewing was last tried, whatever Apple answered (see [`RENEW_GAP`]).
    tried_at: Option<Instant>,
    /// The session token last seen: a new session (signed in since) is not doubted for what
    /// the old one met.
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
        self.note_session();
        let gap = if self.keeping.urgent {
            RENEW_GAP_NOW
        } else {
            RENEW_GAP
        };
        let due = !self.keeping.tried_at.is_some_and(|at| at.elapsed() < gap);
        if self.keeping.doubt && due {
            self.keeping.tried_at = Some(Instant::now());
            self.keeping.urgent = false;
            let renewed = self.renew().await;
            let kept = self.looked(renewed);
            // Apple out of reach: the doubt stands, for the next look.
            if kept != Kept::Away {
                self.keeping.doubt = false;
            }
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

    /// A session other than the one last seen (signed in since, by the person or by a resume
    /// with the kept password) is noted at once: passes still in flight under the old cookies
    /// count as before it, and nothing the old one met is held against it. The run's first look
    /// only notes it.
    fn note_session(&mut self) {
        let token = self.session.session_token.clone();
        if self.keeping.token != token {
            if self.keeping.token.is_some() {
                self.keeping.doubt = false;
                self.keeping.generation += 1;
            }
            self.keeping.token = token;
        }
    }

    /// Remembers what a look found (see the module's notes), and answers it.
    fn settle(&mut self, kept: Kept) -> Kept {
        self.note_session();
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

    /// How many times the cookies have been traded afresh this run: a pass or request notes it
    /// before going out, and is sent again after a refusal only once it has moved.
    #[must_use]
    pub fn session_generation(&self) -> u64 {
        self.keeping.generation
    }

    /// A pass or request was answered "sign in again": the next [`Self::ensure_session`] renews
    /// the session rather than trusting `/validate`, once the gap since the last try is out.
    pub fn session_refused(&mut self) {
        self.keeping.alive_at = None;
        self.keeping.doubt = true;
    }

    /// [`Self::session_refused`] for a request the person is waiting on (opening a folder, a
    /// file): renewed at the next look without waiting out the gap.
    pub fn session_refused_now(&mut self) {
        self.session_refused();
        self.keeping.urgent = true;
    }

    /// Apple took the tokens for fresh cookies (`accountLogin`), by a renewal or by any resume:
    /// counted, and nothing refused before it is held against the session.
    pub(crate) fn cookies_renewed(&mut self) {
        self.keeping.generation += 1;
        self.keeping.doubt = false;
        self.keeping.urgent = false;
    }
}
