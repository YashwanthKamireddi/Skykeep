//! Calendar accounts other than iCloud: Nextcloud, and any other CalDAV server.
//!
//! Signing in, keeping the library in step with the server, and sending what was changed here.
//! The calendars and events themselves are ordinary rows of the library (quince_core::dav), so
//! every screen, search and the outbox treat them as they do iCloud's.

use crate::library_lock::LibraryLock;
use crate::outbox::{self, EventDelete, EventSave};
use crate::{refresh, state_dir, Data, UiResult, SYNCED};
use quince_auth::Store;
use quince_core::calendar::{Calendar, Event};
use quince_core::dav::{is_dav, DavAccount, DavCalendar, DavResource};
use quince_core::outbox::Pending;
use quince_core::Library;
use quince_icloud::caldav::{self, Remote, RemoteCalendar, Resource};
use quince_icloud::calendar::series_of;
use quince_icloud::calendar_feeds as feeds;
use quince_icloud::ical_edit::{self, Change, Removal, Written};
use quince_icloud::{nextcloud, Error};
use reqwest::{Client, Url};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex, Notify};

/// One pass or one send at a time: a pass that fetched just before a write and stored just
/// after it would put the old version back.
pub static PASS: Mutex<()> = Mutex::const_new(());

static WAKE: Notify = Notify::const_new();

/// When the accounts were last looked at, in seconds since 1970.
static LOOKED: AtomicI64 = AtomicI64::new(0);

/// How often the accounts are looked at while a window that draws Calendar is in front.
const FOCUSED_EVERY: i64 = 30;

/// Passwords already read this run. Read from the keyring on every poll, a locked keyring
/// would ask the person over and over.
static SECRETS: StdMutex<Option<HashMap<String, String>>> = StdMutex::new(None);

/// A Nextcloud sign-in waiting on the person's browser.
static SIGNING: StdMutex<Option<Arc<Signing>>> = StdMutex::new(None);

struct Signing {
    flow: nextcloud::Flow,
    server: Url,
    http: Client,
    cancelled: Notify,
}

/// How long the server keeps a sign-in open.
const SIGN_IN_LASTS: Duration = Duration::from_secs(20 * 60);

const YEAR: i64 = 366 * 86_400;

/// The stretch of time repeating events are laid out over, widened as the person looks further.
const WINDOW: &str = "dav.window";

const SIGN_IN_AGAIN: &str = "The server no longer accepts this sign-in.";

const USER_AGENT: &str = "Skykeep";

/// Look at the accounts now, not on the next poll.
pub fn wake() {
    WAKE.notify_one();
}

/// The window `label` came to the front. One that draws Calendar has the accounts looked at
/// now, unless they were within the focused pace: with another app in front they are looked
/// at only every minute, and switching to Calendar must not show what is up to a minute old.
pub fn focused(label: &str) {
    if wakes_for(label, unix_now(), LOOKED.load(Ordering::Relaxed)) {
        wake();
    }
}

fn wakes_for(label: &str, now: i64, looked: i64) -> bool {
    crate::push::CALENDAR_WINDOWS.contains(&label) && now - looked >= FOCUSED_EVERY
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn client_for(hosts: &[String]) -> quince_icloud::Result<Client> {
    Ok(quince_auth::account_only(hosts)
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(90))
        .build()?)
}

fn host_of(url: &Url) -> String {
    url.host_str()
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .to_ascii_lowercase()
}

fn host_named(server: &str) -> String {
    Url::parse(server).map_or_else(|_| server.to_string(), |u| host_of(&u))
}

/// An error in words for this account: `Error`'s own speak of iCloud.
fn words(e: &Error, host: &str) -> String {
    match e {
        Error::Network(n) if n.is_timeout() => {
            format!("{host} took too long to answer. Check your connection and try again.")
        }
        Error::Network(_) => format!(
            "Couldn’t reach {host}. Check the address and your connection. A server with a self-signed certificate can’t be used."
        ),
        Error::Busy(_) => format!("{host} is busy right now. Try again in a minute."),
        Error::Protocol(_) => format!("{host} sent back something unexpected."),
        Error::ReauthRequired => SIGN_IN_AGAIN.to_string(),
        other => other.to_string(),
    }
}

fn store(app: &AppHandle) -> Result<Store, String> {
    Store::keyring(&state_dir(app)).map_err(|e| e.to_string())
}

fn remember(id: &str, secret: &str) {
    SECRETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(id.to_string(), secret.to_string());
}

fn forget_remembered(id: &str) {
    if let Some(map) = SECRETS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        map.remove(id);
    }
}

fn secret_of(app: &AppHandle, id: &str) -> Result<String, Error> {
    let known = SECRETS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(id).cloned());
    if let Some(secret) = known {
        return Ok(secret);
    }
    let kept = tokio::task::block_in_place(|| {
        store(app).and_then(|s| s.account_secret(id).map_err(|e| e.to_string()))
    });
    match kept {
        Ok(Some(secret)) => {
            remember(id, &secret);
            Ok(secret)
        }
        Ok(None) => Err(Error::ReauthRequired),
        // A keyring that would not answer says nothing about the password: try again later.
        Err(_) => Err(Error::Busy(None)),
    }
}

/// The connection to an account.
pub fn remote_of(app: &AppHandle, a: &DavAccount) -> Result<Remote, Error> {
    let secret = secret_of(app, &a.id)?;
    Ok(Remote::new(client_for(&a.hosts)?, &a.username, &secret))
}

fn window(l: &Library) -> (i64, i64) {
    let kept = l.state(WINDOW).ok().flatten().and_then(|text| {
        let (from, to) = text.split_once(',')?;
        Some((from.parse().ok()?, to.parse().ok()?))
    });
    kept.unwrap_or_else(|| {
        let now = unix_now();
        (now - 2 * YEAR, now + 3 * YEAR)
    })
}

/// What one event's text comes to on the calendar.
fn laid_out(r: &DavResource, from: i64, to: i64, here: &jiff::tz::TimeZone) -> Vec<Event> {
    feeds::parse(&r.ics, here)
        .map(|f| feeds::account_events(&f, &r.calendar, &r.guid, r.etag.as_deref(), from, to))
        .unwrap_or_default()
}

