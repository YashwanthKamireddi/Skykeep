//! Apple's pushes, turned into refreshes of exactly what changed.
//!
//! A change made on a phone reaches this computer two ways:
//!
//! - **Live:** the web push socket icloud.com itself uses (`quince_icloud::courier`). Apple
//!   says within about a second that Notes, Reminders or Photos changed, and only that
//!   service is refreshed. A dropped socket is reconnected with backoff; a refusal (an
//!   endpoint Apple retired, a bad request) is tried again only after half an hour and
//!   more, never in a loop.
//! - **Polling, while there is no live channel:** each container's `changes/database` token —
//!   one small read-only request — is asked every few seconds while a window has focus, less
//!   often while one is merely open, and rarely from the tray; at once when the window gains
//!   focus or the network comes back ([`wake`]). Only a container whose token moved is
//!   refreshed.
//!
//! The log hears one line when the channel goes down and one when it is back. The timed
//! pass stays as the floor beneath both, for anything that went missing.

use crate::{alerts, refresh, Data, SYNCED};
use quince_auth::Client;
use quince_icloud::courier::{self, Courier, Failure, Identity};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tokio::sync::{mpsc, watch, Mutex};

/// The containers one connection listens to, and the service each one is. Photos measured
/// live (2026-10-19): a change made elsewhere arrives 0.7 to 1.5 seconds after it is written.
const CONTAINERS: [(&str, &str); 3] = [
    ("com.apple.notes", "notes"),
    ("com.apple.reminders", "reminders"),
    ("com.apple.photos.cloud", "photos"),
];

/// A registration with the push registry lasts 12 hours (`43200`); it is renewed a little
/// before, on the same socket.
const RENEW_REGISTRATION: Duration = Duration::from_secs(11 * 60 * 60);

/// A token is asked for a day (`86400`); the socket is opened afresh, with a new token, a
/// little before.
const TOKEN_LIFE: Duration = Duration::from_secs(23 * 60 * 60);

/// A phone sends a burst of pushes for one edit, one per record it saves. They are taken
/// together, so one edit is one refresh.
const SETTLE: Duration = Duration::from_millis(250);

/// The service a container's changes belong to.
fn service_of(container: &str) -> Option<&'static str> {
    CONTAINERS
        .iter()
        .find(|(c, _)| *c == container)
        .map(|(_, s)| *s)
}

/// Whether the live channel is up right now. While it is, nothing polls.
static LIVE: AtomicBool = AtomicBool::new(false);

/// Set when the channel comes back: one poll then catches up what changed while it was down.
static CATCH_UP: AtomicBool = AtomicBool::new(false);

/// Bumped by [`wake`]; the channel and the poller each watch it.
pub(crate) static WAKE: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

/// Something happened that makes now a good moment to look: a window gained focus, or the
/// network came back. A channel that failed for a passing reason is tried again at once, and
/// a poll runs at once.
pub fn wake() {
    WAKE.send_modify(|n| *n = n.wrapping_add(1));
}

/// Sleep for `wait`, or until [`wake`] — whichever is first. True when woken.
pub(crate) async fn sleep_or_wake(wait: Duration, woken: &mut watch::Receiver<u64>) -> bool {
    tokio::select! {
        () = tokio::time::sleep(wait) => false,
        r = woken.changed() => r.is_ok(),
    }
}

/// Where a pass of one service is asked for: the refresher's queue, once it runs.
static ASK: std::sync::OnceLock<mpsc::UnboundedSender<&'static str>> = std::sync::OnceLock::new();

/// The services with a later look already set, so asking again sets no second one.
static LATER: LazyLock<std::sync::Mutex<Vec<&'static str>>> =
    LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// A pass of `service` in `after`: for what Apple said is not ready yet (a Photos library it
