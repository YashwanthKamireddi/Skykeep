//! Skykeep in the desktop's own search: typing in GNOME's Activities overview or KDE's KRunner
//! shows what the library holds — notes, reminders, events, contacts, files, photos — and
//! choosing one opens its app on it.
//!
//! One well-known name on the session bus, two objects behind it:
//! - `org.gnome.Shell.SearchProvider2` at [`GNOME_PATH`], found by GNOME Shell through
//!   `packaging/<id>.search-provider.ini`;
//! - `org.kde.krunner1` at [`KRUNNER_PATH`], found by KRunner through
//!   `packaging/<id>.krunner.desktop` in `/usr/share/krunner/dbusplugins/`.
//!
//! Neither desktop needs Skykeep already running: `packaging/<id>.SearchProvider.service` lets
//! the bus start it with no window (`--search-provider`) on the first search; started so, it
//! ends after a few idle minutes unless a window is opened.
//!
//! The answer has to be quick — the shell waits on every provider as the person types — so a
//! search reads the library only if its lock comes free within [`LOCK_WAIT`] (a sync pass may be
//! holding it), and gives up on the whole after [`GIVE_UP`]: nothing found beats a frozen
//! overview.

use quince_core::search::Everything;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use zbus::zvariant::Value;

/// The name both desktops call; the same in the `.ini`, the `.service` and the KRunner entry.
pub const BUS_NAME: &str = "io.github.yashwanthkamireddi.Skykeep.SearchProvider";
/// GNOME Shell's object.
pub const GNOME_PATH: &str = "/io/github/yashwanthkamireddi/Skykeep/SearchProvider";
/// KRunner's object.
pub const KRUNNER_PATH: &str = "/io/github/yashwanthkamireddi/Skykeep/KRunner";
/// The program's own icon, installed with every package.
const MAIN_ICON: &str = "io.github.yashwanthkamireddi.Skykeep";
/// At most this many of each app.
const EACH: usize = 5;
/// How long a search waits for the library's lock.
const LOCK_WAIT: Duration = Duration::from_millis(30);
/// How long a search may take in all before it answers with nothing.
const GIVE_UP: Duration = Duration::from_millis(250);
/// The most results remembered for GetResultMetas and Run to describe.
const REMEMBERED: usize = 600;

/// The apps, in the sidebar's order: the key a window and a reveal use, and the name shown.
const APPS: [(&str, &str); 6] = [
    ("notes", "Notes"),
    ("reminders", "Reminders"),
    ("calendar", "Calendar"),
    ("contacts", "Contacts"),
    ("drivews", "Drive"),
    ("photos", "Photos"),
];

/// One thing found, as the desktop shows it and as its app is asked to reveal it.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The app: a key of [`APPS`].
    pub screen: &'static str,
    pub id: String,
    /// Where it lives, when its app needs that to get there: a reminder's list, an event's
    /// calendar, a file's folder.
    pub within: Option<String>,
    /// When, for an event: the day the calendar turns to.
    pub at: Option<i64>,
    pub title: String,
    /// The app's name, and a line of what it holds.
    pub description: String,
}

impl Hit {
    /// The identifier the desktop hands back: the app and the item, so even an identifier from
    /// before a restart (nothing remembered) still opens the right thing.
    pub fn result_id(&self) -> String {
        format!("{}/{}", self.screen, self.id)
    }

    /// The app's name.
    pub fn app_name(&self) -> &'static str {
        APPS.iter()
            .find(|(k, _)| *k == self.screen)
            .map_or("Skykeep", |(_, n)| *n)
    }

    /// The app's own icon name (`<id>.Notes`), as its launcher entry and window are named.
    pub fn app_icon(&self) -> String {
        format!("{MAIN_ICON}.{}", self.app_name())
    }

    /// GNOME's `gicon`: a serialized GThemedIcon naming the app's own icon first and the
    /// program's after it, so a theme without per-app icons still shows Skykeep's.
    pub fn gicon(&self) -> String {
        format!(". GThemedIcon {} {MAIN_ICON}", self.app_icon())
    }
}