/// A calendar's events laid out again from the text held for it.
fn lay_calendar(l: &mut Library, calendar: &str) -> quince_core::Result<()> {
    let (from, to) = window(l);
    let here = jiff::tz::TimeZone::system();
    let events: Vec<Event> = l
        .dav_resources(calendar)?
        .iter()
        .flat_map(|r| laid_out(r, from, to, &here))
        .collect();
    l.replace_dav_calendar_events(calendar, &events)
}

/// An account's calendars laid out again from what the server last sent, with only what still
/// waits to be sent laid over them.
pub fn lay_account(l: &mut Library, account: &str) -> quince_core::Result<()> {
    for c in l.dav_calendars(account)? {
        lay_calendar(l, &c.calendar.guid)?;
    }
    outbox::replay(l, &outbox::account_queue(account))
}

/// What a pass found for one calendar.
struct Found {
    calendar: DavCalendar,
    fetched: Vec<Resource>,
    gone: Vec<String>,
    looked: bool,
}

/// `#rrggbb` for a calendar the server names no colour for: a steady one, by its address.
fn colour_for(guid: &str) -> String {
    const COLOURS: [&str; 7] = [
        "#ff9500", "#34aadc", "#65db39", "#cc73e1", "#ffcc00", "#a2845e", "#ff2d55",
    ];
    let sum: usize = guid.bytes().map(usize::from).sum();
    COLOURS[sum % COLOURS.len()].to_string()
}

fn as_stored(account: &str, c: &RemoteCalendar, rank: i64) -> DavCalendar {
    let guid = caldav::calendar_guid(account, &c.url);
    DavCalendar {
        calendar: Calendar {
            title: c.title.clone(),
            colour: Some(c.colour.clone().unwrap_or_else(|| colour_for(&guid))),
            ctag: c.ctag.clone(),
            sort_order: 500 + rank,
            enabled: true,
            writable: c.writable,
            is_default: false,
            guid,
        },
        url: c.url.clone(),
    }
}

/// Bring the library in step with the server. Answers whether anything on screen moved.
async fn pass(app: &AppHandle, a: &DavAccount) -> Result<bool, Error> {
    let remote = remote_of(app, a)?;
    pass_with(&app.state::<Data>().0, &remote, a).await
}

async fn pass_with(library: &LibraryLock, remote: &Remote, a: &DavAccount) -> Result<bool, Error> {
    let _one = PASS.lock().await;
    let mut listed = remote.calendars(&a.home).await?;
    listed.sort_by(|x, y| x.order.cmp(&y.order).then_with(|| x.title.cmp(&y.title)));
    let held: HashMap<String, DavCalendar> = library
        .read(|l| l.dav_calendars(&a.id))
        .await?
        .into_iter()
        .map(|c| (c.calendar.guid.clone(), c))
        .collect();
    // An account always has a calendar. An answer with none is a broken answer: stored, it
    // would delete every calendar here and every event with them.
    if listed.is_empty() && !held.is_empty() {
        return Err(Error::Protocol(
            "the server listed no calendars, so nothing was changed".into(),
        ));
    }
    let mut found = Vec::new();
    for (rank, c) in listed.iter().enumerate() {
        let calendar = as_stored(&a.id, c, i64::try_from(rank).unwrap_or(0));
        let guid = calendar.calendar.guid.clone();
        // A calendar whose tag has not moved has nothing new in it.
        let same = c.ctag.is_some() && held.get(&guid).is_some_and(|h| h.calendar.ctag == c.ctag);
        if same {
            found.push(Found {
                calendar,
                fetched: Vec::new(),
                gone: Vec::new(),
                looked: false,
            });
            continue;
        }
        let theirs = remote.etags(&c.url).await?;
        let guid_for_read = guid.clone();
        let ours = library.read(move |l| l.dav_etags(&guid_for_read)).await?;
        let (wanted, gone) = caldav::differences(&theirs, &ours);
        let fetched = if wanted.is_empty() {
            Vec::new()
        } else {
            remote.fetch(&c.url, &wanted).await?
        };
        found.push(Found {
            calendar,
            fetched,
            gone,
            looked: true,
        });
    }
    let queue = outbox::account_queue(&a.id);
    let mut library = library.lock_for_pass().await;
    tokio::task::block_in_place(|| {
        library.atomically(|l| {
            // Removed while the pass was out, or another iCloud account's library by now.
            if l.dav_account(&a.id)?.is_none() {
                return Ok::<bool, quince_core::Error>(false);
            }
            store_found(l, &a.id, found, &queue)
        })
    })
    .map_err(Error::from)
}

fn store_found(
    l: &mut Library,
    account: &str,
    found: Vec<Found>,
    queue: &str,
) -> quince_core::Result<bool> {
    let shown = |c: &DavCalendar| {
        (
            c.calendar.guid.clone(),
            c.calendar.title.clone(),
            c.calendar.colour.clone(),
            c.calendar.writable,
        )
    };
    let before: Vec<_> = l.dav_calendars(account)?.iter().map(shown).collect();
    let calendars: Vec<DavCalendar> = found.iter().map(|f| f.calendar.clone()).collect();
    l.put_dav_calendars(account, &calendars)?;
    let after: Vec<_> = l.dav_calendars(account)?.iter().map(shown).collect();
    let mut moved = before != after;
    for f in found {
        if !f.looked || (f.fetched.is_empty() && f.gone.is_empty()) {
            continue;
        }
        moved = true;
        let calendar = f.calendar.calendar.guid;
        for href in f.gone {
            l.forget_dav_resource(&caldav::event_guid(&calendar, &href))?;
        }
        for r in f.fetched {
            l.put_dav_resource(&DavResource {
                guid: caldav::event_guid(&calendar, &r.href),
                calendar: calendar.clone(),
                href: r.href,
                etag: r.etag,
                ics: r.ics,
            })?;
        }
        lay_calendar(l, &calendar)?;
    }
    if moved {
        outbox::replay(l, queue)?;
    }
    Ok(moved)
}