/// is still preparing), looked at again rather than waited on for a push that may not come.
/// One look at a time is set for a service.
pub(crate) fn again_in(service: &'static str, after: Duration) {
    {
        let mut later = LATER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if later.contains(&service) {
            return;
        }
        later.push(service);
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        LATER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|s| *s != service);
        return;
    };
    rt.spawn(async move {
        tokio::time::sleep(after).await;
        LATER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|s| *s != service);
        if let Some(ask) = ASK.get() {
            ask.send(service).ok();
        }
    });
}

/// The services with a later look set now.
#[cfg(test)]
pub(crate) fn looks_set() -> Vec<&'static str> {
    LATER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub fn spawn(app: tauri::AppHandle, client: Arc<Mutex<Client>>) {
    crate::power::watch();
    let (tx, rx) = mpsc::unbounded_channel::<&'static str>();
    let _ = ASK.set(tx.clone());
    tauri::async_runtime::spawn(refresher(app.clone(), Arc::clone(&client), rx));
    tauri::async_runtime::spawn(watch_calendar(app.clone(), Arc::clone(&client), tx.clone()));
    tauri::async_runtime::spawn(watch_contacts(app.clone(), Arc::clone(&client), tx.clone()));
    tauri::async_runtime::spawn(crate::drive_watch::watch(app.clone(), Arc::clone(&client)));
    tauri::async_runtime::spawn(poll_cloudkit(app, Arc::clone(&client), tx.clone()));
    tauri::async_runtime::spawn(listen(client, tx));
}

/// Contacts, likewise without web push: `/co/changeset` from the last tokens answers 420
/// within seconds of a change made anywhere. Tokens are read once, and again after each
/// change, so the check stays against the book as it now is.
async fn watch_contacts(
    app: tauri::AppHandle,
    client: Arc<Mutex<Client>>,
    tx: mpsc::UnboundedSender<&'static str>,
) {
    let mut tokens: Option<quince_icloud::contacts::Tokens> = None;
    let mut backoff = Duration::from_secs(5);
    let mut woken = WAKE.subscribe();
    loop {
        sleep_or_wake(look_every(presence_for(&app, CONTACTS_WINDOWS)), &mut woken).await;
        let session = {
            let guard = client.lock().await;
            let s = guard.session();
            s.webservices
                .is_some()
                .then(|| s.service_url("contacts"))
                .flatten()
                .map(|root| (guard.http().clone(), guard.service_params(), root))
        };
        let Some((http, params, root)) = session else {
            tokens = None;
            continue;
        };
        let result = match tokens.as_mut() {
            None => quince_icloud::contacts::fetch_with_tokens(&http, &root, params)
                .await
                .map(|(_, t)| {
                    tokens = Some(t);
                    false
                }),
            Some(t) => quince_icloud::contacts::changed(&http, &root, params, t).await,
        };
        match result {
            Ok(changed) => {
                backoff = Duration::from_secs(5);
                if changed {
                    tx.send("contacts").ok();
                    // Read afresh next time: a 420 means these tokens are behind for good.
                    tokens = None;
                }
            }
            Err(e) => {
                quiet!("contacts watch: {e}");
                tokens = None;
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5 * 60));
            }
        }
    }
}

/// How present the app is: a window with focus, one merely on screen, or none (the tray).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presence {
    Focused,
    Open,
    Hidden,
}

/// Every window counts — the whole-library one and each app's own. A minimized window is
/// still "visible" to Windows (and to Tauri there), so it is counted as hidden.
pub(crate) fn presence(app: &tauri::AppHandle) -> Presence {
    presence_for(app, EVERY_WINDOW)
}

/// Any window at all: for what every window shows.
const EVERY_WINDOW: &[&str] = &[];

/// The windows that draw Calendar's events: its own, and the whole-library one (its launcher
/// counts today's events).
pub(crate) const CALENDAR_WINDOWS: &[&str] = &["app-calendar", "main"];
/// The windows that draw contacts: their own, Calendar (birthdays are made from the cards) and
/// the whole-library one.
pub(crate) const CONTACTS_WINDOWS: &[&str] = &["app-contacts", "app-calendar", "main"];
/// The windows that draw Drive: its own, and the whole-library one.
pub(crate) const DRIVE_WINDOWS: &[&str] = &["app-drivews", "main"];

