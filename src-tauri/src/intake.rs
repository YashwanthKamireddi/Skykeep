//! What comes into Skykeep from outside its own windows: a calendar or contact file opened
//! with it (`.ics`, `.vcf`) and the launcher actions that start something new (`skykeep --new
//! reminder`).
//!
//! A file is read here, on this computer, and nothing is sent anywhere until the person says
//! so in the import sheet: what it holds is shown first — the events with their days and
//! repeats, the people with their numbers — and what cannot be brought in is said, with why.
//! Confirmed, the sheet writes through the same commands the apps' own forms use
//! (`event_save`, `contacts_import`), so an imported event is made exactly as one typed in.
//!
//! A quick capture is a small window of its own with one field: a reminder into the list last
//! added to, or a note into Notes, written with `reminder_create` / `note_create` and gone.

use serde::Serialize;
use std::path::Path;
use std::sync::Mutex as StdMutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp};
use quince_icloud::calendar::EventRepeat;
use quince_icloud::zones;

/// Told to a window when something has arrived for it: it asks `intake_pending` for it.
const ARRIVED: &str = "quince://intake";
/// Told to the quick-capture window when it is asked for again, perhaps for the other kind.
const CAPTURE: &str = "quince://capture";
/// The quick-capture window's label.
const CAPTURE_WINDOW: &str = "capture";
/// Larger than any calendar or address book a person would open by hand; a file past it is
/// refused rather than read into memory.
const MOST_BYTES: u64 = 16 * 1024 * 1024;

/// What arrived, waiting for the window it is for (by label) to collect it. A window that is
/// still loading collects it once drawn; one already open is told.
static PENDING: StdMutex<Vec<(String, Arrival)>> = StdMutex::new(Vec::new());

/// One opened file, read and ready to be shown.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Arrival {
    /// A calendar file: the events that can be added, and those that cannot.
    Events {
        file: String,
        events: Vec<ImportEvent>,
        refused: Vec<Refused>,
    },
    /// A contacts file: the people in it. Added by `contacts_import` from `path`.
    Contacts {
        file: String,
        path: String,
        contacts: Vec<ImportContact>,
    },
    /// A file that could not be read at all.
    Unreadable { file: String, reason: String },
}

/// An event as `event_save` takes it, less the calendar the person picks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEvent {
    pub title: String,
    pub location: Option<String>,
    pub starts_at: i64,
    pub ends_at: i64,
    pub all_day: bool,
    pub timezone: Option<String>,
    pub notes: Option<String>,
    pub url: Option<String>,
    /// Minutes before the start, `0` at the time; `None` when the file sets none.
    pub alerts: Option<Vec<i64>>,
    pub repeat: Option<EventRepeat>,
    /// Already in a calendar here: the same title at the same time. Left out of the import.
    pub duplicate: bool,
}

/// Something in a file that cannot be brought in, and why, in plain words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Refused {
    pub title: String,
    pub reason: String,
}

/// A person in a contacts file, for the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportContact {
    pub name: String,
    /// A phone number or address to tell people of the same name apart.
    pub detail: Option<String>,
    /// Already in Contacts; `contacts_import` leaves them out.
    pub known: bool,
}

/// Act on a command line — this launch's, or one forwarded by a second launch: a quick
/// capture, or files to import. Whether it asked for either; if not, the caller opens what
/// it would have opened anyway.
pub fn arrived(app: &AppHandle, args: &[String], cwd: &Path) -> bool {
    // Send to iPhone, from a file manager: sent, and no window opened for it.
    if let Some(files) = crate::send::asked(args, cwd) {
        crate::send::send(app, files);
        return true;
    }
    if let Some(kind) = crate::apps::new_asked(args) {
        // An event is made in Calendar's own window, with its days and times; the others in
        // the one-field capture.
        if kind == "event" {
            crate::apps::new_event(app);
        } else {
            capture(app, kind);
        }
        return true;
    }
    let files = crate::apps::files_asked(args, cwd);
    if files.is_empty() {
        return false;
    }
    for path in files {
        // Opened with Skykeep by the person, from the system: its contacts may be imported.
        crate::grants::grant_read(&path);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let (window_key, arrival) = read(&app, &path).await;
            let label = format!("app-{window_key}");
            if let Ok(mut pending) = PENDING.lock() {
                pending.push((label.clone(), arrival));
            }
            // Its app's window: made (it collects the file once drawn) or brought forward
            // and told.
            crate::apps::open(&app, window_key);
            app.emit_to(label.as_str(), ARRIVED, ()).ok();
        });
    }
    true
}