/// A pass, and what came of it told to the account's row and the window.
async fn pass_and_record(app: &AppHandle, a: &DavAccount) -> Result<(), String> {
    let data = app.state::<Data>();
    let host = host_named(&a.server);
    let (result, state) = match pass(app, a).await {
        Ok(moved) => (Ok(moved), (Some(unix_now()), None, false)),
        Err(Error::ReauthRequired) => (
            Err(SIGN_IN_AGAIN.to_string()),
            (None, Some(SIGN_IN_AGAIN.to_string()), true),
        ),
        Err(e) => {
            let said = words(&e, &host);
            (Err(said.clone()), (None, Some(said), false))
        }
    };
    let was = data
        .0
        .read(|l| l.dav_account(&a.id))
        .await
        .ok()
        .flatten()
        .map(|x| (x.problem, x.needs_sign_in));
    let _ = data
        .0
        .lock()
        .await
        .set_dav_account_state(&a.id, state.0, state.1.as_deref(), state.2);
    // The sidebar's "!" goes up or comes down with the problem, whether or not events moved.
    let problem_changed = was.is_some_and(|w| w != (state.1.clone(), state.2));
    if matches!(result, Ok(true)) || problem_changed {
        app.emit(
            SYNCED,
            refresh::status_changed(&data.0, &["calendar"]).await,
        )
        .ok();
    }
    result.map(|_| ())
}

/// Keep every account in step, for as long as the app runs.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            // Another person's iCloud library, or none: not this account's to touch.
            LOOKED.store(unix_now(), Ordering::Relaxed);
            if crate::same_account(&app).await {
                let accounts = app
                    .state::<Data>()
                    .0
                    .read(Library::dav_accounts)
                    .await
                    .unwrap_or_default();
                for a in accounts.iter().filter(|a| !a.needs_sign_in) {
                    let _ = pass_and_record(&app, a).await;
                }
            }
            let wait = match crate::push::presence_for(&app, crate::push::CALENDAR_WINDOWS) {
                crate::push::Presence::Focused => Duration::from_secs(FOCUSED_EVERY.unsigned_abs()),
                crate::push::Presence::Open => Duration::from_secs(60),
                crate::push::Presence::Hidden => Duration::from_secs(300),
            };
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = WAKE.notified() => {}
            }
        }
    });
}

/// The window asked to show `[from, to]` (both `YYYY-MM-DD`): repeating events are laid out
/// that far, if they weren't.
pub async fn widen_window(app: &AppHandle, from: &str, to: &str) {
    let seconds = |text: &str| {
        text.parse::<jiff::civil::Date>()
            .ok()
            .and_then(|d| d.to_zoned(jiff::tz::TimeZone::UTC).ok())
            .map(|z| z.timestamp().as_second())
    };
    let (Some(from), Some(to)) = (seconds(from), seconds(to)) else {
        return;
    };
    let data = app.state::<Data>();
    // Asked on every step of the calendar: looked at first without waiting for a pass.
    let settled = data.0.read(|l| {
        let (have_from, have_to) = window(l);
        Ok(l.dav_accounts()?.is_empty() || (from >= have_from && to + 86_400 <= have_to))
    });
    if settled.await.unwrap_or(true) {
        return;
    }
    let _one = PASS.lock().await;
    let mut library = data.0.lock_for_pass().await;
    let (have_from, have_to) = window(&library);
    let accounts = library.dav_accounts().unwrap_or_default();
    let wider = (have_from.min(from - YEAR), have_to.max(to + YEAR));
    let _ = tokio::task::block_in_place(|| {
        library.atomically(|l| {
            l.set_state(WINDOW, &format!("{},{}", wider.0, wider.1))?;
            for a in &accounts {
                for c in l.dav_calendars(&a.id)? {
                    lay_calendar(l, &c.calendar.guid)?;
                }
                outbox::replay(l, &outbox::account_queue(&a.id))?;
            }
            Ok::<(), quince_core::Error>(())
        })
    });
}

/// Whether a change for this account should wait for the person to sign in again, rather than
/// be tried: a server counts a refused password as someone guessing.
pub fn waiting_for_sign_in(a: &DavAccount) -> bool {
    a.needs_sign_in
}

/// Send one change to the account's server.
pub async fn send(
    app: &AppHandle,
    library: &LibraryLock,
    remote: &Remote,
    account: &DavAccount,
    p: Pending,
) -> Result<(), Error> {
    let sent = send_change(library, remote, account, &p).await;
    if matches!(sent, Err(Error::ReauthRequired)) {
        let _ = library.lock().await.set_dav_account_state(
            &account.id,
            None,
            Some(SIGN_IN_AGAIN),
            true,
        );
        app.emit(
            SYNCED,
            refresh::status_changed(library, &["calendar"]).await,
        )
        .ok();
    }
    sent
}

async fn send_change(
    library: &LibraryLock,
    remote: &Remote,
    account: &DavAccount,
    p: &Pending,
) -> Result<(), Error> {
    match p.kind.as_str() {
        "save" => send_save(library, remote, account, p).await,
        "delete" => send_delete(library, remote, account, p).await,
        other => Err(Error::Refused(format!(
            "Skykeep does not know how to send “{other}”."
        ))),
    }
}

fn unreadable(e: &serde_json::Error) -> Error {
    Error::Refused(format!("That change can’t be read: {e}"))
}

async fn send_save(
    library: &LibraryLock,
    remote: &Remote,
    account: &DavAccount,
    p: &Pending,
) -> Result<(), Error> {
    let s: EventSave = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
    let here = jiff::tz::TimeZone::system();
    let series = series_of(&s.event.guid).to_string();
    let calendar = s.event.calendar_guid.clone();
    let (resource, calendar_url) = {
        let l = library.lock().await;
        (l.dav_resource(&series)?, l.dav_calendar_url(&calendar)?)
    };
    let calendar_url = calendar_url
        .ok_or_else(|| Error::Refused("That calendar is no longer on the server.".into()))?;
    let change = Change {
        event: &s.event,
        base: s.base.as_ref(),
        alerts: s.alerts.as_deref(),
        repeat: s.repeat_changed.then_some(s.repeat.as_ref()),
        scope: s.scope,
    };
    let now = unix_now();
    let Some(resource) = resource else {
        if !s.create {
            return Err(Error::Refused(
                "This event was deleted on another device, so it wasn’t saved.".into(),
            ));
        }
        let uid = series.rsplit(':').next().unwrap_or(&series).to_string();
        let address = format!("{calendar_url}{uid}.ics");
        let ics = ical_edit::new_event(&uid, &change, now, &here)?;
        match remote.create(&address, &ics).await {
            // Already there: the answer to the last try was lost on the way.
            Ok(()) | Err(Error::Conflict) => {}
            Err(e) => return Err(e),
        }
        return settle(library, remote, &calendar, &address, p).await;
    };
    let address = caldav::join(&account.home, &resource.href)?;
    let (mut ics, mut etag) = (resource.ics, resource.etag);
    for attempt in 0..3 {
        let (kept, rest) = match ical_edit::change(&ics, &change, now, &here)? {
            Written::Whole(text) => (text, None),
            Written::Split { kept, rest, uid } => (kept, Some((rest, uid))),
        };
        match remote.update(&address, &kept, etag.as_deref()).await {
            Ok(()) => {
                if let Some((rest, uid)) = rest {
                    let new_address = format!("{calendar_url}{uid}.ics");
                    remote.create(&new_address, &rest).await?;
                    settle(library, remote, &calendar, &new_address, p).await?;
                }
                return settle(library, remote, &calendar, &address, p).await;
            }
            // Changed on another device since it was read: made again on that version, so
            // what changed there stays.
            Err(Error::Conflict) if attempt < 2 => {
                let fresh = remote.get(&address).await?;
                ics = fresh.ics;
                etag = fresh.etag;
            }
            Err(Error::NotFound) => {
                return Err(Error::Refused(
                    "This event was deleted on another device, so it wasn’t saved.".into(),
                ));
            }
            Err(e) => return Err(e),
        }
    }
    Err(Error::Conflict)
}

