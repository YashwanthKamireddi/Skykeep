//! One refresh for the whole account.
//!
//! Four services used to each own a button, a spinner and a "last updated" line, so the
//! same work was asked for four times and any one of them could be quietly out of date.
//! There is one pass now: it refreshes everything, records what each service did, and the
//! window states a single honest freshness — the age of the stalest part.
//!
//! Two rules hold the whole thing together.
//!
//! **The network half never holds the library.** Every service fetches first and stores
//! second, so the screens reading SQLite are never blocked behind a slow endpoint.
//!
//! **One service failing changes nothing for the others.** A refresh returns a report per
//! service; the failure is recorded against that service alone and its last good data stays
//! on screen.

use crate::library_lock::LibraryLock;
use quince_auth::Client;
use quince_core::changes::Items;
use quince_icloud::photos::Links;
use quince_sync::pass;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// How a service's refresh ended, and what it did: the shared pass's own (`quince_sync::pass`),
/// which the phone reports with too.
pub use quince_sync::pass::{Outcome, Report as ServiceReport};

/// Everything one refresh pass did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refresh {
    pub services: Vec<ServiceReport>,
    /// The stalest successful refresh across the services that are actually available —
    /// the only freshness the window can honestly claim. `None` while something has never
    /// synced.
    pub as_of: Option<i64>,
    pub took_ms: u64,
    /// The services whose stored rows this changed, when that is known: a screen reads again
    /// only for its own. Absent, every screen reads again, as it always did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed: Option<Vec<&'static str>>,
    /// For services in `changed` whose items could be placed, which ones: added, changed and
    /// removed, by id. A service in `changed` and not here is read whole.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<BTreeMap<&'static str, Items>>,
}

/// The services one pass covers, in the order a person would notice them going stale.
const COVERED: [(&str, &str); 6] = [
    ("calendar", "Calendar"),
    ("reminders", "Reminders"),
    ("notes", "Notes"),
    ("contacts", "Contacts"),
    ("drive", "Drive"),
    ("photos", "Photos"),
];

/// The calendar window a background refresh keeps warm: the month behind, so last week is
/// still there, and a quarter ahead, which is as far as anyone scrolls without meaning to.
fn default_window() -> (String, String, String) {
    let zone = jiff::tz::TimeZone::system();
    let today = jiff::Timestamp::now().to_zoned(zone.clone()).date();
    let start = today
        .first_of_month()
        .checked_sub(jiff::Span::new().months(1))
        .unwrap_or(today);
    let end = today
        .checked_add(jiff::Span::new().months(3))
        .unwrap_or(today)
        .last_of_month();
    (
        start.to_string(),
        end.to_string(),
        zone.iana_name().unwrap_or("UTC").to_string(),
    )
}

/// Apple's web service each app's data comes from, by the app's sidebar key: what a refresh
/// reads it through, and so what the sidebar shows it for. Notes, Reminders and Photos all live
/// in CloudKit (`ckdatabasews`). The sidebar once looked for each app's own name in Apple's
/// list, and some accounts' lists have no `notes` though CloudKit is there: Notes synced, and
/// its row never showed (#9). One list for both, so they can't disagree.
pub(crate) const APP_SERVICES: &[(&str, &str)] = &[
    ("reminders", CLOUDKIT),
    ("notes", CLOUDKIT),
    ("drivews", DRIVE),
    ("photos", CLOUDKIT),
    ("calendar", CALENDAR),
    ("contacts", CONTACTS),
];

const CALENDAR: &str = "calendar";
const CONTACTS: &str = "contacts";
const CLOUDKIT: &str = "ckdatabasews";
const DRIVE: &str = "drivews";

/// The apps whose data Apple's web services for this account can reach, in the sidebar's keys.
pub(crate) fn apps_offered(webservices: &[String]) -> Vec<String> {
    APP_SERVICES
        .iter()
        .filter(|(_, service)| webservices.iter().any(|w| w == service))
        .map(|(app, _)| (*app).to_string())
        .collect()
}