/// What an app window is asked to show: `src/lib/reveal.ts`'s `Reveal`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reveal {
    pub screen: String,
    pub id: String,
    pub within: Option<String>,
    pub at: Option<i64>,
    pub query: String,
}

/// The day, as a person reads it.
fn day(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%-d %b %Y")
                .to_string()
        })
        .unwrap_or_default()
}

/// The day and time.
fn moment(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|t| {
            t.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%-d %b %Y, %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

/// One line of text: attachment marks gone, runs of space and line breaks one space, cut short.
fn line(text: &str) -> String {
    let flat = text
        .replace('\u{FFFC}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    const MOST: usize = 90;
    if flat.chars().count() <= MOST {
        flat
    } else {
        let cut: String = flat.chars().take(MOST - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// "Notes · the snippet", or the app's name alone.
fn described(app: &str, snippet: &str) -> String {
    let snippet = line(snippet);
    if snippet.is_empty() {
        app.to_string()
    } else {
        format!("{app} · {snippet}")
    }
}

/// What a search found, as the desktop's results: each app's few in the sidebar's order.
pub fn hits(found: Everything, each: usize) -> Vec<Hit> {
    let mut out = Vec::new();
    for n in found.notes.into_iter().take(each) {
        let snippet = if n.locked {
            "Locked".to_string()
        } else {
            n.snippet.unwrap_or_default()
        };
        out.push(Hit {
            screen: "notes",
            title: if n.title.trim().is_empty() {
                "New Note".into()
            } else {
                line(&n.title)
            },
            description: described("Notes", &snippet),
            id: n.guid,
            within: None,
            at: None,
        });
    }
    for r in found.reminders.into_iter().take(each) {
        let snippet = match r.due_at {
            Some(due) => format!("Due {}", moment(due)),
            None => r.notes.unwrap_or_default(),
        };
        out.push(Hit {
            screen: "reminders",
            title: line(&r.title),
            description: described("Reminders", &snippet),
            id: r.guid,
            within: Some(r.list_guid),
            at: None,
        });
    }
    for e in found.events.into_iter().take(each) {
        let when = if e.all_day {
            day(e.starts_at)
        } else {
            moment(e.starts_at)
        };
        let snippet = match e.location.as_deref().map(str::trim) {
            Some(place) if !place.is_empty() => format!("{when} · {place}"),
            _ => when,
        };
        out.push(Hit {
            screen: "calendar",
            title: line(&e.title),
            description: described("Calendar", &snippet),
            id: e.guid,
            within: Some(e.calendar_guid),
            at: Some(e.starts_at),
        });
    }
    for c in found.contacts.into_iter().take(each) {
        let snippet = c
            .methods
            .first()
            .map(|m| m.value.clone())
            .or_else(|| c.company.clone())
            .unwrap_or_default();
        out.push(Hit {
            screen: "contacts",
            title: line(&c.display_name()),
            description: described("Contacts", &snippet),
            id: c.guid,
            within: None,
            at: None,
        });
    }
    for f in found.files.into_iter().take(each) {
        let snippet = f
            .modified_at
            .map(|m| format!("Modified {}", day(m)))
            .unwrap_or_default();
        out.push(Hit {
            screen: "drivews",
            title: line(&f.display_name()),
            description: described("Drive", &snippet),
            id: f.drivewsid,
            within: f.parent_id,
            at: None,
        });
    }
    for p in found.photos.into_iter().take(each) {
        let title = p
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .or_else(|| p.filename.clone())
            .unwrap_or_else(|| "Photo".into());
        out.push(Hit {
            screen: "photos",
            title: line(&title),
            description: described("Photos", &day(p.taken_at)),
            id: p.id,
            within: None,
            at: None,
        });
    }
    out
}

/// GNOME's description of one result.
pub fn gnome_meta(hit: &Hit) -> HashMap<String, Value<'static>> {
    HashMap::from([
        ("id".to_string(), Value::from(hit.result_id())),
        ("name".to_string(), Value::from(hit.title.clone())),
        (
            "description".to_string(),
            Value::from(hit.description.clone()),
        ),
        ("gicon".to_string(), Value::from(hit.gicon())),
    ])
}

/// One KRunner match: (id, text, icon, type, relevance, properties).
pub type KMatch = (
    String,
    String,
    String,
    i32,
    f64,
    HashMap<String, Value<'static>>,
);

/// KRunner's description of the `rank`th result: a title the same as the query is an exact
/// match, anything else a possible one, earlier results a little more relevant.
pub fn krunner_match(hit: &Hit, query: &str, rank: usize) -> KMatch {
    // KRunner's match types (Plasma's QueryMatch::Type).
    const POSSIBLE: i32 = 30;
    const EXACT: i32 = 100;
    let exact = hit.title.trim().to_lowercase() == query.trim().to_lowercase();
    let relevance = (if exact { 1.0 } else { 0.7 }) - (rank as f64) * 0.001;
    // The icon: the program's own, which every package installs. KRunner takes a single name
    // with no fallback, and the per-app names are not installed as icons.
    let properties = HashMap::from([
        ("subtext".to_string(), Value::from(hit.description.clone())),
        (
            "category".to_string(),
            Value::from(hit.app_name().to_string()),
        ),
    ]);
    (
        hit.result_id(),
        hit.title.clone(),
        MAIN_ICON.to_string(),
        if exact { EXACT } else { POSSIBLE },
        relevance,
        properties,
    )
}

/// Searches the library: `None` when it could not be read in time.
type Search = dyn Fn(&str) -> Option<Everything> + Send + Sync;
/// Opens an app on one thing.
type Open = dyn Fn(Reveal) + Send + Sync;
/// Opens the library window to search for more.
type More = dyn Fn(String) + Send + Sync;

/// What both objects share: how to search, how to open, and what was last found.
pub struct Core {
    search: Box<Search>,
    open: Box<Open>,
    more: Box<More>,
    seen: Mutex<HashMap<String, Hit>>,
}

impl Core {
    pub fn new(
        search: impl Fn(&str) -> Option<Everything> + Send + Sync + 'static,
        open: impl Fn(Reveal) + Send + Sync + 'static,
        more: impl Fn(String) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            search: Box::new(search),
            open: Box::new(open),
            more: Box::new(more),
            seen: Mutex::new(HashMap::new()),
        })
    }

    /// What the library holds for `query`, remembered so the desktop can ask after it by id.
    fn find(&self, query: &str) -> Vec<Hit> {
        asked();
        let found = (self.search)(query).map_or_else(Vec::new, |f| hits(f, EACH));
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        if seen.len() + found.len() > REMEMBERED {
            seen.clear();
        }
        for hit in &found {
            seen.insert(hit.result_id(), hit.clone());
        }
        found
    }

    /// A result found earlier, by its id.
    fn recall(&self, id: &str) -> Option<Hit> {
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Opens a result's app on it. An id not remembered (asked after a restart) still names
    /// its app and item; only where it lives is unknown, which the app works out.
    fn activate(&self, id: &str, query: String) {
        let reveal = match self.recall(id) {
            Some(hit) => Reveal {
                screen: hit.screen.to_string(),
                id: hit.id,
                within: hit.within,
                at: hit.at,
                query,
            },
            None => {
                let Some((screen, item)) = id.split_once('/') else {
                    return;
                };
                if !APPS.iter().any(|(k, _)| *k == screen) {
                    return;
                }
                Reveal {
                    screen: screen.to_string(),
                    id: item.to_string(),
                    within: None,
                    at: None,
                    query,
                }
            }
        };
        (self.open)(reveal);
    }
}

/// Runs a search off the bus's own thread, and answers with nothing past [`GIVE_UP`].
async fn search_off_thread(core: &Arc<Core>, query: String) -> Vec<Hit> {
    let core = Arc::clone(core);
    // On Tauri's runtime: the bus's executor has no timer of its own to give up by.
    tauri::async_runtime::spawn(async move {
        let work = tauri::async_runtime::spawn_blocking(move || core.find(&query));
        tokio::time::timeout(GIVE_UP, work)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

/// The terms the shell sends, as one query.
fn joined(terms: &[String]) -> String {
    terms.join(" ")
}

/// GNOME Shell's search provider.
pub struct Gnome(pub Arc<Core>);

#[zbus::interface(name = "org.gnome.Shell.SearchProvider2")]
impl Gnome {
    async fn get_initial_result_set(&self, terms: Vec<String>) -> Vec<String> {
        let found = search_off_thread(&self.0, joined(&terms)).await;
        found.iter().map(Hit::result_id).collect()
    }

    /// The narrower search as the person types on: searched again (it is quick), which also
    /// finds what the first search's cut-off left out.
    async fn get_subsearch_result_set(
        &self,
        _previous_results: Vec<String>,
        terms: Vec<String>,
    ) -> Vec<String> {
        let found = search_off_thread(&self.0, joined(&terms)).await;
        found.iter().map(Hit::result_id).collect()
    }

    async fn get_result_metas(
        &self,
        identifiers: Vec<String>,
    ) -> Vec<HashMap<String, Value<'static>>> {
        identifiers
            .iter()
            .filter_map(|id| self.0.recall(id))
            .map(|hit| gnome_meta(&hit))
            .collect()
    }

    async fn activate_result(&self, identifier: String, terms: Vec<String>, _timestamp: u32) {
        self.0.activate(&identifier, joined(&terms));
    }

    async fn launch_search(&self, terms: Vec<String>, _timestamp: u32) {
        (self.0.more)(joined(&terms));
    }
}

/// KDE's KRunner plugin.
pub struct KRunner(pub Arc<Core>);

#[zbus::interface(name = "org.kde.krunner1")]
impl KRunner {
    /// No actions beyond opening a match.
    async fn actions(&self) -> Vec<(String, String, String)> {
        Vec::new()
    }

    #[zbus(name = "Match")]
    async fn find(&self, query: String) -> Vec<KMatch> {
        let found = search_off_thread(&self.0, query.clone()).await;
        found
            .iter()
            .enumerate()
            .map(|(rank, hit)| krunner_match(hit, &query, rank))
            .collect()
    }

    async fn run(&self, match_id: String, _action_id: String) {
        self.0.activate(&match_id, String::new());
    }

    async fn config(&self) -> HashMap<String, Value<'static>> {
        HashMap::new()
    }

    async fn teardown(&self) {}
}

/// Takes the bus name and serves both objects on `connection`'s bus.
pub async fn serve_on(
    builder: zbus::connection::Builder<'_>,
    name: &str,
    core: Arc<Core>,
) -> zbus::Result<zbus::Connection> {
    builder
        .name(name.to_string())?
        .serve_at(GNOME_PATH, Gnome(Arc::clone(&core)))?
        .serve_at(KRUNNER_PATH, KRunner(core))?
        .build()
        .await
}

/// The connection, held for as long as Skykeep runs.
static BUS: OnceLock<zbus::Connection> = OnceLock::new();

/// The library read for a search, if its lock comes free in time: never waits on a sync pass.
fn search_library(app: &AppHandle, query: &str) -> Option<Everything> {
    let data = app.try_state::<crate::Data>()?;
    let until = Instant::now() + LOCK_WAIT;
    let library = loop {
        if let Ok(library) = data.0.try_lock() {
            break library;
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    library.search_everything(query, EACH).ok()
}

/// A result opened in its app's own window, showing it: the one hand-off every window uses
/// (apps::reveal).
fn open_app(app: &AppHandle, reveal: Reveal) {
    match serde_json::to_value(&reveal) {
        Ok(asked) => crate::apps::reveal(app, &reveal.screen, &asked),
        Err(_) => crate::apps::open(app, &reveal.screen),
    }
}

/// "Show all results" from the desktop's search: Skykeep's apps are windows of their own, with
/// no search across all six, so the result the shell listed first opens in its app; nothing
/// found, the app used last opens. Off the async threads: the library is waited on briefly.
fn search_in_window(app: &AppHandle, query: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        let first =
            search_library(&app, &query).and_then(|found| hits(found, 1).into_iter().next());
        match first {
            Some(hit) => open_app(
                &app,
                Reveal {
                    screen: hit.screen.to_string(),
                    id: hit.id,
                    within: hit.within,
                    at: hit.at,
                    query,
                },
            ),
            None => crate::apps::open_skykeep(&app),
        }
    });
}

/// When a search was last asked, in seconds since the epoch.
static LAST_ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn asked() {
    LAST_ASKED.store(now_seconds(), std::sync::atomic::Ordering::Relaxed);
}

/// Started for a search (`--search-provider`): ends once [`crate::login::SEARCH_IDLE`] passes
/// with no window shown and no search asked. The changes waiting for Apple are in the library,
/// and go with the next start.
pub fn end_when_idle(app: AppHandle) {
    asked();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(30));
        let idle = Duration::from_secs(
            now_seconds().saturating_sub(LAST_ASKED.load(std::sync::atomic::Ordering::Relaxed)),
        );
        if crate::login::was_shown() {
            return;
        }
        if crate::login::search_done(false, idle) {
            // Ends the process from a thread of its own; nothing is left for this one to do.
            crate::shutdown(&app, 0);
            return;
        }
    });
}

/// Takes Skykeep's place in the desktop's search, for as long as it runs. Nothing is lost if
/// there is no session bus or the name is taken: the desktop's search just goes without.
pub fn serve(app: AppHandle) {
    let searching = app.clone();
    let opening = app.clone();
    let core = Core::new(
        move |query| search_library(&searching, query),
        move |reveal| open_app(&opening, reveal),
        move |query| search_in_window(&app, query),
    );
    tauri::async_runtime::spawn(async move {
        let built = match zbus::connection::Builder::session() {
            Ok(builder) => serve_on(builder, BUS_NAME, core).await,
            Err(e) => Err(e),
        };
        match built {
            Ok(connection) => {
                BUS.set(connection).ok();
            }
            Err(e) => quiet!("not in the desktop's search: {e}"),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::Library;

    fn note(guid: &str, title: &str, snippet: &str) -> quince_core::notes::Note {
        quince_core::notes::Note {
            guid: guid.into(),
            folder_guid: None,
            title: title.into(),
            snippet: Some(snippet.into()),
            body: Some(snippet.into()),
            locked: false,
            pinned: false,
            created_at: Some(1),
            modified_at: Some(1),
            editable: true,
            etag: None,
            runs: None,
        }
    }

    fn event(guid: &str, title: &str) -> quince_core::calendar::Event {
        quince_core::calendar::Event {
            guid: guid.into(),
            calendar_guid: "CAL".into(),
            title: title.into(),
            location: Some("  Room 4 ".into()),
            starts_at: 1_767_225_600,
            ends_at: 1_767_229_200,
            all_day: false,
            timezone: None,
            url: None,
            notes: None,
            etag: None,
        }
    }

    /// Each app's results: its title, its app's name with a line of what it holds, the app's
    /// icon, and what its window needs to find it.
    #[test]
    fn results_say_what_and_where() {
        let mut lots = Vec::new();
        for i in 0..8 {
            lots.push(note(&format!("N{i}"), "", "first\nline \u{FFFC} of it"));
        }
        let found = Everything {
            notes: lots,
            events: vec![event("E1", "Standup")],
            ..Everything::default()
        };
        let all = hits(found, EACH);
        assert_eq!(all.len(), EACH + 1, "five notes at most, then the event");
        let n = &all[0];
        assert_eq!(n.title, "New Note", "an untitled note says so");
        assert_eq!(n.description, "Notes · first line of it");
        assert_eq!(n.result_id(), "notes/N0");
        assert_eq!(
            n.gicon(),
            ". GThemedIcon io.github.yashwanthkamireddi.Skykeep.Notes io.github.yashwanthkamireddi.Skykeep"
        );
        let e = all.last().unwrap();
        assert_eq!(e.screen, "calendar");
        assert_eq!(e.within.as_deref(), Some("CAL"));
        assert_eq!(e.at, Some(1_767_225_600));
        assert!(e.description.starts_with("Calendar · "));
        assert!(e.description.ends_with(" · Room 4"), "{}", e.description);
        assert_eq!(
            e.app_icon(),
            "io.github.yashwanthkamireddi.Skykeep.Calendar"
        );
    }

    #[test]
    fn a_long_snippet_is_one_short_line() {
        let long = "word ".repeat(100);
        let d = described("Notes", &long);
        assert!(d.chars().count() <= "Notes · ".chars().count() + 90);
        assert!(d.ends_with('…'));
        assert_eq!(described("Drive", "   "), "Drive");
    }

    #[test]
    fn gnome_and_krunner_describe_the_same_result() {
        let hit = hits(
            Everything {
                notes: vec![note("N1", "Groceries", "milk")],
                ..Everything::default()
            },
            EACH,
        )
        .remove(0);
        let meta = gnome_meta(&hit);
        assert_eq!(meta["id"], Value::from("notes/N1"));
        assert_eq!(meta["name"], Value::from("Groceries"));
        assert_eq!(meta["description"], Value::from("Notes · milk"));
        let (id, text, icon, kind, relevance, props) = krunner_match(&hit, "groceries", 0);
        assert_eq!((id.as_str(), text.as_str()), ("notes/N1", "Groceries"));
        assert_eq!(icon, MAIN_ICON);
        assert_eq!(kind, 100, "the title itself: an exact match");
        assert!(relevance > krunner_match(&hit, "milk", 0).4);
        assert_eq!(krunner_match(&hit, "milk", 0).3, 30);
        assert_eq!(props["category"], Value::from("Notes"));
        assert_eq!(props["subtext"], Value::from("Notes · milk"));
    }

    /// An id from before a restart still opens its app on it; one naming no app opens nothing.
    #[test]
    fn an_unremembered_id_still_opens_its_app() {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let o = Arc::clone(&opened);
        let core = Core::new(|_| None, move |r| o.lock().unwrap().push(r), |_| {});
        core.activate("reminders/R9", "milk".into());
        core.activate("nonsense/X", String::new());
        core.activate("no-slash", String::new());
        let opened = opened.lock().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(opened[0].screen, "reminders");
        assert_eq!(opened[0].id, "R9");
        assert_eq!(opened[0].query, "milk");
        // The shape src/lib/reveal.ts's Reveal takes.
        assert_eq!(
            serde_json::to_value(&opened[0]).unwrap(),
            serde_json::json!({"screen":"reminders","id":"R9","within":null,"at":null,"query":"milk"})
        );
    }

    /// Whether this test runs on a bus of its own (`dbus-run-session -- cargo test …`), never
    /// the person's: their session bus is the one at `$XDG_RUNTIME_DIR/bus`.
    fn private_bus() -> bool {
        let Ok(address) = std::env::var("DBUS_SESSION_BUS_ADDRESS") else {
            return false;
        };
        let own = std::env::var("XDG_RUNTIME_DIR")
            .map(|d| format!("unix:path={d}/bus"))
            .unwrap_or_default();
        !own.is_empty() && !address.starts_with(&own)
    }

    /// The served objects on a private bus, called as GNOME Shell and KRunner call them, over a
    /// library on disk: run under `dbus-run-session -- cargo test -p quince-app search_provider`.
    #[test]
    fn served_on_a_private_bus() {
        if !private_bus() {
            eprintln!("skipped: run under dbus-run-session for a private bus");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let mut library = Library::open(dir.path()).unwrap();
        library
            .put_notes(&[
                note("N1", "Springfield trip", "pack the tent"),
                note("N2", "Budget", "nothing here"),
            ])
            .unwrap();
        library
            .put_calendars(&[quince_core::calendar::Calendar {
                guid: "CAL".into(),
                title: "Home".into(),
                colour: None,
                ctag: None,
                sort_order: 0,
                enabled: true,
                writable: true,
                is_default: true,
            }])
            .unwrap();
        library
            .put_events(&[event("E1", "Springfield fair")])
            .unwrap();
        let library = Arc::new(Mutex::new(library));
        let opened = Arc::new(Mutex::new(Vec::new()));
        let more = Arc::new(Mutex::new(Vec::new()));
        let (lib, o, m) = (Arc::clone(&library), Arc::clone(&opened), Arc::clone(&more));
        let core = Core::new(
            move |q| lib.lock().unwrap().search_everything(q, EACH).ok(),
            move |r| o.lock().unwrap().push(r),
            move |q| m.lock().unwrap().push(q),
        );
        tauri::async_runtime::block_on(async move {
            let name = "io.github.yashwanthkamireddi.Skykeep.SearchProviderTest";
            let _served = serve_on(zbus::connection::Builder::session().unwrap(), name, core)
                .await
                .unwrap();
            let client = zbus::Connection::session().await.unwrap();
            let gnome =
                zbus::Proxy::new(&client, name, GNOME_PATH, "org.gnome.Shell.SearchProvider2")
                    .await
                    .unwrap();
            let started = Instant::now();
            let ids: Vec<String> = gnome
                .call("GetInitialResultSet", &(vec!["springfield"],))
                .await
                .unwrap();
            let took = started.elapsed();
            assert_eq!(ids, vec!["notes/N1", "calendar/E1"]);
            assert!(took < Duration::from_millis(50), "took {took:?}");
            let narrower: Vec<String> = gnome
                .call(
                    "GetSubsearchResultSet",
                    &(ids.clone(), vec!["springfield", "tent"]),
                )
                .await
                .unwrap();
            assert_eq!(narrower, vec!["notes/N1"]);
            let metas: Vec<HashMap<String, zbus::zvariant::OwnedValue>> =
                gnome.call("GetResultMetas", &(ids.clone(),)).await.unwrap();
            assert_eq!(metas.len(), 2);
            let text = |m: &HashMap<String, zbus::zvariant::OwnedValue>, k: &str| {
                String::try_from(m[k].try_clone().unwrap()).unwrap()
            };
            assert_eq!(text(&metas[0], "id"), "notes/N1");
            assert_eq!(text(&metas[0], "name"), "Springfield trip");
            assert_eq!(text(&metas[0], "description"), "Notes · pack the tent");
            assert!(text(&metas[1], "gicon").contains("Skykeep.Calendar"));
            let () = gnome
                .call(
                    "ActivateResult",
                    &("calendar/E1", vec!["springfield"], 0u32),
                )
                .await
                .unwrap();
            let () = gnome
                .call("LaunchSearch", &(vec!["springfield"], 0u32))
                .await
                .unwrap();

            let krunner = zbus::Proxy::new(&client, name, KRUNNER_PATH, "org.kde.krunner1")
                .await
                .unwrap();
            // KRunner's match: id, text, icon, type, relevance, properties.
            type Match = (
                String,
                String,
                String,
                i32,
                f64,
                HashMap<String, zbus::zvariant::OwnedValue>,
            );
            let matches: Vec<Match> = krunner.call("Match", &("budget",)).await.unwrap();
            assert_eq!(matches.len(), 1);
            assert_eq!(matches[0].0, "notes/N2");
            assert_eq!(matches[0].3, 100);
            let actions: Vec<(String, String, String)> =
                krunner.call("Actions", &()).await.unwrap();
            assert!(actions.is_empty());
            let () = krunner.call("Run", &("notes/N2", "")).await.unwrap();
        });
        let opened = opened.lock().unwrap();
        assert_eq!(opened.len(), 2);
        assert_eq!(
            opened[0],
            Reveal {
                screen: "calendar".into(),
                id: "E1".into(),
                within: Some("CAL".into()),
                at: Some(1_767_225_600),
                query: "springfield".into(),
            }
        );
        assert_eq!(opened[1].screen, "notes");
        assert_eq!(opened[1].id, "N2");
        assert_eq!(*more.lock().unwrap(), vec!["springfield".to_string()]);
    }
}
