//! One sync pass of a service, the same on the desktop and the phone.
//!
//! The desktop's refresh (`src-tauri/src/refresh.rs`) grew everything a pass needs around the
//! bare fetch-and-store: whose library it is, a whole walk once a day, Advanced Data
//! Protection's pause, a panic kept to the pass it happened in, the failure recorded where the
//! screens read it, Photos' readiness, and Shared Albums and the Shared Library after the
//! library. The phone called the bare fetch-and-store and had none of it. Both run it from here
//! now, so a pass behaves the same wherever it runs.
//!
//! What a pass holds to:
//!
//! * **The network half never holds the library.** Each service fetches first and stores
//!   second, taking the library as a pass takes it (`LibraryLock::lock_for_pass`).
//! * **Accounts never mix.** The account a pass stores for is the one its session parameters
//!   ask as ([`Ctx::new`]), taken with them in one go; the store checks the library still
//!   names that account under the same lock it writes with. A pass read under one account's
//!   session never lands in the next account's library.
//! * **A delta is not enough forever.** A tombstone Apple sends once and a pass missed would
//!   keep a deleted record for good, so once every [`FULL_SYNC_EVERY`] a service is walked
//!   whole ([`token`]).
//! * **One service failing changes nothing for the others**, a panic included ([`guarded`]):
//!   the failure is recorded against that service alone and its last good data stays.
//!
//! Calendar and Contacts plug into the same pieces ([`store`], [`token`], [`guarded`]) with
//! their own fetch and apply.

use crate::library_lock::LibraryLock;
use futures_util::FutureExt;
use quince_core::changes::Items;
use quince_core::Library;
use quince_icloud::photos::Links;
use serde::Serialize;
use std::collections::HashMap;

pub const NOTES: &str = "notes";
pub const REMINDERS: &str = "reminders";
pub const PHOTOS: &str = "photos";

/// How often a service reconciles against everything Apple has, rather than the changes
/// since last time.
///
/// A delta is what almost every refresh should be — seconds rather than a minute. But a
/// delta can only apply what Apple chose to send, and a tombstone is sent once, so a record
/// whose removal was missed would stay forever. Once a day is often enough to be a floor
/// and rare enough that nobody notices the cost.
pub const FULL_SYNC_EVERY: i64 = 24 * 60 * 60;

/// More items than this changed in one go and the screen reads its list whole: one read of
/// everything is quicker than asking after each.
pub const MOST_ITEMS: usize = 500;

/// What a waiting service says.
pub const PAUSED_WORDS: &str = "Syncs when you approve Skykeep on your iPhone, iPad or Mac.";

/// Said while Apple prepares the library: calm, and true.
pub const PREPARING_WORDS: &str =
    "Apple is still preparing your photo library. It will appear here when it's ready.";

/// Said for a pass read under an account that no longer owns the library.
pub const NOT_YOURS_WORDS: &str = "Signed out while this was being read.";

/// How a service's pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Refreshed cleanly.
    Ok,
    /// Apple does not offer this service for the account, so there is nothing to refresh.
    Unavailable,
    /// It was tried and failed. What is on screen is the last good data.
    Failed,
    /// Advanced Data Protection keeps it until the person next approves on one of their
    /// devices: not asked for, not failed. What is on screen is the copy here, and changes
    /// made meanwhile wait to be sent.
    Paused,
    /// Apple is still preparing it (a Photos library just turned on): nothing was read, and
    /// it is looked at again in a while. Not empty, and not failed.
    Preparing,
}

/// What one service's pass did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub service: &'static str,
    pub label: &'static str,
    pub outcome: Outcome,
    /// A sentence for the person: what came back, or what went wrong.
    pub detail: Option<String>,
    pub last_synced_at: Option<i64>,
    /// It stored something a screen shows — not only where it got to. Told to the screens with
    /// the pass, never sent with the report itself.
    #[serde(skip)]
    pub changed: bool,
    /// Which of the app's items it changed, when that could be placed: its screen redraws those
    /// rows alone. `None`, it reads its list whole.
    #[serde(skip)]
    pub items: Option<Items>,
    /// Apple has more than this pass's rounds took (a large Photos library): the next pass
    /// carries on, and may as well start now.
    #[serde(skip)]
    pub more: bool,
}

impl Report {
    fn of(service: &'static str, outcome: Outcome, detail: Option<String>) -> Self {
        Self {
            service,
            label: label(service),
            outcome,
            detail,
            last_synced_at: None,
            changed: false,
            items: None,
            more: false,
        }
    }
}