/// What each service needs from the session, taken in one go so the auth lock is released
/// before any network work starts. The CloudKit services' part is the shared pass's context:
/// its account is the one its parameters ask as, read in the same lock.
struct Context {
    shared: pass::Ctx<'static>,
    calendar: Option<String>,
    contacts: Option<String>,
    drive: Option<String>,
}

/// What the desktop does with what a pass found beside its rows.
struct Desktop;

impl pass::Hooks for Desktop {
    fn links(&self, links: HashMap<String, Links>) {
        crate::photos::remember(links);
    }
    fn preparing(&self) {
        crate::push::again_in("photos", PREPARING_AGAIN);
    }
    // Advanced Data Protection keeping it out: the approval is asked for after the pass, and
    // the service read again once it is given (`data_access`).
    fn refused(&self, service: &'static str) {
        crate::data_access::refused(service);
    }
}

static DESKTOP: Desktop = Desktop;

async fn context(client: &Arc<Mutex<Client>>) -> Context {
    let guard = client.lock().await;
    let session = guard.session();
    Context {
        shared: pass::Ctx::new(
            guard.http().clone(),
            guard.service_params(),
            session.service_url(CLOUDKIT),
            crate::data_access::waiting(&guard),
            &DESKTOP,
        ),
        calendar: session.service_url(CALENDAR),
        contacts: session.service_url(CONTACTS),
        drive: session.service_url(DRIVE),
    }
}

/// A service Advanced Data Protection keeps until the next approval, reported as waiting —
/// with when it last synced — and not asked of Apple: asked, it would only be refused.
async fn paused(service: &'static str, data: &LibraryLock) -> ServiceReport {
    pass::paused(service, data).await
}