/// Read one file, and which app's window shows it.
async fn read(app: &AppHandle, path: &Path) -> (&'static str, Arrival) {
    let file = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let lower = file.to_ascii_lowercase();
    let contacts_file = lower.ends_with(".vcf") || lower.ends_with(".vcard");
    let key = if contacts_file {
        "contacts"
    } else {
        "calendar"
    };
    let text = match load(path) {
        Ok(text) => text,
        Err(reason) => return (key, Arrival::Unreadable { file, reason }),
    };
    let data = app.state::<crate::Data>();
    if contacts_file {
        // Beside a sync pass, not in the writer's turn (desktop audit H1).
        let book = data
            .0
            .read(|library| library.contacts(None))
            .await
            .unwrap_or_default();
        let contacts = preview_contacts(&text, &book);
        if contacts.is_empty() {
            let reason = "There are no contacts in this file.".to_string();
            return (key, Arrival::Unreadable { file, reason });
        }
        let path = path.display().to_string();
        return (
            key,
            Arrival::Contacts {
                file,
                path,
                contacts,
            },
        );
    }
    let zone = TimeZone::system();
    let (mut events, refused) = match parse_calendar(&text, &zone) {
        Ok(read) => read,
        Err(reason) => return (key, Arrival::Unreadable { file, reason }),
    };
    let duplicates = data
        .0
        .read(|library| {
            Ok(events
                .iter()
                .map(|event| {
                    library
                        .events_starting_between(event.starts_at, event.starts_at + 1)
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|(guid, _)| library.event(guid).ok().flatten())
                        .any(|e| e.title.trim().eq_ignore_ascii_case(event.title.trim()))
                })
                .collect::<Vec<_>>())
        })
        .await
        .unwrap_or_default();
    for (event, duplicate) in events.iter_mut().zip(duplicates) {
        event.duplicate = duplicate;
    }
    (
        key,
        Arrival::Events {
            file,
            events,
            refused,
        },
    )
}

fn load(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path)
        .map_err(|_| "The file couldn’t be found, or can’t be read.".to_string())?
        .len();
    if size > MOST_BYTES {
        return Err("The file is too large to be a calendar or contacts file.".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "The file couldn’t be read.".to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Open the quick-capture window for `kind` ("reminder" or "note"), or bring it back.
pub fn capture(app: &AppHandle, kind: &'static str) {
    if let Some(w) = app.get_webview_window(CAPTURE_WINDOW) {
        app.emit_to(CAPTURE_WINDOW, CAPTURE, kind).ok();
        w.unminimize().ok();
        w.show().ok();
        w.set_focus().ok();
        return;
    }
    let made = WebviewWindowBuilder::new(
        app,
        CAPTURE_WINDOW,
        WebviewUrl::App(format!("index.html?capture={kind}").into()),
    )
    .title(if kind == "note" {
        "New Note"
    } else {
        "New Reminder"
    })
    .inner_size(460.0, 132.0)
    // A fixed size: a tiling compositor (Hyprland, sway) floats a window that cannot be
    // resized, as it floats a dialog, rather than tiling one field across half the screen.
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .center()
    .always_on_top(true)
    .skip_taskbar(true)
    .decorations(false)
    // Shown by window_ready, once drawn: never a blank frame.
    .visible(false)
    .build();
    match made {
        Ok(window) => {
            // Shown anyway if the page never says it is drawn.
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if !window.is_visible().unwrap_or(true) {
                    window.show().ok();
                    window.set_focus().ok();
                }
            });
        }
        Err(e) => quiet!("the quick capture window could not be made: {e}"),
    }
}

/// What has arrived for the window asking, taken from the queue.
#[tauri::command]
pub fn intake_pending(window: tauri::WebviewWindow) -> Vec<Arrival> {
    let Ok(mut pending) = PENDING.lock() else {
        return Vec::new();
    };
    let label = window.label().to_string();
    let (mine, rest): (Vec<_>, Vec<_>) = pending.drain(..).partition(|(l, _)| *l == label);
    *pending = rest;
    mine.into_iter().map(|(_, a)| a).collect()
}