/// [`presence`], for a watch whose changes only some windows draw: focused only while one of
/// those has the focus. With Notes in front, Calendar, Contacts and Drive were each asked at
/// the focused pace — some forty requests a minute for changes no window in front showed.
/// Switching to one of them asks at once all the same ([`wake`] on focus), so the slower pace
/// in between is never seen.
pub(crate) fn presence_for(app: &tauri::AppHandle, shows: &[&str]) -> Presence {
    reckon(
        app.webview_windows()
            .values()
            .filter(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
            .map(|w| (w.label().to_string(), w.is_focused().unwrap_or(false))),
        shows,
    )
}

/// The presence the windows on screen make, each by its label and whether it has the focus.
fn reckon(on_screen: impl IntoIterator<Item = (String, bool)>, shows: &[&str]) -> Presence {
    let mut open = false;
    for (label, focused) in on_screen {
        if focused && (shows.is_empty() || shows.contains(&label.as_str())) {
            return Presence::Focused;
        }
        open = true;
    }
    if open {
        Presence::Open
    } else {
        Presence::Hidden
    }
}

/// How often Calendar and Contacts — which have no push — are asked whether anything changed:
/// every few seconds while a window has the focus, so a change made on the phone shows as it
/// would with a push; less often while one is merely open (behind another, on another
/// workspace), and rarely from the tray, where only an alert depends on it. Focus coming back
/// asks at once ([`wake`]), so the slower pace is never felt. Asked every four seconds whatever
/// the window was doing, the two of them made thirty requests a minute all day.
/// On the battery, what no one is looking at is asked half as often (power.rs).
fn look_every(presence: Presence) -> Duration {
    let every = match presence {
        Presence::Focused => Duration::from_secs(4),
        Presence::Open => Duration::from_secs(15),
        Presence::Hidden => Duration::from_secs(60),
    };
    crate::power::pace(every, presence == Presence::Focused)
}

/// Calendar has no push for the web. Its own "has anything changed" call, `/ca/idle`, answers
/// in a few hundred milliseconds with only the calendars whose tag moved, so it is asked every
/// few seconds and a full refresh runs only when one did — measured within the poll of a
/// change made on another device.
async fn watch_calendar(
    app: tauri::AppHandle,
    client: Arc<Mutex<Client>>,
    tx: mpsc::UnboundedSender<&'static str>,
) {
    let zone = jiff::tz::TimeZone::system()
        .iana_name()
        .unwrap_or("UTC")
        .to_string();
    // Each calendar's tag as last seen; empty until the first answer seeds it.
    let mut known: Vec<(String, String)> = Vec::new();
    let mut seeded = false;
    let mut backoff = Duration::from_secs(5);
    let mut woken = WAKE.subscribe();
    loop {
        sleep_or_wake(look_every(presence_for(&app, CALENDAR_WINDOWS)), &mut woken).await;
        let session = {
            let guard = client.lock().await;
            let s = guard.session();
            s.webservices
                .is_some()
                .then(|| s.service_url("calendar"))
                .flatten()
                .map(|root| (guard.http().clone(), guard.service_params(), root))
        };
        let Some((http, params, root)) = session else {
            seeded = false;
            known.clear();
            continue;
        };
        let calendar = quince_icloud::calendar::CalendarClient::new(&http, &root, params);
        match calendar.changed(&known, &zone).await {
            Ok(changed) => {
                backoff = Duration::from_secs(5);
                for (guid, ctag) in &changed {
                    known.retain(|(g, _)| g != guid);
                    if let Some(ctag) = ctag {
                        known.push((guid.clone(), ctag.clone()));
                    }
                }
                // The first answer is every calendar there is, not a change.
                if seeded && !changed.is_empty() {
                    tx.send("calendar").ok();
                }
                seeded = true;
            }
            Err(e) => {
                quiet!("calendar watch: {e}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5 * 60));
            }
        }
    }
}

/// Refresh what the pushes named, a burst at a time.
async fn refresher(
    app: tauri::AppHandle,
    client: Arc<Mutex<Client>>,
    mut rx: mpsc::UnboundedReceiver<&'static str>,
) {
    while let Some(first) = rx.recv().await {
        let mut services = vec![first];
        tokio::time::sleep(SETTLE).await;
        while let Ok(more) = rx.try_recv() {
            if !services.contains(&more) {
                services.push(more);
            }
        }
        // Each service refreshed on its own, side by side: one taken in turn kept a note
        // edited on the phone waiting behind a long Photos pass (measured: a deletion took 54 s
        // to show). Two refreshes of one service still take turns (its pass lock).
        for service in services {
            let (app, client) = (app.clone(), Arc::clone(&client));
            tauri::async_runtime::spawn(async move {
                let data = app.state::<Data>();
                refresh::some(&client, &data.0, &[service]).await;
                alerts::changed();
                if service == "contacts" {
                    crate::warm_contact_photos(app.clone());
                }
                // Every service's freshness, not only this one: the window shows the whole.
                // What it stored, if anything, was told as it landed.
                let mut status = refresh::status(&data.0).await;
                status.changed = Some(Vec::new());
                app.emit(SYNCED, status).ok();
            });
        }
    }
}

// --- The live channel ------------------------------------------------------------------------

/// What kind of failure was last met, which decides how soon to try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The network, a dropped socket: soon, and sooner still on a wake.
    Transient,
    /// Apple asked to be left alone for this long.
    Limited(Duration),
    /// Refused outright — a retired endpoint, a bad request. Only after a long while.
    Refused,
    /// The session needs signing in again, which happens elsewhere.
    Reauth,
}

impl From<&Failure> for Kind {
    fn from(f: &Failure) -> Self {
        match f {
            Failure::Transient(_) => Self::Transient,
            Failure::RateLimited(d) => Self::Limited(*d),
            Failure::Refused(_) => Self::Refused,
            Failure::Reauth => Self::Reauth,
        }
    }
}

/// A channel that stayed up this long was healthy: its loss starts the backoff over.
const STABLE: Duration = Duration::from_secs(60);

/// When to try the channel again, and when to say anything about it.
///
/// Pure bookkeeping, so it is tested on its own: the backoff for each kind of failure, that a
/// channel which connects and drops at once still backs off, that a refusal is never retried
/// in a tight loop (not even on a wake), and that the log hears one line per change of state.
#[derive(Debug, Default)]
struct Pace {
    /// Failures in a row, since the channel was last healthy.
    failures: u32,
    last: Option<Kind>,
    /// Down, and said so. Cleared — with one line — when it is back.
    down_since: Option<Instant>,
    /// Up at least once in this run.
    ever_up: bool,
}

/// Grows `base` by doubling per failure after the first, up to `cap`.
fn doubling(base: Duration, failures: u32, cap: Duration) -> Duration {
    let shift = failures.saturating_sub(1).min(16);
    base.saturating_mul(1u32 << shift).min(cap)
}

impl Pace {
    /// How long to wait after this failure.
    fn delay(&self) -> Duration {
        let n = self.failures.max(1);
        match self.last.unwrap_or(Kind::Transient) {
            Kind::Transient => doubling(Duration::from_secs(1), n, Duration::from_secs(5 * 60)),
            Kind::Limited(asked) => asked.max(doubling(
                Duration::from_secs(1),
                n,
                Duration::from_secs(5 * 60),
            )),
            Kind::Refused => doubling(
                Duration::from_secs(30 * 60),
                n,
                Duration::from_secs(6 * 60 * 60),
            ),
            Kind::Reauth => doubling(Duration::from_secs(2 * 60), n, Duration::from_secs(30 * 60)),
        }
    }

    /// The channel could not be opened, or was lost after `lasted` (`None`: never opened).
    /// Returns the wait before the next try, and whether this is the moment to say the channel
    /// is down — the first failure only.
    fn failed(&mut self, kind: Kind, lasted: Option<Duration>, now: Instant) -> (Duration, bool) {
        if lasted.is_some_and(|l| l >= STABLE) {
            self.failures = 0;
        }
        self.failures = self.failures.saturating_add(1);
        self.last = Some(kind);
        let announce = self.down_since.is_none();
        if announce {
            self.down_since = Some(now);
        }
        (self.delay(), announce)
    }

    /// The channel is up. `Some(how long it was down)` when that was announced, for the one
    /// line saying it is back. The failure count stays until the channel proves stable.
    fn up(&mut self, now: Instant) -> Option<Duration> {
        self.ever_up = true;
        self.down_since.take().map(|t| now.duration_since(t))
    }

    /// Whether a wake (focus, network back) may cut the wait short. Only a passing failure:
    /// a refusal or a rate limit is waited out whatever happens.
    fn wake_may_retry(&self) -> bool {
        matches!(self.last, None | Some(Kind::Transient))
    }
}

/// Keep the live channel open for as long as the app runs.
async fn listen(client: Arc<Mutex<Client>>, tx: mpsc::UnboundedSender<&'static str>) {
    let containers: Vec<&str> = CONTAINERS.iter().map(|(c, _)| *c).collect();
    let mut identity = Identity::new();
    let mut pace = Pace::default();
    let mut woken = WAKE.subscribe();
    loop {
        let session = {
            let guard = client.lock().await;
            let s = guard.session();
            s.webservices
                .is_some()
                .then(|| s.dsid.clone())
                .flatten()
                .map(|dsid| (guard.http().clone(), dsid))
        };
        let Some((http, dsid)) = session else {
            // Not signed in yet: look again shortly.
            sleep_or_wake(Duration::from_secs(30), &mut woken).await;
            continue;
        };
        let opened = Instant::now();
        let failure = match open(&http, &mut identity, &containers, &dsid).await {
            Err(f) => (f, None),
            Ok(mut channel) => {
                let was_up = pace.ever_up;
                match pace.up(Instant::now()) {
                    Some(down) => quiet!("push: live channel back after {}s", down.as_secs()),
                    None if !was_up => quiet!("push: live channel up"),
                    None => {}
                }
                LIVE.store(true, Ordering::SeqCst);
                // What changed while no channel was up was never pushed: one poll catches it.
                CATCH_UP.store(true, Ordering::SeqCst);
                wake();
                let lost = run(&mut channel, &http, &identity, &dsid, &tx, opened).await;
                LIVE.store(false, Ordering::SeqCst);
                wake();
                // Those wakes were for the poller; this loop is not to take them as its own.
                woken.borrow_and_update();
                channel.close().await;
                let Some(lost) = lost else {
                    // The token's day is nearly up: straight back with a new one, unannounced.
                    identity.forget_token();
                    continue;
                };
                (lost, Some(opened.elapsed()))
            }
        };
        let (failure, lasted) = failure;
        if let Failure::Refused(_) = failure {
            // A fresh identity next time: whatever was refused is not offered again.
            identity = Identity::new();
        }
        let (wait, announce) = pace.failed(Kind::from(&failure), lasted, Instant::now());
        if announce {
            quiet!(
                "push: live channel down ({failure}); polling for changes, next try in {}s",
                wait.as_secs()
            );
        }
        let until = Instant::now() + wait;
        while Instant::now() < until {
            let woke = sleep_or_wake(until - Instant::now(), &mut woken).await;
            if woke && pace.wake_may_retry() {
                break;
            }
        }
    }
}

/// The push registry for this account: iCloud.com's, or Apple's regional one for an account
/// Apple keeps there (issue #10). Where its tokens are registered follows where it is kept.
fn registry() -> &'static str {
    if quince_auth::web_origin() == "https://www.icloud.com" {
        courier::REGISTRY
    } else {
        courier::REGIONAL_REGISTRY
    }
}