/// The app's name for a service, as the person reads it.
#[must_use]
pub fn label(service: &str) -> &'static str {
    match service {
        "calendar" => "Calendar",
        REMINDERS => "Reminders",
        NOTES => "Notes",
        "contacts" => "Contacts",
        "drive" => "Drive",
        PHOTOS => "Photos",
        _ => "iCloud",
    }
}

/// What the platform does with what a pass found beside its rows. All of it optional: the
/// phone keeps download links and takes back an approval Apple let go of; the desktop also
/// looks at a library Apple is preparing again later.
pub trait Hooks: Sync {
    /// Download links Apple sent with the photos, by master id: good for a while.
    fn links(&self, _links: HashMap<String, Links>) {}
    /// Apple is still preparing the photo library: looked at again later.
    fn preparing(&self) {}
    /// Advanced Data Protection refused `service` for want of its key, though one was held:
    /// the approval is asked for again (`Client::data_access_lapsed`), never retried blindly.
    fn refused(&self, _service: &'static str) {}
}

/// Hooks that do nothing.
pub struct NoHooks;
impl Hooks for NoHooks {}

/// What a pass needs from the session, taken in one go (one lock of the session) so the
/// account it stores for is always the one its requests ask as.
pub struct Ctx<'a> {
    pub http: reqwest::Client,
    pub params: Vec<(String, String)>,
    /// CloudKit's address (`ckdatabasews`), where Notes, Reminders and Photos live; `None`
    /// when Apple doesn't offer it to this account.
    pub cloudkit: Option<String>,
    /// The account it is all for (`queue::account_tag`): nothing it brings is kept in a
    /// library that is not that account's.
    pub account: Option<String>,
    /// Services Advanced Data Protection keeps until the next approval: left alone, Apple not
    /// asked (it would only refuse).
    pub waiting: Vec<&'static str>,
    pub hooks: &'a dyn Hooks,
}

impl<'a> Ctx<'a> {
    /// A pass's context from the session's client and parameters: the account it stores for
    /// is the one `params` ask as.
    #[must_use]
    pub fn new(
        http: reqwest::Client,
        params: Vec<(String, String)>,
        cloudkit: Option<String>,
        waiting: Vec<&'static str>,
        hooks: &'a dyn Hooks,
    ) -> Self {
        let account = crate::queue::asked_as(&params);
        Self {
            http,
            params,
            cloudkit,
            account,
            waiting,
            hooks,
        }
    }

    /// Whether this pass leaves `service` alone for Advanced Data Protection.
    #[must_use]
    pub fn waits(&self, service: &str) -> bool {
        self.waiting.contains(&service)
    }
}

/// One whole pass of Notes, Reminders or Photos, as both platforms run it: offered or not,
/// waiting on an approval or not, under the service's pass lock, a panic kept to it, and a
/// failure recorded. Any other service is reported as not offered: Calendar and Contacts run
/// their own fetch and apply through [`store`].
pub async fn run(
    service: &'static str,
    ctx: &Ctx<'_>,
    data: &LibraryLock,
    lock: &tokio::sync::Mutex<()>,
) -> Report {
    let Some(root) = ctx.cloudkit.as_deref() else {
        return unavailable(service);
    };
    if ![NOTES, REMINDERS, PHOTOS].contains(&service) {
        return unavailable(service);
    }
    if ctx.waits(service) {
        return paused(service, data).await;
    }
    driven(service, ctx, data, lock, async {
        match service {
            NOTES => notes(ctx, root, data).await,
            REMINDERS => reminders(ctx, root, data).await,
            _ => photos(ctx, root, data).await,
        }
    })
    .await
}

/// A service's pass under its pass lock, a panic in it ending that pass alone as failed and
/// recorded where the screens read it — in the library, while it is still this account's.
/// What [`run`] does for its own services, and what Calendar's and Contacts' passes can go
/// through with their own fetch and apply.
pub async fn driven<F>(
    service: &'static str,
    ctx: &Ctx<'_>,
    data: &LibraryLock,
    lock: &tokio::sync::Mutex<()>,
    pass: F,
) -> Report
where
    F: std::future::Future<Output = Report>,
{
    let _one = lock.lock().await;
    match std::panic::AssertUnwindSafe(pass).catch_unwind().await {
        Ok(report) => report,
        Err(panic) => {
            let report = failed_by_panic(service, panic.as_ref());
            let library = data.lock_for_pass().await;
            if owned(&library, ctx.account.as_deref()) {
                if let Some(why) = report.detail.as_deref() {
                    library.mark_sync_failed(service, why).ok();
                }
            }
            report
        }
    }
}