/// The quick-capture window, done with: closed.
#[tauri::command]
pub fn intake_capture_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window(CAPTURE_WINDOW) {
        w.close().ok();
    }
}

/// From the quick-capture window when Skykeep is not signed in: the main window, where the
/// sign-in is.
#[tauri::command]
pub fn intake_open_main(app: AppHandle) {
    crate::show_window(&app);
}

// ── vCard ───────────────────────────────────────────────────────────────────────────────────

/// The people in a vCard file, named as Contacts would name them.
pub fn preview_contacts(text: &str, book: &[quince_core::contacts::Contact]) -> Vec<ImportContact> {
    quince_icloud::contacts::split_vcards(text)
        .iter()
        .map(|card| {
            let mut full = None;
            let mut structured = None;
            let mut org = None;
            let mut email = None;
            let mut phone = None;
            for line in unfold(card) {
                let Some(p) = property(&line) else { continue };
                // `item1.EMAIL` — Apple's grouping prefix — is still an email.
                let name = p.name.rsplit('.').next().unwrap_or("").to_string();
                let value = if p
                    .param("ENCODING")
                    .is_some_and(|e| e.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
                {
                    quoted_printable(&p.value)
                } else {
                    p.value.clone()
                };
                match name.as_str() {
                    "FN" => full = Some(unescape(&value)),
                    "N" => {
                        let parts: Vec<String> = split_unescaped(&value, ';')
                            .into_iter()
                            .map(|s| unescape(&s))
                            .collect();
                        let given = parts.get(1).map_or("", String::as_str);
                        let family = parts.first().map_or("", String::as_str);
                        structured = Some(format!("{given} {family}").trim().to_string());
                    }
                    "ORG" => {
                        org = split_unescaped(&value, ';').first().map(|s| unescape(s));
                    }
                    "EMAIL" if email.is_none() => email = Some(unescape(&value)),
                    "TEL" if phone.is_none() => phone = Some(unescape(&value)),
                    _ => {}
                }
            }
            let name = [full, structured, org]
                .into_iter()
                .flatten()
                .map(|s| s.trim().to_string())
                .find(|s| !s.is_empty())
                .unwrap_or_else(|| "No Name".into());
            ImportContact {
                name,
                detail: phone.or(email).filter(|s| !s.trim().is_empty()),
                known: quince_icloud::contacts::already_in(card, book),
            }
        })
        .collect()
}

fn quoted_printable(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' && i + 2 < bytes.len() && s.is_char_boundary(i + 3) {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── iCalendar ───────────────────────────────────────────────────────────────────────────────

/// One content line: `NAME;PARAM=VALUE:value`.
#[derive(Debug, Clone)]
struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Property {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Lines with their folds undone: a line that starts with a space or tab continues the one
/// before it (RFC 5545 §3.1).
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        // vCard 2.1's quoted-printable soft line break: `=` at the end joins the next line.
        if let Some(last) = out.last_mut() {
            if last.ends_with('=') && last.to_ascii_uppercase().contains("QUOTED-PRINTABLE") {
                last.pop();
                last.push_str(line);
                continue;
            }
        }
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

fn property(line: &str) -> Option<Property> {
    // The name and parameters end at the first colon outside quotes.
    let mut quoted = false;
    let mut split = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                split = Some(i);
                break;
            }
            _ => {}
        }
    }
    let split = split?;
    let (head, value) = (&line[..split], &line[split + 1..]);
    let mut parts = split_quoted(head, ';').into_iter();
    let name = parts.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            ))
        })
        .collect();
    Some(Property {
        name,
        params,
        value: value.to_string(),
    })
}

fn split_quoted(s: &str, on: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    for c in s.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if c == on && !quoted {
            out.push(String::new());
        } else if let Some(last) = out.last_mut() {
            last.push(c);
        }
    }
    out
}

/// Split on `on`, but not on an escaped one (`\;`).
fn split_unescaped(s: &str, on: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            if let Some(last) = out.last_mut() {
                last.push('\\');
                last.push(c);
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == on {
            out.push(String::new());
        } else if let Some(last) = out.last_mut() {
            last.push(c);
        }
    }
    out
}