/// What the server now holds for an event, read back and laid out, with what still waits laid
/// back over it.
async fn settle(
    library: &LibraryLock,
    remote: &Remote,
    calendar: &str,
    address: &str,
    p: &Pending,
) -> Result<(), Error> {
    let fresh = remote.get(address).await?;
    let resource = DavResource {
        guid: caldav::event_guid(calendar, &fresh.href),
        calendar: calendar.to_string(),
        href: fresh.href,
        etag: fresh.etag,
        ics: fresh.ics,
    };
    let here = jiff::tz::TimeZone::system();
    let mut l = library.lock().await;
    l.atomically(|l| {
        l.put_dav_resource(&resource)?;
        let (from, to) = window(l);
        l.replace_dav_events(&resource.guid, &laid_out(&resource, from, to, &here))?;
        outbox::replay_except(l, &p.service, p.id)?;
        Ok::<(), Error>(())
    })
}

async fn forget_sent(library: &LibraryLock, series: &str, p: &Pending) -> Result<(), Error> {
    let mut l = library.lock().await;
    l.atomically(|l| {
        l.forget_dav_resource(series)?;
        outbox::replay_except(l, &p.service, p.id)?;
        Ok::<(), Error>(())
    })
}

async fn send_delete(
    library: &LibraryLock,
    remote: &Remote,
    account: &DavAccount,
    p: &Pending,
) -> Result<(), Error> {
    let d: EventDelete = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
    let here = jiff::tz::TimeZone::system();
    let series = series_of(&d.event.guid).to_string();
    let resource = library.lock().await.dav_resource(&series)?;
    let Some(resource) = resource else {
        // Never on the server, or deleted there already.
        return Ok(());
    };
    let address = caldav::join(&account.home, &resource.href)?;
    let calendar = resource.calendar.clone();
    let (mut ics, mut etag) = (resource.ics, resource.etag);
    for attempt in 0..3 {
        let result = match ical_edit::remove(&ics, &d.event, d.scope, unix_now(), &here)? {
            Removal::Whole => match remote.delete(&address, etag.as_deref()).await {
                Ok(()) => return forget_sent(library, &series, p).await,
                Err(e) => Err(e),
            },
            Removal::Kept(text) => match remote.update(&address, &text, etag.as_deref()).await {
                Ok(()) => return settle(library, remote, &calendar, &address, p).await,
                Err(e) => Err(e),
            },
        };
        match result {
            // Changed on another device since it was read: deleted all the same, as asked.
            Err(Error::Conflict) if attempt < 2 => match remote.get(&address).await {
                Ok(fresh) => {
                    ics = fresh.ics;
                    etag = fresh.etag;
                }
                Err(Error::NotFound) => return forget_sent(library, &series, p).await,
                Err(e) => return Err(e),
            },
            Err(Error::NotFound) => return forget_sent(library, &series, p).await,
            other => return other,
        }
    }
    Err(Error::Conflict)
}

/// A new event's guid in this calendar, its file's name chosen here.
#[must_use]
pub fn new_event_guid(calendar: &str) -> String {
    format!(
        "{calendar}:{}",
        uuid::Uuid::new_v4().to_string().to_uppercase()
    )
}

async fn text_of(data: &Data, event: &Event) -> Option<DavResource> {
    let series = series_of(&event.guid).to_string();
    data.0
        .read(move |l| l.dav_resource(&series))
        .await
        .ok()
        .flatten()
}

pub async fn alerts_of(data: &Data, event: &Event) -> Vec<i64> {
    let here = jiff::tz::TimeZone::system();
    text_of(data, event)
        .await
        .map(|r| ical_edit::alerts_of(&r.ics, &event.guid, &here))
        .unwrap_or_default()
}

pub async fn repeat_of(data: &Data, event: &Event) -> Option<quince_icloud::calendar::EventRepeat> {
    let here = jiff::tz::TimeZone::system();
    ical_edit::repeat_of(&text_of(data, event).await?.ics, &here)
}

pub async fn guests_of(data: &Data, event: &Event) -> Vec<quince_icloud::calendar::Guest> {
    let here = jiff::tz::TimeZone::system();
    let me = quince_core::dav::account_of(&event.calendar_guid)
        .map(ToString::to_string)
        .unwrap_or_default();
    let username = data
        .0
        .read(move |l| l.dav_account(&me))
        .await
        .ok()
        .flatten()
        .map(|a| a.username)
        .unwrap_or_default();
    text_of(data, event)
        .await
        .map(|r| ical_edit::guests_of(&r.ics, &event.guid, &username, &here))
        .unwrap_or_default()
}

/// The account a calendar belongs to, connected.
async fn account_of_calendar(
    app: &AppHandle,
    data: &Data,
    guid: &str,
) -> UiResult<(DavAccount, Remote, String)> {
    let id = quince_core::dav::account_of(guid)
        .ok_or("That calendar is no longer here.")?
        .to_string();
    let guid = guid.to_string();
    let (account, url) = data
        .0
        .read(move |l| Ok((l.dav_account(&id)?, l.dav_calendar_url(&guid)?)))
        .await
        .map_err(|e| e.to_string())?;
    let (Some(account), Some(url)) = (account, url) else {
        return Err("That calendar is no longer here.".into());
    };
    let remote = remote_of(app, &account).map_err(|e| words(&e, &host_named(&account.server)))?;
    Ok((account, remote, url))
}