/// Whether this pass leaves `service` alone for Advanced Data Protection.
fn waits(service: &str, waiting: &[&'static str]) -> bool {
    waiting.contains(&service)
}

/// Told each time one service's part of a pass has been stored, before the rest are done,
/// with whether it changed anything a screen shows, and which items when that is known.
type Landed = Box<dyn Fn(&'static str, bool, Option<Items>) + Send + Sync>;
static LANDED: std::sync::OnceLock<Landed> = std::sync::OnceLock::new();

/// What to do when a service lands: set once, by the app, to tell the open screens.
pub fn on_landed(f: impl Fn(&'static str, bool, Option<Items>) + Send + Sync + 'static) {
    let _ = LANDED.set(Box::new(f));
}

fn landed(report: &ServiceReport) {
    if let Some(f) = LANDED.get() {
        f(report.service, report.changed, report.items.clone());
    }
}

/// Where one service's passes have got to: how many have started, and the last one's report.
struct Turns {
    started: AtomicU64,
    last: std::sync::Mutex<Option<(u64, ServiceReport)>>,
}

impl Turns {
    const fn new() -> Self {
        Self {
            started: AtomicU64::new(0),
            last: std::sync::Mutex::new(None),
        }
    }
}

static TURNS: [Turns; COVERED.len()] = [
    Turns::new(),
    Turns::new(),
    Turns::new(),
    Turns::new(),
    Turns::new(),
    Turns::new(),
];

fn turns(service: &str) -> &'static Turns {
    let at = COVERED.iter().position(|(s, _)| *s == service).unwrap_or(0);
    &TURNS[at]
}

/// Run one service's pass under its pass lock — or, when a pass of it started after this was
/// asked for and has finished while this waited, take that one's report instead of running
/// another.
///
/// Focus in each window, the five-minute timer, every push burst and each window opening all
/// ask for a pass; each used to wait for the one before and then run its own, so a burst of
/// five asked Apple the same questions five times over. A pass that started after the asking
/// has read everything the asking could want, so it answers for all of them: a burst costs at
/// most the pass already running and one more. The second value says whether this ran it.
async fn once<F, Fut>(
    service: &'static str,
    lock: &tokio::sync::Mutex<()>,
    pass: F,
) -> (ServiceReport, bool)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ServiceReport>,
{
    let turns = turns(service);
    let asked = turns.started.load(Ordering::SeqCst);
    let _one = lock.lock().await;
    let done = turns
        .last
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    if let Some((n, report)) = done {
        if n > asked {
            return (report, false);
        }
    }
    // Counted before the pass reads anything, so whoever asks from here on waits for the next.
    let n = turns.started.fetch_add(1, Ordering::SeqCst) + 1;
    // A panic inside one service's pass (a record Apple holds that its reader didn't expect)
    // ends that pass as failed, the other services carrying on (`pass::guarded`, issue #14).
    let report = pass::guarded(service, pass()).await;
    *turns
        .last
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((n, report.clone()));
    (report, true)
}

/// A pass's report, told to the screens as it lands when this pass ran it. One taken from a
/// pass that already ran was told then, and counts as no change here: the screens have read it.
fn arrived((mut report, ran): (ServiceReport, bool)) -> ServiceReport {
    if ran {
        landed(&report);
    } else {
        report.changed = false;
        report.items = None;
    }
    report
}

/// Refresh every service the account has, and report on each.
///
/// Never returns an error: a refresh that partly failed is a normal outcome with a partial
/// result, not an exception, and the interface has to be able to show exactly that.
pub async fn all(client: &Arc<Mutex<Client>>, data: &LibraryLock) -> Refresh {
    run(client, data, &COVERED.map(|(service, _)| service)).await
}

/// Refresh only the services `wanted` names — what a push from Apple says changed — and
/// report on those alone.
pub async fn some(client: &Arc<Mutex<Client>>, data: &LibraryLock, wanted: &[&str]) -> Refresh {
    run(client, data, wanted).await
}

async fn run(client: &Arc<Mutex<Client>>, data: &LibraryLock, services: &[&str]) -> Refresh {
    let wanted = |service: &str| services.contains(&service);
    let started = std::time::Instant::now();
    let ctx = context(client).await;
    let (from, to, zone) = default_window();

    // The services fetched side by side, not one after another: each has its own pass lock,
    // and waiting on Photos' whole library before Reminders was asked anything left every
    // screen empty through a first sync. Each one says it has landed as it does, so its
    // screen fills then rather than when the slowest is done.

    let calendar = async {
        if !wanted("calendar") {
            return None;
        }
        let report = match ctx.calendar.as_deref() {
            None => arrived((unavailable("calendar", "Calendar"), true)),
            Some(root) => arrived(
                once("calendar", &crate::CALENDAR_PASS, || async {
                    // Fetch and store as one, never across a save: see `CALENDAR_PASS`.
                    let fetched = quince_icloud::sync::fetch_calendar(
                        &ctx.shared.http,
                        root,
                        ctx.shared.params.clone(),
                        &from,
                        &to,
                        &zone,
                    )
                    .await;
                    pass::store(
                        "calendar",
                        data,
                        &ctx.shared,
                        fetched,
                        |library, snapshot| {
                            let summary = quince_icloud::sync::apply_calendar(library, snapshot)?;
                            crate::outbox::replay(library, crate::outbox::CALENDAR)?;
                            Ok(format!(
                                "{} event{} in {} calendar{}",
                                summary.events,
                                plural(summary.events),
                                summary.calendars,
                                plural(summary.calendars)
                            ))
                        },
                    )
                    .await
                })
                .await,
            ),
        };
        Some(report)
    };

    let reminders = async {
        if !wanted("reminders") {
            return None;
        }
        let report = match ctx.shared.cloudkit.as_deref() {
            None => arrived((unavailable("reminders", "Reminders"), true)),
            Some(_) if waits("reminders", &ctx.shared.waiting) => paused("reminders", data).await,
            // Stored, with every change still on its way laid back over it, a whole walk once a
            // day: shared with the phone (quince_sync::pass::reminders).
            Some(root) => arrived(
                once("reminders", &crate::REMINDERS_PASS, || {
                    pass::reminders(&ctx.shared, root, data)
                })
                .await,
            ),
        };
        Some(report)
    };

    let notes = async {
        if !wanted("notes") {
            return None;
        }
        let report = match ctx.shared.cloudkit.as_deref() {
            None => arrived((unavailable("notes", "Notes"), true)),
            Some(_) if waits("notes", &ctx.shared.waiting) => paused("notes", data).await,
            // One notes pass at a time, from reading the position through storing what came
            // back — see `NOTES_PASS`. Shared with the phone (quince_sync::pass::notes).
            Some(root) => arrived(
                once("notes", &crate::NOTES_PASS, || {
                    pass::notes(&ctx.shared, root, data)
                })
                .await,
            ),
        };
        Some(report)
    };

    let contacts = async {
        if !wanted("contacts") {
            return None;
        }
        let report = match ctx.contacts.as_deref() {
            None => arrived((unavailable("contacts", "Contacts"), true)),
            Some(root) => arrived(
                once("contacts", &crate::CONTACTS_PASS, || async {
                    // Fetch and store as one, never across a save: see `CONTACTS_PASS`. The book
                    // is read only when Apple says it changed (contacts::pass).
                    let since = pass::token(data, "contacts")
                        .await
                        .and_then(|t| quince_icloud::contacts::Tokens::from_stored(&t));
                    let full = since.is_none();
                    let fetched = quince_icloud::contacts::pass(
                        &ctx.shared.http,
                        root,
                        ctx.shared.params.clone(),
                        since,
                    )
                    .await;
                    pass::store(
                        "contacts",
                        data,
                        &ctx.shared,
                        fetched,
                        move |library, pass| {
                            use quince_icloud::contacts::Pass;
                            let count = match pass {
                                Pass::Read(book, tokens) => {
                                    let n = quince_icloud::contacts::apply(library, book)?;
                                    library.mark_synced("contacts", Some(&tokens.stored()))?;
                                    n
                                }
                                Pass::Unchanged(tokens) => {
                                    library.mark_synced("contacts", Some(&tokens.stored()))?;
                                    let n: i64 = library
                                        .conn()
                                        .query_row("SELECT count(*) FROM contacts", [], |r| {
                                            r.get(0)
                                        })
                                        .map_err(quince_core::Error::from)?;
                                    usize::try_from(n).unwrap_or(0)
                                }
                            };
                            // Cards saved here and still on their way to Apple stay as saved.
                            crate::outbox::replay(library, crate::outbox::CONTACTS)?;
                            if full {
                                library.mark_full_sync("contacts")?;
                            }
                            Ok(format!("{count} contact{}", plural(count)))
                        },
                    )
                    .await
                })
                .await,
            ),
        };
        Some(report)
    };

    // Drive is read a folder at a time as it is opened; the pass keeps its top level current,
    // so what changed there on another device is here before anyone looks, and its freshness
    // is stated with the rest. Fetched and stored as one — see `drive_pass`.
    let drive = async {
        if !wanted("drive") {
            return None;
        }
        let report = match ctx.drive.as_deref() {
            None => arrived((unavailable("drive", "Drive"), true)),
            Some(_) if waits("drive", &ctx.shared.waiting) => paused("drive", data).await,
            Some(root) => {
                let lock = crate::drive_pass(quince_core::drive::ROOT);
                arrived(
                    once("drive", &lock, || async {
                        let fetched = quince_icloud::drive::fetch_folder(
                            &ctx.shared.http,
                            root,
                            ctx.shared.params.clone(),
                            quince_core::drive::ROOT,
                        )
                        .await;
                        pass::store("drive", data, &ctx.shared, fetched, |library, items| {
                            let n = quince_icloud::drive::store_folder(
                                library,
                                quince_core::drive::ROOT,
                                &items,
                            )?;
                            Ok(format!("{n} item{} at the top of iCloud Drive", plural(n)))
                        })
                        .await
                    })
                    .await,
                )
            }
        };
        Some(report)
    };

    // Photos: the whole library through the change feed once, then only what changed — under
    // half a second when nothing did. Fetched and stored as one: see `PHOTOS_PASS`.
    // A large library is read a few pages at a time, each stored before the next is asked for;
    // a library Apple is still preparing is not read; then Shared Albums and the Shared
    // Library. Shared with the phone (quince_sync::pass::photos).
    let photos = async {
        if !wanted("photos") {
            return None;
        }
        let report = match ctx.shared.cloudkit.as_deref() {
            None => arrived((unavailable("photos", "Photos"), true)),
            Some(_) if waits("photos", &ctx.shared.waiting) => paused("photos", data).await,
            Some(root) => arrived(
                once("photos", &crate::PHOTOS_PASS, || {
                    pass::photos(&ctx.shared, root, data)
                })
                .await,
            ),
        };
        Some(report)
    };

    let (calendar, reminders, notes, contacts, drive, photos) =
        tokio::join!(calendar, reminders, notes, contacts, drive, photos);
    // Apple answered: whatever waits to be sent goes now, not on the outbox's own timer.
    if [&calendar, &reminders, &notes, &contacts]
        .iter()
        .any(|r| r.as_ref().is_some_and(|r| r.outcome == Outcome::Ok))
    {
        crate::outbox::kick();
    }
    let reports: Vec<ServiceReport> = [calendar, reminders, notes, contacts, drive, photos]
        .into_iter()
        .flatten()
        .collect();

    let available: Vec<&str> = reports
        .iter()
        .filter(|r| r.outcome != Outcome::Unavailable)
        .map(|r| r.service)
        .collect();
    let as_of = data
        .read(|library| library.oldest_sync(&available))
        .await
        .ok()
        .flatten();
    let changed: Vec<_> = reports
        .iter()
        .filter(|r| r.changed)
        .map(|r| r.service)
        .collect();
    // New or changed photos reach the Photos folder in File Explorer too (Windows), whether or
    // not a window is open; off the async threads, as it touches the disk.
    if changed.contains(&"photos") {
        tauri::async_runtime::spawn_blocking(crate::mount::photos_changed);
    }
    // The backup carried on after every Photos sync, changed or not: a launch in the tray at
    // login, with nothing new since, still finishes a backup left part way. When nothing is
    // left to copy it is one query.
    if reports
        .iter()
        .any(|r| r.service == "photos" && r.outcome == Outcome::Ok)
    {
        crate::backup::photos_changed();
    }
    let items = reports
        .iter()
        .filter(|r| r.changed)
        .filter_map(|r| Some((r.service, r.items.clone()?)))
        .collect();

    Refresh {
        services: reports,
        as_of,
        took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        changed: Some(changed),
        items: Some(items),
    }
}

/// [`status`], saying whose rows changed: only those apps' screens read their lists again, and
/// with none named, none does — the freshness alone is new. Sent without it, every open screen
/// read its whole list again (Photos its whole index) for a calendar feed fetched, a
/// subscription loaded or a pass already told as it landed.
pub async fn status_changed(data: &LibraryLock, changed: &[&'static str]) -> Refresh {
    Refresh {
        changed: Some(changed.to_vec()),
        ..status(data).await
    }
}

/// [`status`], for one service that has just landed: whose rows changed and, when known, which
/// of its items — so its screen redraws those rows and every other screen stays as it is.
pub async fn status_landed(
    data: &LibraryLock,
    service: &'static str,
    changed: bool,
    items: Option<Items>,
) -> Refresh {
    Refresh {
        changed: Some(if changed { vec![service] } else { Vec::new() }),
        items: Some(match items {
            Some(items) if changed => BTreeMap::from([(service, items)]),
            _ => BTreeMap::new(),
        }),
        ..status(data).await
    }
}

/// What the library already knows, without asking Apple anything.
///
/// The window renders from this the instant it opens, so freshness is on screen before the
/// first refresh has finished rather than after it.
pub async fn status(data: &LibraryLock) -> Refresh {
    let covered: Vec<&str> = COVERED.iter().map(|(service, _)| *service).collect();
    // Read beside a pass, not behind it: asked as each service lands, it used to wait for the
    // next service's store to finish.
    let (health, as_of) = data
        .read(|library| {
            Ok((
                library.sync_health().unwrap_or_default(),
                library.oldest_sync(&covered).ok().flatten(),
            ))
        })
        .await
        .unwrap_or_default();
    let services = COVERED
        .iter()
        .map(|(service, label)| {
            let found = health.iter().find(|h| h.service == *service);
            let error = found.and_then(|h| h.last_error.clone());
            ServiceReport {
                service,
                label,
                outcome: if error.is_some() {
                    Outcome::Failed
                } else {
                    Outcome::Ok
                },
                detail: error,
                last_synced_at: found.and_then(|h| h.last_synced_at),
                changed: false,
                items: None,
                more: false,
            }
        })
        .collect();
    Refresh {
        services,
        as_of,
        took_ms: 0,
        changed: None,
        items: None,
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// How long until a library Apple is still preparing is looked at again.
pub const PREPARING_AGAIN: std::time::Duration = std::time::Duration::from_secs(120);

fn unavailable(service: &'static str, label: &'static str) -> ServiceReport {
    ServiceReport {
        label,
        ..pass::unavailable(service)
    }
}

#[cfg(test)]
mod tests {
    use quince_sync::pass::{joined, MOST_ITEMS};

    /// The Photos report while Apple prepares the library, with another look set for later,
    /// as the desktop's pass makes it (the shared pass, then the desktop's hook).
    async fn preparing(
        data: &LibraryLock,
        ready: quince_icloud::Result<quince_icloud::photos::Readiness>,
    ) -> Option<ServiceReport> {
        use quince_sync::pass::Hooks;
        let report = pass::preparing(data, ready).await;
        if report.is_some() {
            DESKTOP.preparing();
        }
        report
    }

    /// The sidebar's apps (Library.tsx's SCREENS) and the services they read (APP_SERVICES)
    /// are one list: an app added or renamed in one and not the other fails here, never in a
    /// person's sidebar.
    #[test]
    fn every_sidebar_app_has_its_service_and_no_other() {
        let screens = include_str!("../../src/screens/Library.tsx");
        let start = screens
            .find("const SCREENS")
            .expect("SCREENS in Library.tsx");
        let list = &screens[start..start + screens[start..].find("];").expect("end of SCREENS")];
        let keys: Vec<&str> = list
            .split("key: \"")
            .skip(1)
            .filter_map(|k| k.split('"').next())
            .collect();
        let ours: Vec<&str> = APP_SERVICES.iter().map(|(app, _)| *app).collect();
        assert_eq!(
            keys, ours,
            "Library.tsx's SCREENS and APP_SERVICES must list the same apps"
        );
    }

    /// An app shows when the service its data comes from is offered, whatever else is listed:
    /// an account whose list has CloudKit but no `notes` still gets Notes (#9); one without
    /// CloudKit gets none of the three that live there.
    #[test]
    fn an_app_shows_when_the_service_it_reads_is_there() {
        let list = |w: &[&str]| w.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let offered = apps_offered(&list(&[
            "ckdatabasews",
            "reminders",
            "photos",
            "calendar",
            "contacts",
            "drivews",
        ]));
        assert_eq!(
            offered,
            [
                "reminders",
                "notes",
                "drivews",
                "photos",
                "calendar",
                "contacts"
            ]
        );
        let offered = apps_offered(&list(&["calendar", "contacts", "notes", "reminders"]));
        assert_eq!(offered, ["calendar", "contacts"]);
    }
    use super::*;

    /// Apple still preparing the library: the pass reads nothing (so stores no empty library
    /// over the one being made), says so calmly as its own state rather than a failure or an
    /// empty library, and sets another look for later. Ready, or Apple not saying, the read
    /// goes ahead.
    /// A pass that panics ends as failed, and the refresh it is part of still finishes (#14).
    #[tokio::test]
    async fn a_pass_that_panics_fails_alone() {
        let lock = tokio::sync::Mutex::new(());
        let (report, ran) = super::once("reminders", &lock, || async {
            if lock.try_lock().is_err() {
                panic!("invalid time");
            }
            unreachable!()
        })
        .await;
        assert!(ran);
        assert_eq!(report.outcome, super::Outcome::Failed);
        assert_eq!(report.label, "Reminders");
        assert!(report.detail.unwrap().contains("invalid time"));
        // The pass lock is let go: the next pass runs.
        assert!(lock.try_lock().is_ok());
    }

    #[tokio::test]
    async fn a_library_apple_is_still_preparing_waits_and_says_so() {
        use quince_icloud::photos::Readiness;
        let data = LibraryLock::new(quince_core::Library::in_memory().unwrap(), None);
        let report = preparing(&data, Ok(Readiness::Preparing { progress: Some(0) }))
            .await
            .expect("not read while preparing");
        assert_eq!(report.outcome, Outcome::Preparing);
        assert_eq!(
            report.detail.as_deref(),
            Some(
                "Apple is still preparing your photo library. It will appear here when it's ready."
            )
        );
        assert!(!report.changed);
        assert_eq!(
            serde_json::to_value(&report).unwrap()["outcome"],
            "preparing",
            "the window's name for it"
        );
        assert!(
            crate::push::looks_set().contains(&"photos"),
            "looked at again later"
        );
        assert!(preparing(&data, Ok(Readiness::Ready)).await.is_none());
        assert!(preparing(&data, Err(quince_icloud::Error::Busy))
            .await
            .is_none());
    }

    /// A freshness sent with the services it changed names exactly those, and one that
    /// changed nothing names none — never "unknown", which has every screen read again.
    #[tokio::test]
    async fn a_status_sent_names_whose_rows_changed() {
        let data = LibraryLock::new(quince_core::Library::in_memory().unwrap(), None);
        let told = status_changed(&data, &["calendar"]).await;
        assert_eq!(told.changed, Some(vec!["calendar"]));
        let json = serde_json::to_value(&told).unwrap();
        assert_eq!(json["changed"], serde_json::json!(["calendar"]));
        let quiet = status_changed(&data, &[]).await;
        assert_eq!(quiet.changed, Some(Vec::new()));
        assert_eq!(
            serde_json::to_value(&quiet).unwrap()["changed"],
            serde_json::json!([])
        );
        assert_eq!(quiet.services.len(), status(&data).await.services.len());
    }

    /// A service landing names its items with it — for its own screen alone — and one that
    /// stored nothing new names none.
    #[tokio::test]
    async fn a_landing_names_the_items_it_changed() {
        let data = LibraryLock::new(quince_core::Library::in_memory().unwrap(), None);
        let items = Items {
            added: vec!["A".into()],
            changed: vec!["B".into()],
            removed: vec![],
        };
        let told = status_landed(&data, "contacts", true, Some(items)).await;
        let json = serde_json::to_value(&told).unwrap();
        assert_eq!(json["changed"], serde_json::json!(["contacts"]));
        assert_eq!(
            json["items"],
            serde_json::json!({"contacts": {"added": ["A"], "changed": ["B"], "removed": []}})
        );
        let whole = status_landed(&data, "contacts", true, None).await;
        assert_eq!(
            serde_json::to_value(&whole).unwrap()["items"],
            serde_json::json!({}),
            "changed, with no items named: read whole"
        );
        let quiet = status_landed(&data, "contacts", false, None).await;
        assert_eq!(
            serde_json::to_value(&quiet).unwrap()["changed"],
            serde_json::json!([])
        );
    }

    /// Photos' rounds put together: an item two rounds touched is one change, and too many
    /// are read whole.
    #[test]
    fn rounds_join_into_one_set_of_items() {
        let ids = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let first = Items {
            added: ids(&["a"]),
            changed: ids(&["b"]),
            removed: vec![],
        };
        let second = Items {
            added: ids(&["c"]),
            changed: ids(&["a"]),
            removed: ids(&["d"]),
        };
        let all = joined(first, second).unwrap();
        assert_eq!(all.added, ids(&["c"]));
        assert_eq!(all.changed, ids(&["b", "a"]));
        assert_eq!(all.removed, ids(&["d"]));
        let many = Items {
            added: (0..=MOST_ITEMS).map(|n| n.to_string()).collect(),
            changed: vec![],
            removed: vec![],
        };
        assert_eq!(joined(Items::default(), many), None);
    }

    #[test]
    fn the_default_window_reaches_back_a_month_and_forward_a_quarter() {
        let (from, to, _zone) = default_window();
        let start: jiff::civil::Date = from.parse().unwrap();
        let end: jiff::civil::Date = to.parse().unwrap();
        let today = jiff::Timestamp::now()
            .to_zoned(jiff::tz::TimeZone::system())
            .date();

        assert!(
            start < today,
            "the window must include what already happened"
        );
        assert!(end > today, "and what is coming");
        assert_eq!(start.day(), 1, "a month starts on the first");
        let days = (end - start).get_days();
        assert!(
            (110..=160).contains(&days),
            "about five months of window, got {days}"
        );
    }

    /// The window is the same shape wherever the machine is, so a refresh never silently
    /// covers less for someone in a different timezone.
    #[test]
    fn the_window_is_a_whole_number_of_months() {
        let (from, to, _) = default_window();
        let start: jiff::civil::Date = from.parse().unwrap();
        let end: jiff::civil::Date = to.parse().unwrap();
        assert_eq!(start.day(), 1);
        assert_eq!(
            end.day(),
            end.last_of_month().day(),
            "the window ends on a month boundary"
        );
    }

    #[test]
    fn counts_read_as_english() {
        assert_eq!(plural(1), "");
        assert_eq!(plural(0), "s");
        assert_eq!(plural(2), "s");
    }

    /// Five asks while a pass is running cost that pass and one more, not five: each that
    /// waited takes the report of the pass that started after it asked. An ask after that
    /// one has finished runs a pass of its own.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_burst_of_asks_costs_the_running_pass_and_one_more() {
        use std::sync::atomic::AtomicUsize;
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let gate = Arc::new(tokio::sync::Notify::new());
        let pass = |gate: Option<Arc<tokio::sync::Notify>>| {
            || async move {
                let n = RUNS.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                let mut report = unavailable("contacts", "Contacts");
                report.detail = Some(format!("pass {n}"));
                report
            }
        };
        let first = {
            let gate = Arc::clone(&gate);
            tokio::spawn(async move { once("contacts", &LOCK, pass(Some(gate))).await })
        };
        while RUNS.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let waiting: Vec<_> = (0..5)
            .map(|_| tokio::spawn(async move { once("contacts", &LOCK, pass(None)).await }))
            .collect();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        gate.notify_one();
        let (report, ran) = first.await.unwrap();
        assert!(ran);
        assert_eq!(report.detail.as_deref(), Some("pass 1"));
        let mut ran_here = 0;
        for w in waiting {
            let (report, ran) = w.await.unwrap();
            ran_here += usize::from(ran);
            assert_eq!(
                report.detail.as_deref(),
                Some("pass 2"),
                "every waiting ask is answered by the pass after the one running"
            );
        }
        assert_eq!(ran_here, 1);
        assert_eq!(RUNS.load(Ordering::SeqCst), 2);

        let (report, ran) = once("contacts", &LOCK, pass(None)).await;
        assert!(ran, "an ask after the burst runs its own pass");
        assert_eq!(report.detail.as_deref(), Some("pass 3"));
    }
}

#[cfg(test)]
mod paused_tests {
    use super::*;
    use quince_sync::pass::PAUSED_WORDS;

    #[test]
    fn a_waiting_service_is_left_alone_and_the_rest_are_not() {
        let waiting = ["notes", "reminders", "photos", "drive"];
        for s in waiting {
            assert!(waits(s, &waiting));
        }
        for s in ["calendar", "contacts"] {
            assert!(
                !waits(s, &waiting),
                "{s} is never held by Advanced Data Protection"
            );
        }
        assert!(!waits("notes", &[]), "approved: nothing is left alone");
    }

    #[test]
    fn waiting_reads_as_waiting_not_failure() {
        assert_eq!(serde_json::to_value(Outcome::Paused).unwrap(), "paused");
        assert!(!PAUSED_WORDS.to_lowercase().contains("error"));
    }
}