/// TEXT values with their escapes undone: `\n`, `\,`, `\;`, `\\`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// A component's own properties, and the components inside it (an event's alarms).
#[derive(Debug, Default)]
struct Component {
    name: String,
    props: Vec<Property>,
    children: Vec<Component>,
}

impl Component {
    fn get(&self, name: &str) -> Option<&Property> {
        self.props.iter().find(|p| p.name == name)
    }
    fn all(&self, name: &str) -> impl Iterator<Item = &Property> {
        let name = name.to_string();
        self.props.iter().filter(move |p| p.name == name)
    }
    fn text(&self, name: &str) -> Option<String> {
        self.get(name)
            .map(|p| unescape(&p.value).trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

/// The components of a file, nested as written.
fn components(text: &str) -> Vec<Component> {
    let mut stack: Vec<Component> = Vec::new();
    let mut top = Vec::new();
    for line in unfold(text) {
        let Some(p) = property(&line) else { continue };
        match p.name.as_str() {
            "BEGIN" => stack.push(Component {
                name: p.value.trim().to_ascii_uppercase(),
                ..Default::default()
            }),
            "END" => {
                let Some(done) = stack.pop() else { continue };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => top.push(done),
                }
            }
            _ => {
                if let Some(current) = stack.last_mut() {
                    current.props.push(p);
                }
            }
        }
    }
    // A file cut short: what was open still counts.
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(done),
            None => top.push(done),
        }
    }
    top
}

/// A date or time as a file writes it.
#[derive(Debug, Clone)]
enum When {
    /// `VALUE=DATE`: a day, not an instant.
    Day(Date),
    /// No zone: the same clock time wherever it is read — this computer's.
    Floating(DateTime),
    /// `…Z`.
    Utc(Timestamp),
    /// `TZID=…`: a clock time in that zone, and the zone's IANA name — none for a zone only
    /// the file's own `VTIMEZONE` defines.
    Zoned(DateTime, TimeZone, Option<String>),
}

impl When {
    fn instant(&self, here: &TimeZone) -> Option<Timestamp> {
        Some(match self {
            When::Day(d) => d.to_zoned(here.clone()).ok()?.timestamp(),
            When::Floating(dt) => dt.to_zoned(here.clone()).ok()?.timestamp(),
            When::Utc(ts) => *ts,
            When::Zoned(dt, tz, _) => dt.to_zoned(tz.clone()).ok()?.timestamp(),
        })
    }
    /// The civil time in the zone the event's own clock runs on.
    fn civil(&self, here: &TimeZone) -> DateTime {
        match self {
            When::Day(d) => d.to_datetime(jiff::civil::Time::midnight()),
            When::Floating(dt) | When::Zoned(dt, _, _) => *dt,
            When::Utc(ts) => ts.to_zoned(here.clone()).datetime(),
        }
    }
}

fn parse_date(s: &str) -> Option<Date> {
    let s = s.trim();
    if s.len() != 8 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Date::new(
        s[0..4].parse().ok()?,
        s[4..6].parse().ok()?,
        s[6..8].parse().ok()?,
    )
    .ok()
}

fn parse_datetime(s: &str) -> Option<(DateTime, bool)> {
    let s = s.trim();
    let (s, utc) = match s.strip_suffix(['Z', 'z']) {
        Some(rest) => (rest, true),
        None => (s, false),
    };
    let (d, t) = s.split_once(['T', 't'])?;
    let date = parse_date(d)?;
    if t.len() < 4 || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hour: i8 = t[0..2].parse().ok()?;
    let minute: i8 = t[2..4].parse().ok()?;
    // A leap second is read as the second before it.
    let second: i8 = t.get(4..6).map_or(Some(0), |x| x.parse().ok())?.min(59);
    let time = jiff::civil::Time::new(hour, minute, second, 0).ok()?;
    Some((date.to_datetime(time), utc))
}