/// A calendar renamed or recoloured on its server.
pub async fn update_calendar(
    app: &AppHandle,
    data: &Data,
    guid: &str,
    title: Option<&str>,
    colour: Option<&str>,
) -> UiResult<()> {
    let (account, remote, url) = account_of_calendar(app, data, guid).await?;
    remote
        .set_calendar(&url, title, colour)
        .await
        .map_err(|e| words(&e, &host_named(&account.server)))?;
    pass_and_record(app, &account).await
}

/// A calendar and its events deleted on their server, once the person has confirmed it.
pub async fn delete_calendar(app: &AppHandle, data: &Data, guid: &str) -> UiResult<()> {
    let (account, remote, url) = account_of_calendar(app, data, guid).await?;
    remote
        .delete_calendar(&url)
        .await
        .map_err(|e| words(&e, &host_named(&account.server)))?;
    pass_and_record(app, &account).await
}

/// On sign-out, keeping nothing: every account's password out of the keyring.
pub fn forget_secrets(app: &AppHandle, library: &Library) {
    let Ok(store) = store(app) else { return };
    for a in library.dav_accounts().unwrap_or_default() {
        let _ = store.forget_account_secret(&a.id);
        forget_remembered(&a.id);
    }
}

/// Which hosts the accounts reach, for Settings.
pub fn hosts_of(library: &Library) -> Vec<String> {
    let mut hosts: Vec<String> = library
        .dav_accounts()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|a| a.hosts)
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// Whether an event belongs to such an account.
#[must_use]
pub fn is_account_event(event: &Event) -> bool {
    is_dav(&event.calendar_guid)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// Keep an account whose password was just found to work, and read it for the first time.
async fn keep_account(
    app: &AppHandle,
    kind: &str,
    server: &Url,
    username: &str,
    password: &str,
    starts: &[Url],
) -> UiResult<DavAccount> {
    let host = host_of(server);
    let finder: caldav::ClientFor<'_> = &client_for;
    let found = caldav::discover(finder, starts, username, password)
        .await
        .map_err(|e| words(&e, &host))?;
    let data = app.state::<Data>();
    let accounts = data
        .0
        .read(Library::dav_accounts)
        .await
        .map_err(|e| e.to_string())?;
    let server_text = server.as_str().trim_end_matches('/').to_string();
    // The same account signed in to again takes its new password, and is not a second copy.
    let same = accounts
        .iter()
        .find(|a| a.server == server_text && a.username == username)
        .cloned();
    let id = same.as_ref().map_or_else(new_id, |a| a.id.clone());
    let wanted = if kind == "nextcloud" {
        "Nextcloud".to_string()
    } else {
        host.clone()
    };
    let taken = accounts.iter().any(|a| a.id != id && a.title == wanted);
    let title = if taken {
        format!("{wanted} ({username})")
    } else {
        wanted
    };
    tokio::task::block_in_place(|| {
        store(app).and_then(|s| {
            s.set_account_secret(&id, password).map_err(|_| {
                "Skykeep couldn’t keep the password in your system keyring. Check that it is running and unlocked."
                    .to_string()
            })
        })
    })?;
    remember(&id, password);
    let account = DavAccount {
        id,
        kind: kind.to_string(),
        title,
        server: server_text,
        username: username.to_string(),
        home: found.home,
        hosts: found.hosts,
        synced_at: same.as_ref().and_then(|a| a.synced_at),
        problem: None,
        needs_sign_in: false,
    };
    data.0
        .lock()
        .await
        .put_dav_account(&account)
        .map_err(|e| e.to_string())?;
    // What couldn't be read now is said on the account, and tried again.
    let _ = pass_and_record(app, &account).await;
    app.emit(
        SYNCED,
        refresh::status_changed(&data.0, &["calendar"]).await,
    )
    .ok();
    outbox::kick();
    wake();
    let id = account.id.clone();
    Ok(data
        .0
        .read(move |l| l.dav_account(&id))
        .await
        .ok()
        .flatten()
        .unwrap_or(account))
}

/// The calendar accounts, for the sidebar. Never their passwords: those are in the keyring.
#[tauri::command]
pub async fn dav_accounts(data: State<'_, Data>) -> UiResult<Vec<DavAccount>> {
    data.0
        .read(Library::dav_accounts)
        .await
        .map_err(|e| e.to_string())
}

fn open_in_browser(app: &AppHandle, url: &str) -> UiResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| "Skykeep couldn’t open your browser.".to_string())
}

/// Start signing in to a Nextcloud: its page opens in the browser, where the person approves
/// Skykeep. Answers that page's address.
#[tauri::command]
pub async fn nextcloud_begin(server: String, app: AppHandle) -> UiResult<String> {
    let server = nextcloud::normalise_server(&server).map_err(|e| e.to_string())?;
    let host = host_of(&server);
    let http = client_for(std::slice::from_ref(&host)).map_err(|e| e.to_string())?;
    let flow = nextcloud::begin(&http, &server)
        .await
        .map_err(|e| words(&e, &host))?;
    let login = flow.login.clone();
    let signing = Arc::new(Signing {
        flow,
        server,
        http,
        cancelled: Notify::new(),
    });
    let earlier = SIGNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(signing);
    if let Some(earlier) = earlier {
        earlier.cancelled.notify_one();
    }
    open_in_browser(&app, &login)?;
    Ok(login)
}

/// The sign-in page again, for a browser closed too soon.
#[tauri::command]
pub async fn nextcloud_reopen(app: AppHandle) -> UiResult<()> {
    let login = SIGNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.flow.login.clone())
        .ok_or("That sign-in has ended.")?;
    open_in_browser(&app, &login)
}

