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

use crate::{Client, Error};
use std::time::{Duration, Instant};

/// How long a session found alive is taken to stay so without asking Apple again.
pub const ALIVE_FOR: Duration = Duration::from_secs(60);

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
        if self
            .keeping
            .alive_at
            .is_some_and(|at| at.elapsed() < ALIVE_FOR)
        {
            return Kept::Alive;
        }
        let kept = match self.heartbeat().await {
            Ok(true) => Kept::Alive,
            Ok(false) if self.awaiting_code() => Kept::Code,
            Ok(false) => Kept::SignIn,
            // Offline, Apple busy or asking to wait, or the sign-in's own cooldown: not a
            // sign-out, and not a reason to ask again sooner than the next pass.
            Err(
                Error::Network(_)
                | Error::Unavailable(_)
                | Error::RateLimited
                | Error::CoolingDown { .. },
            ) => Kept::Away,
            // Anything else (an answer not understood, a keyring that would not answer) says
            // nothing about whether Apple still holds the session either.
            Err(_) => Kept::Away,
        };
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

    /// A pass found the session refused after all (Apple answered "sign in again" to a service
    /// call): the next [`Self::ensure_session`] asks Apple rather than trusting the last look.
    pub fn session_doubted(&mut self) {
        self.keeping.alive_at = None;
    }
}