/// A date or time property, its `TZID` read as this file defines it (see [`zones::Defined`]).
fn when(p: &Property, zones: &zones::Defined) -> Result<When, String> {
    let date_only = p
        .param("VALUE")
        .is_some_and(|v| v.eq_ignore_ascii_case("DATE"))
        || (p.value.trim().len() == 8 && !p.value.contains('T'));
    if date_only {
        return parse_date(&p.value)
            .map(When::Day)
            .ok_or_else(|| "Its date can’t be read.".to_string());
    }
    let (dt, utc) =
        parse_datetime(&p.value).ok_or_else(|| "Its date can’t be read.".to_string())?;
    if utc {
        return dt
            .to_zoned(TimeZone::UTC)
            .map(|z| When::Utc(z.timestamp()))
            .map_err(|_| "Its date can’t be read.".to_string());
    }
    match p.param("TZID") {
        None => Ok(When::Floating(dt)),
        Some(tzid) => match zones.zone(tzid) {
            Some(zone) => Ok(When::Zoned(dt, zone.tz, zone.name)),
            None => Err(format!(
                "It uses a time zone this computer doesn’t know ({tzid})."
            )),
        },
    }
}

/// An iCalendar duration in seconds: `PT1H30M`, `-P1D`, `P2W`.
fn duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (sign, s) = match s.as_bytes().first()? {
        b'-' => (-1, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (1, s),
    };
    let s = s.strip_prefix(['P', 'p'])?;
    let mut total = 0i64;
    let mut number = String::new();
    let mut in_time = false;
    let mut any = false;
    for c in s.chars() {
        match c.to_ascii_uppercase() {
            '0'..='9' => number.push(c),
            'T' => in_time = true,
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                any = true;
                total += n * match (unit, in_time) {
                    ('W', _) => 7 * 86_400,
                    ('D', _) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    (any && number.is_empty()).then_some(sign * total)
}

/// A calendar file read into the events it holds, and what of it cannot be brought in. An
/// error when it is not a calendar file at all.
pub fn parse_calendar(
    text: &str,
    here: &TimeZone,
) -> Result<(Vec<ImportEvent>, Vec<Refused>), String> {
    let top = components(text);
    let Some(calendar) = top.iter().find(|c| c.name == "VCALENDAR") else {
        return Err("This isn’t a calendar file.".into());
    };
    let cancelled_file = calendar
        .get("METHOD")
        .is_some_and(|m| m.value.trim().eq_ignore_ascii_case("CANCEL"));
    let here_name = here.iana_name().map(ToString::to_string);
    let zones = zones::Defined::read(text);
    let mut events = Vec::new();
    let mut refused = Vec::new();
    for c in &calendar.children {
        let title = c.text("SUMMARY");
        let shown = || title.clone().unwrap_or_else(|| "Untitled".into());
        match c.name.as_str() {
            "VEVENT" => {}
            "VTODO" => {
                refused.push(Refused {
                    title: shown(),
                    reason: "It’s a to-do, not an event. Skykeep imports events.".into(),
                });
                continue;
            }
            "VJOURNAL" => {
                refused.push(Refused {
                    title: shown(),
                    reason: "It’s a journal entry, not an event.".into(),
                });
                continue;
            }
            _ => continue,
        }
        if cancelled_file {
            refused.push(Refused {
                title: shown(),
                reason: "The file cancels this event rather than adding it.".into(),
            });
            continue;
        }
        match event_of(c, here, here_name.as_deref(), &zones) {
            Ok(event) => events.push(event),
            Err(reason) => refused.push(Refused {
                title: shown(),
                reason,
            }),
        }
    }
    Ok((events, refused))
}

fn event_of(
    c: &Component,
    here: &TimeZone,
    here_name: Option<&str>,
    zones: &zones::Defined,
) -> Result<ImportEvent, String> {
    if c.get("STATUS")
        .is_some_and(|s| s.value.trim().eq_ignore_ascii_case("CANCELLED"))
    {
        return Err("It was cancelled.".into());
    }
    if c.get("RECURRENCE-ID").is_some() {
        return Err(
            "It’s a change to one day of a repeating event, which can’t be imported on its own."
                .into(),
        );
    }
    let start = when(c.get("DTSTART").ok_or("It has no start date.")?, zones)?;
    let all_day = matches!(start, When::Day(_));
    let starts_at = start.instant(here).ok_or("Its date can’t be read.")?;

    let ends_at = if all_day {
        let When::Day(first) = start else {
            unreachable!()
        };
        let last = match (c.get("DTEND"), c.get("DURATION")) {
            (Some(end), _) => match when(end, zones)? {
                When::Day(d) => d,
                other => other.civil(here).date(),
            },
            (None, Some(d)) => {
                let secs = duration(&d.value).ok_or("Its length can’t be read.")?;
                first
                    .checked_add(Span::new().days(secs.div_euclid(86_400)))
                    .map_err(|_| "Its length can’t be read.")?
            }
            (None, None) => first,
        };
        // DTEND is the day after the last (exclusive); a one-day event may leave it out, or
        // give the same day.
        let last = if last <= first {
            first.tomorrow().map_err(|_| "Its date can’t be read.")?
        } else {
            last
        };
        When::Day(last)
            .instant(here)
            .ok_or("Its date can’t be read.")?
    } else {
        match (c.get("DTEND"), c.get("DURATION")) {
            (Some(end), _) => when(end, zones)?
                .instant(here)
                .ok_or("Its end can’t be read.")?,
            (None, Some(d)) => {
                let secs = duration(&d.value).ok_or("Its length can’t be read.")?;
                starts_at
                    .checked_add(Span::new().seconds(secs))
                    .map_err(|_| "Its length can’t be read.")?
            }
            (None, None) => starts_at,
        }
    };
    if ends_at < starts_at {
        return Err("It ends before it starts.".into());
    }

    let repeat = repeat_of(c, &start, here)?;
    let alerts = alerts_of(c, starts_at);
    let timezone = match &start {
        // A zone only the file defines has no name Apple would know: the instants are its,
        // and the event is kept on this computer's clock.
        When::Zoned(_, _, Some(name)) => Some(name.clone()),
        _ => here_name.map(ToString::to_string),
    };
    Ok(ImportEvent {
        title: c.text("SUMMARY").unwrap_or_else(|| "New Event".into()),
        location: c.text("LOCATION"),
        starts_at: starts_at.as_second(),
        ends_at: ends_at.as_second(),
        all_day,
        timezone,
        notes: c.text("DESCRIPTION"),
        url: c
            .get("URL")
            .map(|p| p.value.trim().to_string())
            .filter(|s| !s.is_empty()),
        alerts,
        repeat,
        duplicate: false,
    })
}

/// How an event repeats, as Apple's repeat holds it ([`quince_icloud::rrule`], the one reading
/// every way in shares). A rule saying more than that (the second Tuesday, skipped or extra
/// dates) is refused rather than brought in as something else.
fn repeat_of(c: &Component, start: &When, here: &TimeZone) -> Result<Option<EventRepeat>, String> {
    let rules: Vec<&Property> = c.all("RRULE").collect();
    if c.get("RDATE").is_some() || rules.len() > 1 {
        return Err("It repeats on extra dates, which Skykeep can’t set up.".into());
    }
    let Some(rule) = rules.first() else {
        return Ok(None);
    };
    if c.get("EXDATE").is_some() {
        return Err("Its repeat skips some dates, which Skykeep can’t set up.".into());
    }
    let zone = match start {
        When::Zoned(_, tz, _) => tz.clone(),
        _ => here.clone(),
    };
    quince_icloud::rrule::from_rrule(
        &rule.value,
        start.civil(here),
        &zone,
        matches!(start, When::Day(_)),
    )
}

/// The event's alerts: minutes before its start, from each `VALARM` that rings at or before
/// it. One timed from the end, or after the start, is not something Apple's alerts hold.
fn alerts_of(c: &Component, starts_at: Timestamp) -> Option<Vec<i64>> {
    let mut minutes: Vec<i64> = c
        .children
        .iter()
        .filter(|a| a.name == "VALARM")
        .filter_map(|a| {
            let trigger = a.get("TRIGGER")?;
            if trigger
                .param("RELATED")
                .is_some_and(|r| r.eq_ignore_ascii_case("END"))
            {
                return None;
            }
            let before = if trigger
                .param("VALUE")
                .is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME"))
            {
                let (dt, _) = parse_datetime(&trigger.value)?;
                let at = dt.to_zoned(TimeZone::UTC).ok()?.timestamp();
                starts_at.as_second() - at.as_second()
            } else {
                -duration(&trigger.value)?
            };
            (before >= 0).then_some(before / 60)
        })
        .collect();
    minutes.sort_unstable();
    minutes.dedup();
    (!minutes.is_empty()).then_some(minutes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/intake");

    fn sample(name: &str) -> String {
        std::fs::read_to_string(format!("{DIR}/{name}")).expect("sample file")
    }

    fn kolkata() -> TimeZone {
        TimeZone::get("Asia/Kolkata").expect("zone")
    }

    fn at(s: &str) -> i64 {
        s.parse::<Timestamp>().expect("timestamp").as_second()
    }

    #[test]
    fn a_meeting_invite_is_read_in_its_own_zone_with_its_alert() {
        let (events, refused) = parse_calendar(&sample("invite.ics"), &kolkata()).unwrap();
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.title, "Design review, round 2");
        assert_eq!(e.location.as_deref(), Some("Studio; Room 4"));
        assert_eq!(
            e.notes.as_deref(),
            Some("Bring the prototypes.\nAnd the notes.")
        );
        // 10:00 in Berlin in November is 09:00 UTC.
        assert_eq!(e.starts_at, at("2026-11-05T09:00:00Z"));
        assert_eq!(e.ends_at, at("2026-11-05T10:30:00Z"));
        assert!(!e.all_day);
        assert_eq!(e.timezone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(e.alerts, Some(vec![15]));
        assert_eq!(e.repeat, None);
    }

    #[test]
    fn several_events_with_all_day_repeats_and_refusals() {
        let (events, refused) = parse_calendar(&sample("family.ics"), &kolkata()).unwrap();
        let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Holiday in Goa",
                "Gym",
                "Rent",
                "Team sync",
                "Floating lunch",
                "Classes",
                "Book club"
            ]
        );

        // All day, three days: the 24th to the 26th, ends on the 27th's midnight, here.
        let goa = &events[0];
        assert!(goa.all_day);
        assert_eq!(goa.starts_at, at("2026-12-23T18:30:00Z"));
        assert_eq!(goa.ends_at, at("2026-12-26T18:30:00Z"));
        assert_eq!(goa.timezone.as_deref(), Some("Asia/Kolkata"));

        // Weekly on the start's own weekday, ten times: passed through.
        assert_eq!(
            events[1].repeat,
            Some(EventRepeat {
                freq: "weekly".into(),
                interval: 1,
                count: Some(10),
                ..EventRepeat::default()
            })
        );
        // Monthly until a date: the end day kept as Apple's repeat holds one.
        assert_eq!(
            events[2].repeat,
            Some(EventRepeat {
                freq: "monthly".into(),
                interval: 1,
                until: Some("2027-06-01".into()),
                ..EventRepeat::default()
            })
        );
        assert!(events[2].all_day);
        // Every other day, for ever, in UTC: the instant kept, written on this computer's clock.
        assert_eq!(
            events[3].repeat,
            Some(EventRepeat {
                freq: "daily".into(),
                interval: 2,
                ..EventRepeat::default()
            })
        );
        assert_eq!(events[3].starts_at, at("2026-11-02T04:30:00Z"));
        assert_eq!(events[3].timezone.as_deref(), Some("Asia/Kolkata"));
        // No zone at all: 13:00 on this computer's clock; no end, so a duration.
        assert_eq!(events[4].starts_at, at("2026-11-03T07:30:00Z"));
        assert_eq!(events[4].ends_at, at("2026-11-03T08:15:00Z"));

        let reasons: Vec<(&str, &str)> = refused
            .iter()
            .map(|r| (r.title.as_str(), r.reason.as_str()))
            .collect();
        // Monday, Wednesday and Friday each week: Apple's repeat holds that.
        assert_eq!(
            events[5].repeat,
            Some(EventRepeat {
                freq: "weekly".into(),
                interval: 1,
                week_days: vec!["MO".into(), "WE".into(), "FR".into()],
                ..EventRepeat::default()
            })
        );
        // Monthly on the second Tuesday, starting on one (10 Nov 2026): Apple's repeat holds that.
        assert_eq!(
            events[6].repeat,
            Some(EventRepeat {
                freq: "monthly".into(),
                interval: 1,
                position: Some("2nd".into()),
                week_days: vec!["TU".into()],
                ..EventRepeat::default()
            })
        );
        assert_eq!(reasons.len(), 4, "{reasons:?}");
        assert!(reasons[0].0 == "Standup" && reasons[0].1.contains("skips some dates"));
        assert!(reasons[1].0 == "Standup" && reasons[1].1.contains("one day of a repeating"));
        assert!(reasons[2].0 == "Cancelled thing" && reasons[2].1.contains("cancelled"));
        assert!(reasons[3].0 == "Buy milk" && reasons[3].1.contains("to-do"));
    }

    #[test]
    fn outlook_zone_names_and_unknown_zones() {
        let (events, refused) = parse_calendar(&sample("outlook.ics"), &kolkata()).unwrap();
        assert_eq!(events.len(), 2);
        // 09:00 Eastern in July is 13:00 UTC.
        assert_eq!(events[0].starts_at, at("2026-07-14T13:00:00Z"));
        assert_eq!(events[0].timezone.as_deref(), Some("America/New_York"));
        // A zone carried inside a longer id.
        assert_eq!(events[1].timezone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(events[1].alerts, Some(vec![0, 1440]));
        assert_eq!(refused.len(), 1);
        assert!(refused[0].reason.contains("Mars/Olympus_Mons"));
    }

    /// An Exchange invitation as it arrives by mail: a Windows zone with its VTIMEZONE, and
    /// Outlook's "Customized Time Zone" that only the file defines. The second was refused as
    /// a zone this computer doesn't know.
    #[test]
    fn an_exchange_invite_and_a_zone_only_the_file_defines() {
        let (events, refused) = parse_calendar(&sample("exchange.ics"), &kolkata()).unwrap();
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(events.len(), 2);
        // 10:00 in Berlin in July is 08:00 UTC; kept in Berlin by its IANA name.
        assert_eq!(events[0].title, "Budget review");
        assert_eq!(events[0].starts_at, at("2026-07-15T08:00:00Z"));
        assert_eq!(events[0].ends_at, at("2026-07-15T09:30:00Z"));
        assert_eq!(events[0].timezone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(events[0].alerts, Some(vec![15]));
        // 09:00 at +05:45 is 03:15 UTC; a zone with no name is kept on this computer's clock.
        assert_eq!(events[1].starts_at, at("2026-12-01T03:15:00Z"));
        assert_eq!(events[1].ends_at, at("2026-12-01T03:45:00Z"));
        assert_eq!(events[1].timezone.as_deref(), Some("Asia/Kolkata"));
    }

    #[test]
    fn not_a_calendar_and_a_cancellation() {
        assert!(parse_calendar("hello", &kolkata()).is_err());
        let cancel = "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nSUMMARY:Off\r\nDTSTART:20261101T100000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let (events, refused) = parse_calendar(cancel, &kolkata()).unwrap();
        assert!(events.is_empty());
        assert!(refused[0].reason.contains("cancels"));
    }

    #[test]
    fn durations_and_escapes() {
        assert_eq!(duration("PT1H30M"), Some(5400));
        assert_eq!(duration("-PT15M"), Some(-900));
        assert_eq!(duration("P1W"), Some(604_800));
        assert_eq!(duration("P1DT2H"), Some(93_600));
        assert_eq!(duration("PT"), None);
        assert_eq!(duration("1H"), None);
        assert_eq!(unescape(r"a\, b\; c\\d\ne"), "a, b; c\\d\ne");
    }

    #[test]
    fn contacts_are_named_and_known_ones_marked() {
        let book: Vec<quince_core::contacts::Contact> =
            serde_json::from_value(serde_json::json!([{
                "guid": "c1", "firstName": "Zoya", "lastName": "Ahuja", "company": null,
                "isCompany": false, "birthday": null, "notes": null, "etag": null,
                "methods": [{ "kind": "phone", "label": null, "value": "+91 90000 00001" }]
            }]))
            .unwrap();
        let people = preview_contacts(&sample("people.vcf"), &book);
        let names: Vec<&str> = people.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Zoya Ahuja",
                "Ravi Kumar",
                "Northfield University",
                "José Núñez"
            ]
        );
        assert!(people[0].known);
        assert!(!people[1].known);
        assert_eq!(people[1].detail.as_deref(), Some("+91 98765 43210"));
        assert_eq!(
            people[2].detail.as_deref(),
            Some("office@northfield.example")
        );
        assert!(preview_contacts("nothing here", &book).is_empty());
    }
}