/// Wait for the person to approve Skykeep, then keep the account and read its calendars.
#[tauri::command]
pub async fn nextcloud_finish(app: AppHandle) -> UiResult<DavAccount> {
    let signing = SIGNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or("That sign-in has ended.")?;
    let host = host_of(&signing.server);
    let began = Instant::now();
    let granted = loop {
        tokio::select! {
            () = signing.cancelled.notified() => return Err("cancelled".into()),
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
        if began.elapsed() > SIGN_IN_LASTS {
            return Err("The sign-in timed out. Try again.".into());
        }
        match nextcloud::poll(&signing.http, &signing.flow).await {
            Ok(Some(granted)) => break granted,
            // Not approved yet; or the network came and went while the person was signing in.
            Ok(None) | Err(Error::Network(_)) => {}
            Err(e) => return Err(words(&e, &host)),
        }
    };
    {
        let mut slot = SIGNING.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &signing)) {
            *slot = None;
        }
    }
    let root = nextcloud::dav_root(&signing.server).map_err(|e| e.to_string())?;
    keep_account(
        &app,
        "nextcloud",
        &signing.server,
        &granted.login_name,
        &granted.app_password,
        &[root],
    )
    .await
}

/// The person gave up on signing in.
#[tauri::command]
pub async fn nextcloud_cancel() -> UiResult<()> {
    let earlier = SIGNING.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(earlier) = earlier {
        earlier.cancelled.notify_one();
    }
    Ok(())
}

/// Where a CalDAV server's calendars may be found from the address given: that address, then
/// the well-known place (RFC 6764), then the server's root and Nextcloud's own place.
fn caldav_starts(server: &Url) -> Vec<Url> {
    let mut starts = Vec::new();
    if server.path() != "/" {
        starts.push(server.clone());
    }
    for tail in ["/.well-known/caldav", "/", "/remote.php/dav/"] {
        let mut url = server.clone();
        url.set_path(tail);
        starts.push(url);
    }
    starts
}

/// Add a CalDAV account by address, username and password.
#[tauri::command]
pub async fn caldav_add(
    server: String,
    username: String,
    password: String,
    app: AppHandle,
) -> UiResult<DavAccount> {
    let url = nextcloud::normalise_address(&server).map_err(|e| e.to_string())?;
    let username = username.trim();
    if username.is_empty() || password.is_empty() {
        return Err("Enter a username and password.".into());
    }
    keep_account(
        &app,
        "caldav",
        &url,
        username,
        &password,
        &caldav_starts(&url),
    )
    .await
}

/// Refresh one account now, including one that needs no longer wait for a sign-in.
#[tauri::command]
pub async fn dav_refresh(id: String, app: AppHandle, data: State<'_, Data>) -> UiResult<()> {
    let account = data
        .0
        .read(move |l| l.dav_account(&id))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("That account is no longer here.")?;
    if account.needs_sign_in {
        return Err(SIGN_IN_AGAIN.to_string());
    }
    pass_and_record(&app, &account).await
}