/// Connect, get a token and register it.
async fn open(
    http: &reqwest::Client,
    identity: &mut Identity,
    containers: &[&str],
    dsid: &str,
) -> Result<Courier, Failure> {
    let mut channel = Courier::connect(
        courier::SERVER,
        quince_auth::web_origin(),
        identity,
        containers,
    )
    .await?;
    if let Err(f) = channel.register(http, registry(), identity, dsid).await {
        channel.close().await;
        return Err(f);
    }
    Ok(channel)
}

/// Pass pushes on until the channel is lost (`Some`) or its token is due to be renewed
/// (`None`); renew the registration on the way.
async fn run(
    channel: &mut Courier,
    http: &reqwest::Client,
    identity: &Identity,
    dsid: &str,
    tx: &mpsc::UnboundedSender<&'static str>,
    opened: Instant,
) -> Option<Failure> {
    let mut registered = Instant::now();
    loop {
        if opened.elapsed() >= TOKEN_LIFE {
            return None;
        }
        if registered.elapsed() >= RENEW_REGISTRATION {
            if let Err(f) = channel.register(http, registry(), identity, dsid).await {
                return Some(f);
            }
            registered = Instant::now();
        }
        let until_renewal = RENEW_REGISTRATION.saturating_sub(registered.elapsed());
        match channel
            .next(until_renewal.min(Duration::from_secs(10 * 60)))
            .await
        {
            Ok(Some(courier::Event::Changed(container))) => {
                if let Some(service) = service_of(&container) {
                    quiet!("push: {service} changed (live)");
                    tx.send(service).ok();
                }
            }
            Ok(Some(courier::Event::Other) | None) => {}
            Err(f) => return Some(f),
        }
    }
}