/// A pass, with a panic inside it ending that pass as failed. Before, a panic (a record Apple
/// holds that its reader didn't expect) unwound the whole refresh: the other services never
/// stored what they read, and Sync Now waited forever (issue #14).
pub async fn guarded<F>(service: &'static str, pass: F) -> Report
where
    F: std::future::Future<Output = Report>,
{
    match std::panic::AssertUnwindSafe(pass).catch_unwind().await {
        Ok(report) => report,
        Err(panic) => failed_by_panic(service, panic.as_ref()),
    }
}

/// The report of a pass that panicked: failed, saying so plainly, with the panic in the log.
#[must_use]
pub fn failed_by_panic(service: &'static str, panic: &(dyn std::any::Any + Send)) -> Report {
    let what = panic
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".into());
    crate::quiet!("{service} pass panicked: {what}");
    let label = label(service);
    Report::of(
        service,
        Outcome::Failed,
        Some(format!(
            "Skykeep couldn’t read something in {label} ({what}). The other apps carried on."
        )),
    )
}

/// A service Apple doesn't offer this account.
#[must_use]
pub fn unavailable(service: &'static str) -> Report {
    Report::of(service, Outcome::Unavailable, None)
}

/// A service Advanced Data Protection keeps until the next approval, reported as waiting —
/// with when it last synced — and not asked of Apple: asked, it would only be refused.
pub async fn paused(service: &'static str, data: &LibraryLock) -> Report {
    Report {
        last_synced_at: data
            .read(|library| Ok(library.last_synced(service).ok().flatten()))
            .await
            .ok()
            .flatten(),
        ..Report::of(service, Outcome::Paused, Some(PAUSED_WORDS.into()))
    }
}

/// Whether the library is `account`'s: it names that account as its owner.
#[must_use]
pub fn owned(library: &Library, account: Option<&str>) -> bool {
    let owner = library.state(crate::queue::ACCOUNT_KEY).ok().flatten();
    account.is_some() && owner.as_deref() == account
}

/// Where the last sync got to — or `None` when this service is due a full pass, which is
/// what makes the next fetch walk everything: none yet, or the last whole walk more than
/// [`FULL_SYNC_EVERY`] ago.
pub async fn token(data: &LibraryLock, service: &str) -> Option<String> {
    // Read beside the other services' stores, which the passes run alongside.
    data.read(|library| {
        if library
            .needs_full_sync(service, FULL_SYNC_EVERY)
            .unwrap_or(false)
        {
            return Ok(None);
        }
        Ok(library.sync_token(service).ok().flatten())
    })
    .await
    .ok()
    .flatten()
}

/// SQLite is synchronous: a full pass's apply (a whole photo library, every reminder and its
/// alarms) is CPU and disk work that would otherwise sit on one of the runtime's few workers,
/// stalling every task queued behind it there. Run in place on a multi-threaded runtime, its
/// worker hands those tasks on first; anywhere else it simply runs.
fn in_place<T>(f: impl FnOnce() -> T) -> T {
    let multi = tokio::runtime::Handle::try_current()
        .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
    if multi {
        tokio::task::block_in_place(f)
    } else {
        f()
    }
}

/// What every pass writes whatever came back: where it got to, and when.
const BOOKKEEPING: &[&str] = &["sync_state"];

/// Store what a fetch returned, and turn either half's failure into a report.
///
/// The library is taken only here — after the network work is done — and released as soon
/// as the rows are in. Taken as a pass takes it: what it stores is one transaction, and the
/// screens read beside it meanwhile (`LibraryLock::lock_for_pass`). Fetched for an account
/// that is no longer the library's, nothing is written, not even the failure.
pub async fn store<T, F>(
    service: &'static str,
    data: &LibraryLock,
    ctx: &Ctx<'_>,
    fetched: quince_icloud::Result<T>,
    apply: F,
) -> Report
where
    F: FnOnce(&mut Library, T) -> quince_icloud::Result<String>,
{
    let mut library = data.lock_for_pass().await;
    // Fetched for an account that is no longer the library's — signed out, or another signed
    // in, while it was on the way: dropped, never laid over someone else's data.
    if !owned(&library, ctx.account.as_deref()) {
        return Report::of(service, Outcome::Failed, Some(NOT_YOURS_WORDS.into()));
    }
    let started = std::time::Instant::now();
    // And as one change: the records, the changes still waiting laid over them and the new
    // position land together, or — failing, or the app dying part way — none of them do.
    // Whether it stored anything beyond its position is noted on the way, so a pass that found
    // nothing new does not have every screen read its whole list again.
    //
    // Which items it changed is noted too — rows stored again just as they were cancel out —
    // so the screen redraws those rows alone, and a pass that rewrote everything as it was
    // (a whole address book read again for one card's sake) leaves the rest untouched.
    let (result, seen) = in_place(|| {
        library.changing_items(BOOKKEEPING, |library| {
            fetched.and_then(|value| library.atomically(|library| apply(library, value)))
        })
    });
    let items = seen.for_service(service).filter(|i| i.len() <= MOST_ITEMS);
    // Written, but nothing it shows is different: no screen has anything to read.
    let changed = seen.written && items.as_ref().is_none_or(|i| !i.is_empty());
    let took = started.elapsed();
    if took > std::time::Duration::from_millis(50) {
        crate::quiet!("{service}: stored in {} ms", took.as_millis());
    }
    let last_synced_at = library.last_synced(service).ok().flatten();
    match result {
        Ok(detail) => Report {
            last_synced_at,
            changed,
            items: if changed { items } else { None },
            ..Report::of(service, Outcome::Ok, Some(detail))
        },
        // Advanced Data Protection keeping it out: the approval is asked for again, and the
        // service read once it is given. Not a failure, and never retried before that.
        Err(quince_icloud::Error::NeedsDeviceApproval) => {
            ctx.hooks.refused(service);
            Report {
                last_synced_at,
                ..Report::of(service, Outcome::Paused, Some(PAUSED_WORDS.into()))
            }
        }
        Err(e) => {
            let message = e.to_string();
            library.mark_sync_failed(service, &message).ok();
            Report {
                last_synced_at,
                ..Report::of(service, Outcome::Failed, Some(message))
            }
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Reminders: from where the last pass got to (or the start, once a day), stored with every
/// change still on its way laid back over it.
pub async fn reminders(ctx: &Ctx<'_>, root: &str, data: &LibraryLock) -> Report {
    let since = token(data, REMINDERS).await;
    let full = since.is_none();
    let fetched = quince_icloud::reminders::fetch_cloudkit(
        &ctx.http,
        root,
        ctx.params.clone(),
        since.as_deref(),
    )
    .await;
    store(REMINDERS, data, ctx, fetched, move |library, f| {
        let (lists, count) = crate::reminders::apply(library, f, full)?;
        Ok(format!(
            "{count} reminder{} in {lists} list{}",
            plural(count),
            plural(lists)
        ))
    })
    .await
}

/// Notes: as Reminders, with every edit still on its way laid back over what Apple sent.
pub async fn notes(ctx: &Ctx<'_>, root: &str, data: &LibraryLock) -> Report {
    let since = token(data, NOTES).await;
    let full = since.is_none();
    let fetched =
        quince_icloud::notes::fetch_cloudkit(&ctx.http, root, ctx.params.clone(), since.as_deref())
            .await;
    store(NOTES, data, ctx, fetched, move |library, f| {
        let (folders, notes) = crate::notes::apply(library, f, full)?;
        Ok(format!(
            "{notes} note{} in {folders} folder{}",
            plural(notes),
            plural(folders)
        ))
    })
    .await
}

/// The Photos report while Apple says the library is still being prepared; `None` when it is
/// ready — or Apple would not say, which leaves the read to go ahead as it always did.
pub async fn preparing(
    data: &LibraryLock,
    ready: quince_icloud::Result<quince_icloud::photos::Readiness>,
) -> Option<Report> {
    let Ok(quince_icloud::photos::Readiness::Preparing { .. }) = ready else {
        return None;
    };
    let last_synced_at = data
        .read(|library| library.last_synced(quince_icloud::photos::SERVICE))
        .await
        .ok()
        .flatten();
    Some(Report {
        last_synced_at,
        ..Report::of(PHOTOS, Outcome::Preparing, Some(PREPARING_WORDS.into()))
    })
}

/// Photos: the whole library through the change feed once, then only what changed — under
/// half a second when nothing did. A large library is read a few pages at a time, each stored
/// before the next is asked for: the walk holds no more of it in memory than those pages.
/// Then, on a pass that went well, Shared Albums and the Shared Library.
pub async fn photos(ctx: &Ctx<'_>, root: &str, data: &LibraryLock) -> Report {
    let mut changed = false;
    // What every round changed, put together; read whole once any round can't say.
    let mut items = Some(Items::default());
    let mut rounds = 0;
    loop {
        rounds += 1;
        let since = token(data, quince_icloud::photos::SERVICE).await;
        let full = since.is_none();
        // A whole read of a library Apple is still preparing finds it empty or part-made — and
        // a whole read takes what it never sees as gone. Asked first (as icloud.com asks), and
        // while it is preparing, not read.
        if full && rounds == 1 {
            let ready = quince_icloud::photos::readiness(&ctx.http, root, ctx.params.clone()).await;
            if let Some(report) = preparing(data, ready).await {
                ctx.hooks.preparing();
                break report;
            }
        }
        let fetched =
            quince_icloud::photos::fetch(&ctx.http, root, ctx.params.clone(), since.as_deref())
                .await;
        let more = fetched.as_ref().is_ok_and(|f| !f.complete);
        let mut report = store(PHOTOS, data, ctx, fetched, move |library, f| {
            // Stored, and a pass from the start marked (or every pass after walked the whole
            // library again).
            let (records, links) = crate::photos::apply(library, f, full)?;
            ctx.hooks.links(links);
            // Said from what the pass brought: counting the library to say it held the
            // writer's lock through each of a refresh's ten rounds (4.4 s each at a million).
            Ok(if records == 0 {
                "Up to date".to_string()
            } else {
                format!("{records} change{} from iCloud", plural(records))
            })
        })
        .await;
        changed |= report.changed;
        if report.changed {
            items = match (items, report.items.take()) {
                (Some(all), Some(round)) => joined(all, round),
                _ => None,
            };
        }
        let last_round = rounds >= quince_icloud::photos::PASS_ROUNDS;
        if report.outcome != Outcome::Ok || !more || last_round {
            report.changed = changed;
            report.items = if changed { items } else { None };
            report.more = report.outcome == Outcome::Ok && more;
            // Shared Albums after the library, on a pass that went well: what changed there
            // redraws Photos whole (it has no rows of its own in the library's change
            // tracking).
            if report.outcome == Outcome::Ok && shared_albums(ctx, root, data).await {
                report.changed = true;
                report.items = None;
            }
            // The Shared Library's photos are the library's own rows: what changed redraws
            // Photos whole, as the albums' do.
            if report.outcome == Outcome::Ok && shared_library(ctx, root, data).await {
                report.changed = true;
                report.items = None;
            }
            break report;
        }
    }
}

/// Two rounds' items as one: an item two rounds touched is counted once, as changed — its
/// screen reads it again either way. More than are worth placing, and it is read whole.
#[must_use]
pub fn joined(mut all: Items, round: Items) -> Option<Items> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for id in all.added.iter().chain(&all.changed).chain(&all.removed) {
        seen.insert(id.clone());
    }
    for (id, list) in round
        .added
        .into_iter()
        .map(|id| (id, 0))
        .chain(round.changed.into_iter().map(|id| (id, 1)))
        .chain(round.removed.into_iter().map(|id| (id, 2)))
    {
        if seen.contains(&id) {
            all.added.retain(|x| *x != id);
            all.removed.retain(|x| *x != id);
            if !all.changed.contains(&id) {
                all.changed.push(id);
            }
            continue;
        }
        seen.insert(id.clone());
        match list {
            0 => all.added.push(id),
            1 => all.changed.push(id),
            _ => all.removed.push(id),
        }
    }
    (all.len() <= MOST_ITEMS).then_some(all)
}

/// Shared Albums (iOS 27's shared collections): every album found, each read whole when its
/// change feed says it changed, and any no longer there let go of. Answers whether anything
/// was stored. Quiet on failure — the library's own pass speaks for Photos — and a failure
/// leaves every album as it was: nothing is let go of unless Apple's whole list was read.
pub async fn shared_albums(ctx: &Ctx<'_>, root: &str, data: &LibraryLock) -> bool {
    use quince_icloud::shared_collections::{changed, places, read};
    let found = match places(&ctx.http, root, ctx.params.clone()).await {
        Ok(found) => found,
        Err(e) => {
            crate::quiet!("shared albums: couldn't list them ({e})");
            return false;
        }
    };
    let mut reads = Vec::new();
    for place in &found {
        let token = data
            .read(|l| l.shared_album_token(&place.zone))
            .await
            .ok()
            .flatten();
        match changed(&ctx.http, root, ctx.params.clone(), place, token.as_deref()).await {
            Ok(false) => continue,
            Ok(true) => {}
            Err(e) => {
                crate::quiet!("shared albums: {} ({e})", place.zone);
                continue;
            }
        }
        match read(&ctx.http, root, ctx.params.clone(), place).await {
            Ok(r) => reads.push((place.clone(), r)),
            Err(e) => crate::quiet!("shared albums: couldn't read {} ({e})", place.zone),
        }
    }
    let mut library = data.lock_for_pass().await;
    // Read for an account that is no longer the library's: dropped, as `store` drops it.
    if !owned(&library, ctx.account.as_deref()) {
        return false;
    }
    let before = library.shared_albums().unwrap_or_default();
    let zones: Vec<&str> = found.iter().map(|p| p.zone.as_str()).collect();
    let stored = in_place(|| {
        library.atomically(|library| {
            for (place, r) in &reads {
                let album = quince_core::shared::SharedAlbum {
                    zone: place.zone.clone(),
                    database: if place.owned() { "private" } else { "shared" }.into(),
                    owner: place.owner.clone(),
                    title: r.about.title.clone(),
                    expires_at: r.about.expires_at,
                    count: 0,
                    cover: None,
                };
                library.put_shared_album(&album, &r.photos, r.token.as_deref())?;
            }
            library.keep_shared_albums(&zones)
        })
    });
    if let Err(e) = stored {
        crate::quiet!("shared albums: couldn't store them ({e})");
        return false;
    }
    let mut links = HashMap::new();
    for (_, r) in &mut reads {
        links.extend(std::mem::take(&mut r.links));
    }
    ctx.hooks.links(links);
    !reads.is_empty()
        || library
            .shared_albums()
            .map_or(true, |after| after.len() != before.len())
}

/// The Shared Libraries (family photo libraries) after the personal one: each read through its
/// own change feed, from its own place, a few pages per refresh as the personal library is.
/// Its photos join the library's own rows, marked with its zone. A library this account left
/// or that was ended goes with its photos — but only when Apple's list of them was read: a
/// list that failed lets nothing go. Answers whether anything changed.
pub async fn shared_library(ctx: &Ctx<'_>, root: &str, data: &LibraryLock) -> bool {
    use quince_icloud::photos::{fetch_in, service_of, shared_libraries, PASS_ROUNDS};
    let found = match shared_libraries(&ctx.http, root, ctx.params.clone()).await {
        Ok(found) => found,
        Err(e) => {
            crate::quiet!("shared library: couldn't list them ({e})");
            return false;
        }
    };
    let account = ctx.account.as_deref();
    let mut changed = false;
    {
        let mut library = data.lock_for_pass().await;
        if !owned(&library, account) {
            return false;
        }
        let before = library.shared_libraries().unwrap_or_default().len();
        let zones: Vec<&str> = found.iter().map(|p| p.zone.as_str()).collect();
        let kept = in_place(|| {
            library.atomically(|library| {
                for place in &found {
                    library.put_shared_library(&quince_core::shared::SharedLibrary {
                        zone: place.zone.clone(),
                        database: if place.owned() { "private" } else { "shared" }.into(),
                        owner: place.owner.clone(),
                        title: None,
                    })?;
                }
                library.keep_shared_libraries(&zones)
            })
        });
        if let Err(e) = kept {
            crate::quiet!("shared library: couldn't store the list ({e})");
            return false;
        }
        changed |= before != found.len();
    }
    for place in &found {
        let service = service_of(Some(&place.zone));
        for _ in 0..PASS_ROUNDS {
            let since = data.read(|l| l.sync_token(&service)).await.ok().flatten();
            let fetched = match fetch_in(
                &ctx.http,
                root,
                ctx.params.clone(),
                Some(place),
                since.as_deref(),
            )
            .await
            {
                Ok(f) => f,
                Err(e) => {
                    crate::quiet!("shared library: couldn't read {} ({e})", place.zone);
                    break;
                }
            };
            let more = !fetched.complete;
            let brought = fetched.records.len();
            let mut library = data.lock_for_pass().await;
            if !owned(&library, account) {
                return changed;
            }
            // The flag changes still on their way laid back over it, as over the personal library.
            match in_place(|| crate::photos::apply_shared_library(&mut library, fetched)) {
                Ok((n, links)) => {
                    changed |= n > 0;
                    ctx.hooks.links(links);
                }
                Err(e) => {
                    crate::quiet!("shared library: couldn't store {} ({e})", place.zone);
                    break;
                }
            }
            if !more || brought == 0 {
                break;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_icloud::Error;

    const DSID: &str = "1234567890";

    fn params(dsid: &str) -> Vec<(String, String)> {
        vec![("dsid".to_string(), dsid.to_string())]
    }

    /// A library owned by the account `dsid` signs in as.
    fn library_of(dsid: &str) -> LibraryLock {
        let mut library = Library::in_memory().unwrap();
        library
            .set_state(crate::queue::ACCOUNT_KEY, &crate::queue::account_tag(dsid))
            .unwrap();
        LibraryLock::new(library, None)
    }

    fn ctx<'a>(dsid: &str, hooks: &'a dyn Hooks) -> Ctx<'a> {
        Ctx::new(
            reqwest::Client::new(),
            params(dsid),
            Some("http://127.0.0.1:9".into()),
            Vec::new(),
            hooks,
        )
    }

    /// The account a pass stores for is the one its own session parameters ask as.
    #[test]
    fn the_account_comes_with_the_session_it_asks_as() {
        let c = ctx(DSID, &NoHooks);
        assert_eq!(c.account, Some(crate::queue::account_tag(DSID)));
        let none = Ctx::new(
            reqwest::Client::new(),
            Vec::new(),
            None,
            Vec::new(),
            &NoHooks,
        );
        assert_eq!(none.account, None);
    }

    /// Once a day a service walks everything again: its token is held back after the
    /// interval, so the next fetch starts from the beginning; within it the token is used.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_full_walk_is_chosen_once_the_day_has_passed() {
        let data = library_of(DSID);
        assert_eq!(
            token(&data, NOTES).await,
            None,
            "never synced: walked whole"
        );
        {
            let l = data.lock().await;
            l.mark_synced(NOTES, Some("token-1")).unwrap();
            l.mark_full_sync(NOTES).unwrap();
        }
        assert_eq!(token(&data, NOTES).await.as_deref(), Some("token-1"));
        {
            let l = data.lock().await;
            l.conn()
                .execute(
                    "UPDATE sync_state SET last_full_at = last_full_at - ?1 WHERE service = ?2",
                    rusqlite_params(FULL_SYNC_EVERY + 1),
                )
                .unwrap();
        }
        assert_eq!(
            token(&data, NOTES).await,
            None,
            "a day after the last whole walk, the next pass is one"
        );
        {
            let l = data.lock().await;
            l.conn()
                .execute(
                    "UPDATE sync_state SET last_full_at = last_full_at + ?1 WHERE service = ?2",
                    rusqlite_params(FULL_SYNC_EVERY - 60),
                )
                .unwrap();
        }
        assert_eq!(
            token(&data, NOTES).await.as_deref(),
            Some("token-1"),
            "a minute short of the day: still a delta"
        );
    }

    fn rusqlite_params(seconds: i64) -> (i64, &'static str) {
        (seconds, NOTES)
    }

    /// A pass read under one account's session, landing after another account took the
    /// library: refused, and nothing written — not the records, not even a failure.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pass_for_another_account_writes_nothing() {
        let data = library_of("someone-else");
        let hooks = NoHooks;
        let c = ctx(DSID, &hooks);
        let ran = std::sync::atomic::AtomicBool::new(false);
        let report = store(NOTES, &data, &c, Ok(()), |library, ()| {
            ran.store(true, std::sync::atomic::Ordering::SeqCst);
            library.mark_synced(NOTES, Some("theirs"))?;
            Ok("stored".into())
        })
        .await;
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.detail.as_deref(), Some(NOT_YOURS_WORDS));
        assert!(
            !ran.load(std::sync::atomic::Ordering::SeqCst),
            "never applied"
        );
        let l = data.lock().await;
        assert_eq!(l.sync_token(NOTES).unwrap(), None, "nothing stored");
        assert!(
            l.sync_health()
                .unwrap()
                .iter()
                .all(|h| h.last_error.is_none()),
            "and no failure written into their library"
        );
        drop(l);

        // A library naming nobody is nobody's: refused too.
        let unowned = LibraryLock::new(Library::in_memory().unwrap(), None);
        let report = store(NOTES, &unowned, &c, Ok(()), |_, ()| Ok("stored".into())).await;
        assert_eq!(report.outcome, Outcome::Failed);

        // And this account's own: stored.
        let ours = library_of(DSID);
        let report = store(NOTES, &ours, &c, Ok(()), |library, ()| {
            library.mark_synced(NOTES, Some("ours"))?;
            Ok("stored".into())
        })
        .await;
        assert_eq!(report.outcome, Outcome::Ok);
        assert_eq!(
            ours.lock().await.sync_token(NOTES).unwrap().as_deref(),
            Some("ours")
        );
    }

    /// A failure from Apple is recorded against the service, where the screens read it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failure_is_recorded_for_the_service() {
        let data = library_of(DSID);
        let c = ctx(DSID, &NoHooks);
        let report = store::<(), _>(
            NOTES,
            &data,
            &c,
            Err(Error::Busy),
            |_, ()| Ok(String::new()),
        )
        .await;
        assert_eq!(report.outcome, Outcome::Failed);
        let health = data.lock().await.sync_health().unwrap();
        let notes = health.iter().find(|h| h.service == NOTES).unwrap();
        assert!(notes.last_error.is_some());
    }

    /// A pass that panics ends as failed, its pass lock let go, the failure recorded; the
    /// next pass runs as usual.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pass_that_panics_fails_alone_and_the_next_runs() {
        let lock = tokio::sync::Mutex::new(());
        let report = guarded(REMINDERS, async {
            let _held = lock.lock().await;
            panic!("invalid time");
        })
        .await;
        assert_eq!(report.outcome, Outcome::Failed);
        assert_eq!(report.label, "Reminders");
        assert!(report.detail.unwrap().contains("invalid time"));
        assert!(lock.try_lock().is_ok(), "the pass lock is let go");

        let next = guarded(REMINDERS, async { unavailable(REMINDERS) }).await;
        assert_eq!(next.outcome, Outcome::Unavailable, "the next pass ran");
    }

    /// The same as both platforms drive it: the panic is recorded against the service, in
    /// this account's library, the pass lock is let go, and the next pass runs and stores.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_driven_pass_that_panics_is_recorded_and_the_next_runs() {
        let data = library_of(DSID);
        let c = ctx(DSID, &NoHooks);
        let lock = tokio::sync::Mutex::new(());
        let report = driven(NOTES, &c, &data, &lock, async {
            panic!("a record it didn't expect");
        })
        .await;
        assert_eq!(report.outcome, Outcome::Failed);
        let health = data.lock().await.sync_health().unwrap();
        let notes = health.iter().find(|h| h.service == NOTES).unwrap();
        assert!(notes
            .last_error
            .as_deref()
            .unwrap()
            .contains("a record it didn't expect"));
        assert!(lock.try_lock().is_ok(), "the pass lock is let go");

        let next = driven(NOTES, &c, &data, &lock, async {
            store(NOTES, &data, &c, Ok(()), |library, ()| {
                library.mark_synced(NOTES, Some("next"))?;
                Ok("stored".into())
            })
            .await
        })
        .await;
        assert_eq!(next.outcome, Outcome::Ok, "the next pass ran and stored");

        // Another account's library: its health is left alone.
        let theirs = library_of("someone-else");
        driven(NOTES, &c, &theirs, &lock, async { panic!("boom") }).await;
        assert!(theirs
            .lock()
            .await
            .sync_health()
            .unwrap()
            .iter()
            .all(|h| h.last_error.is_none()));
    }

    /// Advanced Data Protection keeping a service out: the pass asks Apple nothing while it
    /// waits on an approval, and a refusal met anyway pauses it (never a failure to retry) and
    /// says so once, so the approval is asked for again rather than the pass in a loop.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_service_held_by_data_protection_is_not_retried() {
        #[derive(Default)]
        struct Told(std::sync::Mutex<Vec<&'static str>>);
        impl Hooks for Told {
            fn refused(&self, service: &'static str) {
                self.0.lock().unwrap().push(service);
            }
        }
        let server = wiremock::MockServer::start().await;
        let data = library_of(DSID);
        let told = Told::default();
        let mut c = Ctx::new(
            reqwest::Client::new(),
            params(DSID),
            Some(server.uri()),
            vec![NOTES, REMINDERS, PHOTOS],
            &told,
        );
        let lock = tokio::sync::Mutex::new(());
        for _ in 0..5 {
            for service in [NOTES, REMINDERS, PHOTOS] {
                let report = run(service, &c, &data, &lock).await;
                assert_eq!(report.outcome, Outcome::Paused);
                assert_eq!(report.detail.as_deref(), Some(PAUSED_WORDS));
            }
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "nothing asked of Apple while it waits on an approval"
        );

        // Approved, and then Apple lets the key go early: refused once, paused, told.
        c.waiting.clear();
        let report = store::<(), _>(
            NOTES,
            &data,
            &c,
            Err(Error::NeedsDeviceApproval),
            |_, ()| Ok(String::new()),
        )
        .await;
        assert_eq!(report.outcome, Outcome::Paused);
        assert_eq!(*told.0.lock().unwrap(), vec![NOTES]);
        let health = data.lock().await.sync_health().unwrap();
        assert!(
            health.iter().all(|h| h.last_error.is_none()),
            "waiting is not a failure"
        );
    }

    /// Not offered (no CloudKit for this account), or a service this does not run: said as
    /// that, and nothing asked.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_service_not_offered_is_unavailable() {
        let data = library_of(DSID);
        let c = Ctx::new(
            reqwest::Client::new(),
            params(DSID),
            None,
            Vec::new(),
            &NoHooks,
        );
        let lock = tokio::sync::Mutex::new(());
        assert_eq!(
            run(NOTES, &c, &data, &lock).await.outcome,
            Outcome::Unavailable
        );
        let c = ctx(DSID, &NoHooks);
        assert_eq!(
            run("drive", &c, &data, &lock).await.outcome,
            Outcome::Unavailable
        );
    }

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
    }

    #[test]
    fn waiting_reads_as_waiting_not_failure() {
        assert_eq!(serde_json::to_value(Outcome::Paused).unwrap(), "paused");
        assert!(!PAUSED_WORDS.to_lowercase().contains("error"));
    }
}