/// Let an account go: its calendars leave this computer, nothing is deleted on the server, and
/// a Nextcloud is asked to forget the app password made for Skykeep.
#[tauri::command]
pub async fn dav_remove(id: String, app: AppHandle, data: State<'_, Data>) -> UiResult<()> {
    let wanted = id.clone();
    let account = data
        .0
        .read(move |l| l.dav_account(&wanted))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("That account is no longer here.")?;
    if account.kind == "nextcloud" {
        if let (Ok(secret), Ok(http)) = (secret_of(&app, &id), client_for(&account.hosts)) {
            let asked = nextcloud::revoke(&http, &account.server, &account.username, &secret);
            let _ = tokio::time::timeout(Duration::from_secs(10), asked).await;
        }
    }
    {
        let _one = PASS.lock().await;
        data.0
            .lock()
            .await
            .forget_dav_account(&id, &outbox::account_queue(&id))
            .map_err(|e| e.to_string())?;
    }
    forget_remembered(&id);
    tokio::task::block_in_place(|| {
        store(&app).and_then(|s| s.forget_account_secret(&id).map_err(|e| e.to_string()))
    })?;
    app.emit(
        SYNCED,
        refresh::status_changed(&data.0, &["calendar"]).await,
    )
    .ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    //! A pass and a change, end to end, against a server that keeps state and answers as
    //! Sabre (Nextcloud's CalDAV) does: tags that move, `If-Match` refused when stale.
    use super::*;
    use std::sync::Mutex as Shared;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    #[test]
    fn coming_to_calendar_looks_at_the_accounts_unless_just_looked() {
        let now = 1_000_000;
        // Another app in front: the accounts were looked at 45 s ago, on the minute's pace.
        assert!(wakes_for("app-calendar", now, now - 45));
        assert!(wakes_for("main", now, now - 45));
        // Notes draws no events: its focus asks nothing.
        assert!(!wakes_for("app-notes", now, now - 45));
        // Looked at moments ago: nothing to catch up.
        assert!(!wakes_for("app-calendar", now, now - 10));
    }

    const HOME: &str = "/remote.php/dav/calendars/felix/";
    const CAL: &str = "/remote.php/dav/calendars/felix/personal/";

    #[derive(Default)]
    struct Files {
        files: std::collections::BTreeMap<String, (u32, String)>,
        version: u32,
        tag: u32,
    }

    impl Files {
        fn put(&mut self, path: &str, ics: &str) {
            self.version += 1;
            self.tag += 1;
            self.files
                .insert(path.to_string(), (self.version, ics.to_string()));
        }
    }

    #[derive(Clone, Default)]
    struct Server(Arc<Shared<Files>>);

    impl Server {
        fn put(&self, path: &str, ics: &str) {
            self.0.lock().unwrap().put(path, ics);
        }
        fn text(&self, path: &str) -> Option<String> {
            self.0.lock().unwrap().files.get(path).map(|f| f.1.clone())
        }
        fn count(&self) -> usize {
            self.0.lock().unwrap().files.len()
        }
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;")
    }

    impl Respond for Server {
        fn respond(&self, r: &Request) -> ResponseTemplate {
            let mut s = self.0.lock().unwrap();
            let path = r.url.path().to_string();
            let tag_of = |v: u32| format!("&quot;v{v}&quot;");
            let body = String::from_utf8_lossy(&r.body).to_string();
            let header = |name: &str| {
                r.headers
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(ToString::to_string)
            };
            match r.method.as_str() {
                "PROPFIND" if path == "/empty/" => ResponseTemplate::new(207)
                    .set_body_string(r#"<d:multistatus xmlns:d="DAV:"/>"#),
                "PROPFIND" if path == HOME => ResponseTemplate::new(207).set_body_string(format!(
                    r#"<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/"><d:response><d:href>{CAL}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Personal</d:displayname><cs:getctag>c{}</cs:getctag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#,
                    s.tag
                )),
                "REPORT" if body.contains("calendar-multiget") => {
                    let mut out = String::new();
                    for (p, (v, ics)) in &s.files {
                        if body.contains(p.as_str()) {
                            out.push_str(&format!(
                                r#"<d:response><d:href>{p}</d:href><d:propstat><d:prop><d:getetag>{}</d:getetag><cal:calendar-data>{}</cal:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#,
                                tag_of(*v),
                                escape(ics)
                            ));
                        }
                    }
                    ResponseTemplate::new(207).set_body_string(format!(
                        r#"<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav">{out}</d:multistatus>"#
                    ))
                }
                "REPORT" => {
                    let mut out = String::new();
                    for (p, (v, _)) in &s.files {
                        out.push_str(&format!(
                            r#"<d:response><d:href>{p}</d:href><d:propstat><d:prop><d:getetag>{}</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"#,
                            tag_of(*v)
                        ));
                    }
                    ResponseTemplate::new(207)
                        .set_body_string(format!(r#"<d:multistatus xmlns:d="DAV:">{out}</d:multistatus>"#))
                }
                "GET" => match s.files.get(&path) {
                    Some((v, ics)) => ResponseTemplate::new(200)
                        .insert_header("ETag", format!("\"v{v}\"").as_str())
                        .set_body_string(ics.clone()),
                    None => ResponseTemplate::new(404),
                },
                "PUT" => {
                    let now = s.files.get(&path).map(|f| f.0);
                    if header("if-none-match").as_deref() == Some("*") && now.is_some() {
                        return ResponseTemplate::new(412);
                    }
                    if let Some(want) = header("if-match") {
                        if want != "*" && now.map(|v| format!("\"v{v}\"")) != Some(want) {
                            return ResponseTemplate::new(412);
                        }
                    }
                    s.put(&path, &body);
                    ResponseTemplate::new(201)
                }
                "DELETE" => {
                    if s.files.remove(&path).is_some() {
                        s.tag += 1;
                        ResponseTemplate::new(204)
                    } else {
                        ResponseTemplate::new(404)
                    }
                }
                _ => ResponseTemplate::new(405),
            }
        }
    }

    const LUNCH: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:lunch\r\nSUMMARY:Lunch\r\nX-KEEP:yes\r\nDTSTART:20300105T120000Z\r\nDTEND:20300105T130000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    const STANDUP: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:standup\r\nSUMMARY:Standup\r\nDTSTART:20300107T090000Z\r\nDTEND:20300107T093000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    struct World {
        _mock: MockServer,
        server: Server,
        library: LibraryLock,
        remote: Remote,
        account: DavAccount,
        uri: String,
    }

    async fn world() -> World {
        let mock = MockServer::start().await;
        let server = Server::default();
        Mock::given(wiremock::matchers::any())
            .respond_with(server.clone())
            .mount(&mock)
            .await;
        server.put(&format!("{CAL}lunch.ics"), LUNCH);
        server.put(&format!("{CAL}standup.ics"), STANDUP);
        let uri = mock.uri();
        let account = DavAccount {
            id: "a1".into(),
            kind: "nextcloud".into(),
            title: "Nextcloud".into(),
            server: uri.clone(),
            username: "felix".into(),
            home: format!("{uri}{HOME}"),
            hosts: vec!["127.0.0.1".into()],
            ..Default::default()
        };
        let mut lib = Library::in_memory().unwrap();
        lib.put_dav_account(&account).unwrap();
        lib.set_state(WINDOW, "1800000000,2000000000").unwrap();
        let library = LibraryLock::new(lib, None);
        let remote = Remote::new(client_for(&account.hosts).unwrap(), "felix", "secret");
        World {
            _mock: mock,
            server,
            library,
            remote,
            account,
            uri,
        }
    }

    fn calendar_of(uri: &str) -> String {
        caldav::calendar_guid("a1", &format!("{uri}{CAL}"))
    }

    fn lunch_guid(uri: &str) -> String {
        format!("{}:lunch", calendar_of(uri))
    }

    /// Everything waiting in the account's queue, sent.
    async fn send_all(w: &World) {
        let queue = outbox::account_queue("a1");
        loop {
            let next = w.library.lock().await.outbox_take(&queue).unwrap();
            let Some(p) = next else { break };
            let id = p.id;
            send_change(&w.library, &w.remote, &w.account, &p)
                .await
                .unwrap();
            w.library.lock().await.outbox_done(id).unwrap();
        }
    }

    fn save_of(event: Event, base: Option<Event>) -> EventSave {
        EventSave {
            create: base.is_none(),
            event,
            base,
            alerts: None,
            repeat_changed: false,
            repeat: None,
            scope: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_first_pass_brings_the_calendars_and_lays_out_repeats() {
        let w = world().await;
        assert!(pass_with(&w.library, &w.remote, &w.account).await.unwrap());
        let l = w.library.lock().await;
        let calendars = l.calendars().unwrap();
        assert_eq!(calendars.len(), 1);
        assert_eq!(calendars[0].title, "Personal");
        assert_eq!(
            l.event(&lunch_guid(&w.uri)).unwrap().unwrap().title,
            "Lunch"
        );
        let all = l.events_between(1_800_000_000, 2_000_000_000).unwrap();
        assert_eq!(all.len(), 4, "lunch and three standups: {all:?}");
        assert_eq!(all.iter().filter(|e| e.guid.contains("__")).count(), 3);
        drop(l);
        assert!(
            !pass_with(&w.library, &w.remote, &w.account).await.unwrap(),
            "nothing moved, nothing to redraw"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_change_elsewhere_arrives_and_a_deletion_goes() {
        let w = world().await;
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        w.server.put(
            &format!("{CAL}lunch.ics"),
            &LUNCH.replace("Lunch", "Dinner"),
        );
        {
            let mut files = w.server.0.lock().unwrap();
            files.files.remove(&format!("{CAL}standup.ics"));
            files.tag += 1;
        }
        assert!(pass_with(&w.library, &w.remote, &w.account).await.unwrap());
        let l = w.library.lock().await;
        assert_eq!(
            l.event(&lunch_guid(&w.uri)).unwrap().unwrap().title,
            "Dinner"
        );
        assert_eq!(
            l.events_between(1_800_000_000, 2_000_000_000)
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_event_reaches_the_server_under_the_guid_it_was_given() {
        let w = world().await;
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let calendar = calendar_of(&w.uri);
        let guid = new_event_guid(&calendar);
        let event = Event {
            guid: guid.clone(),
            calendar_guid: calendar,
            title: "Dentist".into(),
            starts_at: 1_900_000_000,
            ends_at: 1_900_003_600,
            timezone: Some("UTC".into()),
            ..Default::default()
        };
        {
            let mut l = w.library.lock().await;
            outbox::record_event_save(&mut l, save_of(event, None)).unwrap();
        }
        send_all(&w).await;
        let uid = guid.rsplit(':').next().unwrap();
        let on_server = w
            .server
            .text(&format!("{CAL}{uid}.ics"))
            .expect("on the server");
        assert!(on_server.contains("SUMMARY:Dentist"), "{on_server}");
        assert!(on_server.contains(&format!("UID:{uid}")));
        // The next pass finds the same event, not a copy of it.
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let l = w.library.lock().await;
        let all = l.events_between(1_900_000_000, 1_900_003_600).unwrap();
        assert_eq!(all.len(), 1, "{all:?}");
        assert_eq!(all[0].guid, guid);
        assert!(all[0].etag.is_some());
    }

    /// Changed on another device between the read and the write: the server refuses the stale
    /// version, the change is made again on the new one, and both edits stand.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stale_write_is_made_again_on_the_newer_version() {
        let w = world().await;
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let guid = lunch_guid(&w.uri);
        let base = w.library.lock().await.event(&guid).unwrap().unwrap();
        let mine = Event {
            location: Some("Cafe".into()),
            ..base.clone()
        };
        // Another device renames it, unseen here.
        w.server.put(
            &format!("{CAL}lunch.ics"),
            &LUNCH.replace("Lunch", "Team lunch"),
        );
        {
            let mut l = w.library.lock().await;
            outbox::record_event_save(&mut l, save_of(mine, Some(base))).unwrap();
        }
        send_all(&w).await;
        let text = w.server.text(&format!("{CAL}lunch.ics")).unwrap();
        assert!(text.contains("SUMMARY:Team lunch"), "theirs stands: {text}");
        assert!(text.contains("LOCATION:Cafe"), "mine stands: {text}");
        assert!(
            text.contains("X-KEEP:yes"),
            "what Skykeep has no field for stands: {text}"
        );
        assert_eq!(
            w.library.lock().await.event(&guid).unwrap().unwrap().title,
            "Team lunch"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_one_occurrence_and_a_whole_event() {
        let w = world().await;
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let occurrence = {
            let l = w.library.lock().await;
            let mut all = l.events_between(1_800_000_000, 2_000_000_000).unwrap();
            all.retain(|e| e.guid.contains("__"));
            all.remove(1)
        };
        let lunch = w
            .library
            .lock()
            .await
            .event(&lunch_guid(&w.uri))
            .unwrap()
            .unwrap();
        {
            let mut l = w.library.lock().await;
            outbox::record_event_delete(
                &mut l,
                &occurrence,
                Some(quince_icloud::calendar::Scope::This),
            )
            .unwrap();
            outbox::record_event_delete(&mut l, &lunch, None).unwrap();
        }
        send_all(&w).await;
        assert_eq!(w.server.count(), 1, "lunch is gone, the series stays");
        assert!(w
            .server
            .text(&format!("{CAL}standup.ics"))
            .unwrap()
            .contains("EXDATE"));
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let l = w.library.lock().await;
        assert_eq!(
            l.events_between(1_800_000_000, 2_000_000_000)
                .unwrap()
                .len(),
            2
        );
    }

    /// A change waiting to be sent is not undone by a pass that read the server before it.
    #[tokio::test(flavor = "multi_thread")]
    async fn what_waits_to_be_sent_stays_as_made_through_a_pass() {
        let w = world().await;
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let guid = lunch_guid(&w.uri);
        let base = w.library.lock().await.event(&guid).unwrap().unwrap();
        let mine = Event {
            title: "Mine".into(),
            ..base.clone()
        };
        {
            let mut l = w.library.lock().await;
            outbox::record_event_save(&mut l, save_of(mine, Some(base))).unwrap();
        }
        w.server.put(
            &format!("{CAL}lunch.ics"),
            &LUNCH.replace("X-KEEP:yes", "X-KEEP:changed"),
        );
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        assert_eq!(
            w.library.lock().await.event(&guid).unwrap().unwrap().title,
            "Mine"
        );
    }

    /// A new event the server refused, given up on: gone from the calendar, though no pass
    /// has a reason to look at it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_change_given_up_on_leaves_nothing_behind() {
        let w = world().await;
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let calendar = calendar_of(&w.uri);
        let guid = new_event_guid(&calendar);
        let event = Event {
            guid: guid.clone(),
            calendar_guid: calendar,
            title: "Ghost".into(),
            starts_at: 1_900_000_000,
            ends_at: 1_900_003_600,
            timezone: Some("UTC".into()),
            ..Default::default()
        };
        let mut l = w.library.lock().await;
        outbox::record_event_save(&mut l, save_of(event, None)).unwrap();
        assert!(l.event(&guid).unwrap().is_some(), "shown at once");
        let queue = outbox::account_queue("a1");
        let id = l.outbox_take(&queue).unwrap().unwrap().id;
        l.outbox_refuse(id, "no").unwrap();
        l.outbox_discard(id).unwrap();
        lay_account(&mut l, "a1").unwrap();
        assert!(l.event(&guid).unwrap().is_none());
        assert_eq!(
            l.events_between(1_800_000_000, 2_000_000_000)
                .unwrap()
                .len(),
            4
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_broken_empty_answer_deletes_nothing() {
        let w = world().await;
        pass_with(&w.library, &w.remote, &w.account).await.unwrap();
        let elsewhere = DavAccount {
            home: format!("{}/empty/", w.uri),
            ..w.account.clone()
        };
        let said = pass_with(&w.library, &w.remote, &elsewhere).await;
        assert!(matches!(said, Err(Error::Protocol(_))));
        assert_eq!(w.library.lock().await.calendars().unwrap().len(), 1);
    }

    /// A pass over iCloud (`put_calendars` with Apple's list) never touches these, and a server
    /// that is down holds up only its own queue.
    #[test]
    fn each_account_waits_in_a_queue_of_its_own() {
        assert_eq!(outbox::account_queue("a1"), "caldav:a1");
    }
}