// --- Polling, while there is no live channel ---------------------------------------------------

/// How often the containers are asked whether anything changed while no live channel is up:
/// often with a window in focus, less with one merely open, rarely from the tray.
/// On the battery, what no one is looking at is asked half as often (power.rs).
pub(crate) fn poll_every(presence: Presence) -> Duration {
    let every = match presence {
        Presence::Focused => Duration::from_secs(5),
        Presence::Open => Duration::from_secs(15),
        Presence::Hidden => Duration::from_secs(60),
    };
    crate::power::pace(every, presence == Presence::Focused)
}

/// How often a window on screen checks anyway while the live channel is up.
const SAFETY_CHECK: Duration = Duration::from_secs(2 * 60);

/// While the live channel is down, ask each container's database whether it moved, and
/// refresh only the ones that did. The tokens are seeded once at the start, so a channel lost
/// hours later is still compared with a known point.
async fn poll_cloudkit(
    app: tauri::AppHandle,
    client: Arc<Mutex<Client>>,
    tx: mpsc::UnboundedSender<&'static str>,
) {
    let mut tokens: Vec<Option<String>> = vec![None; CONTAINERS.len()];
    let mut seeded = false;
    let mut woken = WAKE.subscribe();
    let mut backoff: Option<Duration> = None;
    loop {
        let live = LIVE.load(Ordering::SeqCst);
        let catch_up = CATCH_UP.swap(false, Ordering::SeqCst);
        if seeded && live && !catch_up {
            // The channel carries it; woken when that changes. A window on screen still gets
            // a check every couple of minutes, in case a push went astray.
            let woke = sleep_or_wake(SAFETY_CHECK, &mut woken).await;
            if woke || !LIVE.load(Ordering::SeqCst) || presence(&app) == Presence::Hidden {
                continue;
            }
        }
        let session = {
            let guard = client.lock().await;
            let s = guard.session();
            s.webservices
                .is_some()
                .then(|| s.service_url("ckdatabasews"))
                .flatten()
                .map(|root| (guard.http().clone(), guard.service_params(), root))
        };
        let Some((http, params, root)) = session else {
            sleep_or_wake(Duration::from_secs(30), &mut woken).await;
            continue;
        };
        let mut failed = None;
        for (i, (container, service)) in CONTAINERS.iter().enumerate() {
            match quince_icloud::push::database_changed(
                &http,
                &root,
                &params,
                container,
                tokens[i].as_deref(),
            )
            .await
            {
                Ok((moved, next)) => {
                    tokens[i] = next;
                    if moved {
                        if live && !catch_up {
                            quiet!("push: {service} changed (poll; the live channel missed it)");
                        } else {
                            quiet!("push: {service} changed (poll)");
                        }
                        tx.send(service).ok();
                    }
                }
                Err(quince_icloud::Error::StaleCursor) => {
                    // Start over, and refresh in case something moved meanwhile.
                    tokens[i] = None;
                    tx.send(service).ok();
                }
                Err(e) => failed = Some(e),
            }
        }
        seeded = true;
        backoff = match failed {
            None => None,
            Some(e) => {
                let next = backoff.map_or(Duration::from_secs(5), |b| {
                    (b * 2).min(Duration::from_secs(5 * 60))
                });
                if backoff.is_none() {
                    quiet!(
                        "push: change check failed ({e}); trying again in {}s",
                        next.as_secs()
                    );
                }
                Some(next)
            }
        };
        let wait = backoff.unwrap_or_else(|| poll_every(presence(&app)));
        sleep_or_wake(wait, &mut woken).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_container_is_its_service() {
        assert_eq!(service_of("com.apple.notes"), Some("notes"));
        assert_eq!(service_of("com.apple.reminders"), Some("reminders"));
        assert_eq!(service_of("com.apple.photos.cloud"), Some("photos"));
        assert_eq!(service_of("com.apple.clouddocs"), None);
    }

    #[test]
    fn a_passing_failure_backs_off_doubling_to_five_minutes() {
        let mut pace = Pace::default();
        let t = Instant::now();
        let waits: Vec<u64> = (0..12)
            .map(|_| pace.failed(Kind::Transient, None, t).0.as_secs())
            .collect();
        assert_eq!(&waits[..5], &[1, 2, 4, 8, 16]);
        assert_eq!(*waits.last().unwrap(), 300, "capped at five minutes");
        assert!(pace.wake_may_retry(), "network back retries at once");
    }

    #[test]
    fn a_refusal_is_never_retried_in_a_tight_loop() {
        // What Apple now answers the old endpoint: 403, "This endpoint is deprecated."
        let mut pace = Pace::default();
        let t = Instant::now();
        let (first, _) = pace.failed(Kind::Refused, None, t);
        assert!(first >= Duration::from_secs(30 * 60), "{first:?}");
        let (second, _) = pace.failed(Kind::Refused, None, t);
        assert!(second > first);
        for _ in 0..20 {
            pace.failed(Kind::Refused, None, t);
        }
        assert_eq!(pace.delay(), Duration::from_secs(6 * 60 * 60));
        assert!(
            !pace.wake_may_retry(),
            "not even focus or the network cuts it short"
        );
        // A rate limit is honoured whatever the count.
        let mut pace = Pace::default();
        let (wait, _) = pace.failed(Kind::Limited(Duration::from_secs(900)), None, t);
        assert_eq!(wait, Duration::from_secs(900));
        assert!(!pace.wake_may_retry());
    }

    #[test]
    fn a_channel_that_drops_at_once_still_backs_off() {
        let mut pace = Pace::default();
        let t = Instant::now();
        let mut last = Duration::ZERO;
        for _ in 0..6 {
            pace.up(t);
            let (wait, _) = pace.failed(Kind::Transient, Some(Duration::from_secs(2)), t);
            assert!(wait >= last, "never shorter while it keeps dropping");
            last = wait;
        }
        assert!(last >= Duration::from_secs(32));
        // One that stayed up a while starts over.
        pace.up(t);
        let (wait, _) = pace.failed(Kind::Transient, Some(STABLE), t);
        assert_eq!(wait, Duration::from_secs(1));
    }

    #[test]
    fn the_log_hears_one_line_down_and_one_line_back() {
        let mut pace = Pace::default();
        let t0 = Instant::now();
        let said: Vec<bool> = (0..5)
            .map(|_| pace.failed(Kind::Transient, None, t0).1)
            .collect();
        assert_eq!(said, [true, false, false, false, false]);
        let back = pace.up(t0 + Duration::from_secs(40));
        assert_eq!(back, Some(Duration::from_secs(40)));
        assert_eq!(pace.up(t0 + Duration::from_secs(50)), None, "said once");
        let (_, said) = pace.failed(Kind::Transient, Some(Duration::from_secs(600)), t0);
        assert!(said, "a new outage is a new line");
    }

    #[test]
    fn polling_is_quickest_with_focus_and_slowest_from_the_tray() {
        let (f, o, h) = (
            poll_every(Presence::Focused),
            poll_every(Presence::Open),
            poll_every(Presence::Hidden),
        );
        assert!(f <= Duration::from_secs(5), "changes show within seconds");
        assert!(f < o && o < h);
        assert!(h <= Duration::from_secs(60));
    }

    fn on_screen(windows: &[(&str, bool)]) -> Vec<(String, bool)> {
        windows
            .iter()
            .map(|(l, f)| ((*l).to_string(), *f))
            .collect()
    }

    #[test]
    fn a_watch_is_focused_only_while_a_window_that_draws_it_is() {
        // Notes in front, Calendar behind it: Calendar's changes are on screen, not in front.
        let notes = on_screen(&[("app-notes", true), ("app-calendar", false)]);
        assert_eq!(reckon(notes.clone(), CALENDAR_WINDOWS), Presence::Open);
        assert_eq!(reckon(notes.clone(), CONTACTS_WINDOWS), Presence::Open);
        assert_eq!(reckon(notes.clone(), DRIVE_WINDOWS), Presence::Open);
        assert_eq!(reckon(notes, EVERY_WINDOW), Presence::Focused);
        // Calendar in front draws its events and the birthdays made from the cards.
        let calendar = on_screen(&[("app-calendar", true)]);
        assert_eq!(
            reckon(calendar.clone(), CALENDAR_WINDOWS),
            Presence::Focused
        );
        assert_eq!(
            reckon(calendar.clone(), CONTACTS_WINDOWS),
            Presence::Focused
        );
        assert_eq!(reckon(calendar, DRIVE_WINDOWS), Presence::Open);
        // The whole-library window's launcher counts every app.
        let main = on_screen(&[("main", true)]);
        for shows in [CALENDAR_WINDOWS, CONTACTS_WINDOWS, DRIVE_WINDOWS] {
            assert_eq!(reckon(main.clone(), shows), Presence::Focused);
        }
        assert_eq!(reckon(Vec::new(), CALENDAR_WINDOWS), Presence::Hidden);
    }

    #[test]
    fn with_notes_in_front_the_watches_for_other_apps_ask_a_few_times_a_minute() {
        // Requests a minute to Apple from the Calendar, Contacts and Drive watches, on mains
        // power, with only Notes in front. It was 42 when any focused window set their pace.
        let per_minute = |d: Duration| 60 / d.as_secs();
        let notes = on_screen(&[("app-notes", true)]);
        let asked = per_minute(look_every(reckon(notes.clone(), CALENDAR_WINDOWS)))
            + per_minute(look_every(reckon(notes.clone(), CONTACTS_WINDOWS)))
            + per_minute(poll_every(reckon(notes, DRIVE_WINDOWS)));
        assert!(asked <= 12, "{asked} a minute");
    }

    #[test]
    fn calendar_and_contacts_are_asked_often_only_while_looked_at() {
        let (f, o, h) = (
            look_every(Presence::Focused),
            look_every(Presence::Open),
            look_every(Presence::Hidden),
        );
        assert!(f <= Duration::from_secs(5), "changes show within seconds");
        // A window merely open — behind another, on another workspace — is not asked at the
        // focused pace: at most four requests a minute for the two of them, not thirty.
        assert!(o >= Duration::from_secs(15), "{o:?}");
        assert!(o < h && h <= Duration::from_secs(120));
    }
}
