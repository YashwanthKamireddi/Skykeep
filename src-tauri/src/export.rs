//! Export Everything: the whole local copy of the person's iCloud, written into a folder they
//! choose in formats any other app opens — Markdown, iCalendar, vCard, CSV and the files
//! themselves. Their data, theirs to keep.
//!
//! Read only from the library and the caches beside it: nothing is asked of Apple, so an export
//! works offline and costs no request. What is not on this computer (a photo never opened, a
//! Drive file never downloaded) is listed rather than fetched.
//!
//! Crash-safe: everything is written into `dest/.partial-<time>` and renamed to its name only
//! once whole. A cancel, an error or a crash never leaves a folder that looks finished; a
//! partial one a crash left is cleared by the next export to the same place.

use crate::{Data, UiResult};
use jiff::tz::TimeZone;
use jiff::Timestamp;
use quince_core::calendar::{Calendar, Event};
use quince_core::contacts::{Contact, ContactDetails, PostalAddress};
use quince_core::drive::DriveItem;
use quince_core::export::ExportPhoto;
use quince_core::notes::{Note, NoteFolder};
use quince_core::reminders::{Reminder, ReminderList};
use quince_core::Library;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, State};

/// Told to the window as an export moves along: an [`ExportProgress`].
pub const EVENT: &str = "quince://export";

/// What a cancelled export answers, so the window can tell it from a failure.
pub const CANCELLED: &str = "Export cancelled.";

/// Left inside a partial export, so a later export knows the folder is one of its own leftovers
/// and may clear it — and nothing else.
const MARKER: &str = ".skykeep-export";

/// How long the finished export's folder waits to be renamed into place while Windows holds a
/// file in it: a scan of the last large videos written can take a while, and the alternative
/// is an export of hours thrown away at its very last step.
const EXPORT_HELD_FOR: std::time::Duration = std::time::Duration::from_secs(30);

static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);
/// The last finished export, the one folder Show in Folder may show.
static LAST: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Where an export is: which service (`step` of `steps`), and how far through it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub service: &'static str,
    pub step: usize,
    pub steps: usize,
    pub done: u64,
    pub total: u64,
}

/// What a finished export holds.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub path: String,
    pub notes: usize,
    pub reminders: usize,
    pub contacts: usize,
    pub events: usize,
    pub drive_files: usize,
    pub photos: usize,
    /// Photos left out because no copy is on this computer.
    pub photos_missing: usize,
}

/// Everything an export writes, read from the library at once and then let go of, so the
/// library is never held while files are written.
#[derive(Default)]
pub(crate) struct Snapshot {
    folders: Vec<NoteFolder>,
    notes: Vec<Note>,
    lists: Vec<ReminderList>,
    reminders: Vec<Reminder>,
    contacts: Vec<(Contact, ContactDetails, Vec<PostalAddress>)>,
    calendars: Vec<Calendar>,
    events: Vec<Event>,
    drive: Vec<DriveItem>,
    /// `None`: photos were not asked for.
    photos: Option<Vec<ExportPhoto>>,
}

pub(crate) fn gather(lib: &Library, photos: bool) -> Result<Snapshot, String> {
    let e = |e: quince_core::Error| e.to_string();
    // Whole cards: the details too (title, nickname, prefix…), which a backup left out.
    let contacts = lib
        .cards(None)
        .map_err(e)?
        .into_iter()
        .map(|card| {
            let addresses = lib
                .contact_addresses(&card.contact.guid)
                .unwrap_or_default();
            (card.contact, card.details, addresses)
        })
        .collect();
    Ok(Snapshot {
        folders: lib.note_folders().map_err(e)?,
        notes: lib.export_notes().map_err(e)?,
        lists: lib.reminder_lists().map_err(e)?,
        reminders: lib.export_reminders().map_err(e)?,
        contacts,
        calendars: lib.calendars().map_err(e)?,
        events: lib.export_events().map_err(e)?,
        drive: lib.export_drive_items().map_err(e)?,
        photos: if photos {
            Some(lib.export_photos().map_err(e)?)
        } else {
            None
        },
    })
}

/// The caches an export copies from, inside the app's data folder. Only ever read.
pub(crate) struct Sources {
    note_media: PathBuf,
    photos: PathBuf,
    drive_cache: PathBuf,
}

impl Sources {
    pub(crate) fn at(data_dir: &Path) -> Self {
        Self {
            note_media: data_dir.join("note-media"),
            photos: data_dir.join("photos"),
            drive_cache: data_dir.join("drive-cache"),
        }
    }
}

/// Why an export stopped short.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    Cancelled,
    Failed(String),
}

impl From<std::io::Error> for Stop {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::StorageFull {
            Stop::Failed("The disk is full. Free some room, or choose another drive.".into())
        } else {
            Stop::Failed(format!("The export couldn’t be written: {e}"))
        }
    }
}

type Done<T> = Result<T, Stop>;

// ── The folder chosen ──────────────────────────────────────────────────────────────────────

/// A folder no export goes into: the system's own. Writing there would need rights a person
/// should not hand an app, and anything put there is in the way of the system.
fn is_system(p: &Path) -> bool {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy().to_ascii_lowercase();
        let s = s.trim_start_matches(r"\\?\");
        let rest = s.get(2..).unwrap_or("");
        rest.is_empty()
            || rest == r"\"
            || [
                r"\windows",
                r"\program files",
                r"\program files (x86)",
                r"\programdata",
            ]
            .iter()
            .any(|d| rest == *d || rest.starts_with(&format!(r"{d}\")))
    }
    #[cfg(not(windows))]
    {
        // A person's own places that sit under a system folder: removable drives and the
        // home folders of the distributions that keep them under /var.
        const OURS: [&str; 4] = ["/run/media", "/var/home", "/var/mnt", "/Volumes"];
        if OURS.iter().any(|o| p.starts_with(o) && p != Path::new(o)) {
            return false;
        }
        const SYSTEM: [&str; 24] = [
            "/bin",
            "/boot",
            "/dev",
            "/etc",
            "/efi",
            "/lib",
            "/lib32",
            "/lib64",
            "/libx32",
            "/nix",
            "/opt",
            "/proc",
            "/root",
            "/run",
            "/sbin",
            "/snap",
            "/srv",
            "/sys",
            "/usr",
            "/var",
            "/System",
            "/Library",
            "/Applications",
            "/private",
        ];
        p == Path::new("/")
            || p == Path::new("/home")
            || p == Path::new("/Users")
            || SYSTEM.iter().any(|d| p.starts_with(d))
    }
}

/// The folder an export may go into, made absolute and real — or why not.
///
/// Never inside the app's own data (an export there would be copied into the next export, and
/// cleared with the caches), never in a folder that is iCloud itself (`synced`: the Drive and
/// Photos folders, where every file written goes up), and never in a system folder.
pub(crate) fn check_dest(
    dest: &Path,
    data_dir: &Path,
    synced: &[PathBuf],
) -> Result<PathBuf, String> {
    if dest.as_os_str().is_empty() || !dest.is_absolute() {
        return Err("Choose a folder to export into.".into());
    }
    let real =
        std::fs::canonicalize(dest).map_err(|_| "That folder isn’t there any more.".to_string())?;
    if !real.is_dir() {
        return Err("Choose a folder to export into.".into());
    }
    let data = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    if real.starts_with(&data) {
        return Err("That folder holds Skykeep’s own data. Choose another one.".into());
    }
    for s in synced {
        let s = std::fs::canonicalize(s).unwrap_or_else(|_| s.clone());
        if real.starts_with(&s) {
            return Err("That folder is in iCloud: everything put there would upload again. Choose one on this computer.".into());
        }
    }
    if is_system(&real) {
        return Err("That’s a system folder. Choose one in your home folder or on a drive.".into());
    }
    Ok(real)
}

// ── Names ──────────────────────────────────────────────────────────────────────────────────

/// A name made safe to be one file name on every system (see [`crate::attachment_name`]): no
/// path in it, nothing hidden, nothing reversed; `fallback` when nothing is left.
pub(crate) fn file_name(given: &str, fallback: &str) -> String {
    let cleaned = crate::attachment_name(Some(given), "");
    // `attachment_name` names an empty one after the id, and the id given was empty.
    if cleaned == "Attachment " {
        return fallback.to_string();
    }
    // Cut short at its length limit, it can end in a space, which Windows will not keep.
    cleaned.trim_end_matches(['.', ' ']).to_string()
}

/// Names taken so far, folder by folder, ignoring case — Windows and macOS do.
#[derive(Default)]
struct Names(HashSet<String>);

impl Names {
    /// `stem.ext` in `dir`, or `stem 2.ext`, `stem 3.ext` … when it is taken.
    fn unique(&mut self, dir: &Path, stem: &str, ext: &str) -> String {
        let dot = if ext.is_empty() { "" } else { "." };
        let mut n = 1;
        loop {
            let name = if n == 1 {
                format!("{stem}{dot}{ext}")
            } else {
                format!("{stem} {n}{dot}{ext}")
            };
            let key = dir.join(&name).to_string_lossy().to_lowercase();
            if self.0.insert(key) {
                return name;
            }
            n += 1;
        }
    }
}

/// A name split into its stem and extension (`""` for none).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 8 => (stem, ext),
        _ => (name, ""),
    }
}

/// What a file is, by its first bytes, as an extension; `None` when it can't be told.
fn sniff(path: &Path) -> Option<&'static str> {
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .ok()?;
    let h = &head[..n];
    Some(if h.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if h.starts_with(b"\x89PNG") {
        "png"
    } else if h.starts_with(b"GIF8") {
        "gif"
    } else if h.starts_with(b"%PDF") {
        "pdf"
    } else if h.len() >= 12 && &h[4..8] == b"ftyp" {
        match &h[8..12] {
            b"heic" | b"heix" | b"mif1" | b"msf1" | b"heim" | b"heis" => "heic",
            b"qt  " => "mov",
            _ => "mp4",
        }
    } else if h.starts_with(b"PK\x03\x04") {
        "zip"
    } else {
        return None;
    })
}

// ── Writing ────────────────────────────────────────────────────────────────────────────────

/// One file's text, written whole.
fn write(path: &Path, text: &str) -> Done<()> {
    std::fs::write(path, text)?;
    Ok(())
}

/// A file copied a megabyte at a time, stopping between any two when asked to.
fn copy(from: &Path, to: &Path, cancel: &AtomicBool) -> Done<u64> {
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::File::create(to)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
        total += n as u64;
    }
    dst.flush()?;
    drop(dst);
    // Every file copied out is iCloud's, much of it other people's (Windows: Mark of the Web).
    crate::saving::mark_from_internet(to);
    Ok(total)
}

/// Dated as the thing it holds was last changed, so a file manager sorts it truly.
fn date(path: &Path, at: Option<i64>) {
    let Some(at) = at.and_then(|s| u64::try_from(s).ok()) else {
        return;
    };
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(at);
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(when);
    }
}

/// Progress, told no more than every tenth of a second, and always at a service's start and
/// end.
struct Ticker<'a> {
    step: usize,
    steps: usize,
    last: std::time::Instant,
    tell: &'a mut dyn FnMut(ExportProgress),
    cancel: &'a AtomicBool,
}

impl Ticker<'_> {
    fn start(&mut self, service: &'static str, total: u64) -> Done<()> {
        self.step += 1;
        self.last = std::time::Instant::now();
        self.send(service, 0, total);
        self.check()
    }
    fn tick(&mut self, service: &'static str, done: u64, total: u64) -> Done<()> {
        if done >= total || self.last.elapsed() >= std::time::Duration::from_millis(100) {
            self.last = std::time::Instant::now();
            self.send(service, done, total);
        }
        self.check()
    }
    fn send(&mut self, service: &'static str, done: u64, total: u64) {
        (self.tell)(ExportProgress {
            service,
            step: self.step,
            steps: self.steps,
            done: done.min(total),
            total,
        });
    }
    fn check(&self) -> Done<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(Stop::Cancelled)
        } else {
            Ok(())
        }
    }
}

// ── iCalendar and vCard ────────────────────────────────────────────────────────────────────

/// Text as a property value: backslash, semicolon, comma and line breaks escaped (RFC 5545
/// §3.3.11, RFC 6350 §3.4).
fn escape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let v = v.replace("\r\n", "\n").replace('\r', "\n");
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// One content line, folded at 75 octets — never inside a character — and ended with CRLF.
fn line(out: &mut String, text: &str) {
    let mut width = 0;
    for c in text.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

fn utc(ts: i64) -> String {
    Timestamp::from_second(ts)
        .map(|t| t.strftime("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_else(|_| "19700101T000000Z".into())
}

/// The zone a record names, or this computer's.
fn zone_of(name: Option<&str>, fallback: &TimeZone) -> TimeZone {
    name.and_then(|n| TimeZone::get(n).ok())
        .unwrap_or_else(|| fallback.clone())
}

fn day(ts: i64, tz: &TimeZone) -> String {
    Timestamp::from_second(ts)
        .map(|t| t.to_zoned(tz.clone()).strftime("%Y%m%d").to_string())
        .unwrap_or_else(|_| "19700101".into())
}

fn calendar_head(out: &mut String, name: &str) {
    line(out, "BEGIN:VCALENDAR");
    line(out, "VERSION:2.0");
    line(out, "PRODID:-//Skykeep//Export Everything//EN");
    line(out, "CALSCALE:GREGORIAN");
    line(out, &format!("X-WR-CALNAME:{}", escape(name)));
}

fn event_ics(out: &mut String, e: &Event, stamp: &str, tz: &TimeZone) {
    line(out, "BEGIN:VEVENT");
    line(out, &format!("UID:{}", escape(&e.guid)));
    line(out, &format!("DTSTAMP:{stamp}"));
    if e.all_day {
        let tz = zone_of(e.timezone.as_deref(), tz);
        line(
            out,
            &format!("DTSTART;VALUE=DATE:{}", day(e.starts_at, &tz)),
        );
        let end = if e.ends_at > e.starts_at {
            e.ends_at
        } else {
            e.starts_at + 86_400
        };
        line(out, &format!("DTEND;VALUE=DATE:{}", day(end, &tz)));
    } else {
        line(out, &format!("DTSTART:{}", utc(e.starts_at)));
        line(out, &format!("DTEND:{}", utc(e.ends_at.max(e.starts_at))));
    }
    line(out, &format!("SUMMARY:{}", escape(&e.title)));
    if let Some(l) = e.location.as_deref().filter(|s| !s.is_empty()) {
        line(out, &format!("LOCATION:{}", escape(l)));
    }
    if let Some(n) = e.notes.as_deref().filter(|s| !s.is_empty()) {
        line(out, &format!("DESCRIPTION:{}", escape(n)));
    }
    if let Some(u) = e.url.as_deref().filter(|s| !s.is_empty()) {
        line(out, &format!("URL:{}", escape(u)));
    }
    line(out, "END:VEVENT");
}

/// Apple's repeat frequency (0 daily … 3 yearly) as an RRULE.
fn rrule(r: &Reminder) -> Option<String> {
    let freq = match r.recur_frequency? {
        0 => "DAILY",
        1 => "WEEKLY",
        2 => "MONTHLY",
        3 => "YEARLY",
        _ => return None,
    };
    let interval = r.recur_interval.unwrap_or(1).max(1);
    Some(format!("RRULE:FREQ={freq};INTERVAL={interval}"))
}

fn todo_ics(out: &mut String, r: &Reminder, stamp: &str, tz: &TimeZone) {
    line(out, "BEGIN:VTODO");
    line(out, &format!("UID:{}", escape(&r.guid)));
    line(out, &format!("DTSTAMP:{stamp}"));
    line(out, &format!("SUMMARY:{}", escape(&r.title)));
    if let Some(n) = r.notes.as_deref().filter(|s| !s.is_empty()) {
        line(out, &format!("DESCRIPTION:{}", escape(n)));
    }
    if let Some(due) = r.due_at {
        if r.all_day {
            let tz = zone_of(r.timezone.as_deref(), tz);
            line(out, &format!("DUE;VALUE=DATE:{}", day(due, &tz)));
        } else {
            line(out, &format!("DUE:{}", utc(due)));
        }
    }
    match r.completed_at {
        Some(at) => {
            line(out, "STATUS:COMPLETED");
            line(out, &format!("COMPLETED:{}", utc(at)));
        }
        None => line(out, "STATUS:NEEDS-ACTION"),
    }
    // Apple's scale is iCalendar's: 1 high … 9 low, 0 none.
    if (1..=9).contains(&r.priority) {
        line(out, &format!("PRIORITY:{}", r.priority));
    }
    if r.flagged {
        line(out, "CATEGORIES:Flagged");
        line(out, "X-APPLE-FLAGGED:TRUE");
    }
    if let Some(p) = &r.parent_guid {
        line(out, &format!("RELATED-TO;RELTYPE=PARENT:{}", escape(p)));
    }
    if let Some(rule) = rrule(r) {
        line(out, &rule);
    }
    line(out, "END:VTODO");
}

/// A label as a `TYPE=` value: Apple's `_$!<Mobile>!$_` read as `Mobile`, quoted when it has
/// anything a parameter can't hold bare.
fn type_param(label: Option<&str>) -> String {
    let Some(label) = label else {
        return String::new();
    };
    let plain = label
        .trim_start_matches("_$!<")
        .trim_end_matches(">!$_")
        .replace(['"', '\r', '\n'], "");
    if plain.trim().is_empty() {
        return String::new();
    }
    // Apple's labels in the types every address book reads: "Mobile" is vCard's CELL, and a fax
    // is two types, not one with a space ("HOME FAX" was read as neither home nor fax).
    let standard = match plain.trim().to_ascii_uppercase().as_str() {
        "MOBILE" | "CELL" => Some("CELL"),
        "IPHONE" => Some("CELL,IPHONE"),
        "HOME FAX" => Some("HOME,FAX"),
        "WORK FAX" => Some("WORK,FAX"),
        "OTHER FAX" => Some("FAX"),
        _ => None,
    };
    if let Some(types) = standard {
        return format!(";TYPE={types}");
    }
    if plain.contains([',', ';', ':', ' ']) {
        format!(";TYPE=\"{plain}\"")
    } else {
        format!(";TYPE={plain}")
    }
}

fn vcard(out: &mut String, c: &Contact, details: &ContactDetails, addresses: &[PostalAddress]) {
    let s = |v: &Option<String>| escape(v.as_deref().unwrap_or(""));
    line(out, "BEGIN:VCARD");
    line(out, "VERSION:3.0");
    line(out, &format!("UID:{}", escape(&c.guid)));
    line(
        out,
        &format!(
            "N:{};{};{};{};{}",
            s(&c.last_name),
            s(&c.first_name),
            s(&c.middle_name),
            s(&details.prefix),
            s(&details.suffix)
        ),
    );
    line(out, &format!("FN:{}", escape(&c.display_name())));
    let filled = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(escape)
    };
    if let Some(n) = filled(&details.nickname) {
        line(out, &format!("NICKNAME:{n}"));
    }
    match (filled(&c.company), filled(&details.department)) {
        (Some(o), Some(d)) => line(out, &format!("ORG:{o};{d}")),
        (Some(o), None) => line(out, &format!("ORG:{o}")),
        (None, Some(d)) => line(out, &format!("ORG:;{d}")),
        (None, None) => {}
    }
    if let Some(t) = filled(&details.job_title) {
        line(out, &format!("TITLE:{t}"));
    }
    if let Some(f) = filled(&details.phonetic_first_name) {
        line(out, &format!("X-PHONETIC-FIRST-NAME:{f}"));
    }
    if let Some(l) = filled(&details.phonetic_last_name) {
        line(out, &format!("X-PHONETIC-LAST-NAME:{l}"));
    }
    if let Some(o) = filled(&details.phonetic_company) {
        line(out, &format!("X-PHONETIC-ORG:{o}"));
    }
    if c.is_company {
        line(out, "X-ABShowAs:COMPANY");
    }
    if let Some(b) = c.birthday.as_deref().filter(|s| !s.is_empty()) {
        // A birthday given without a year is kept by Apple in the year 1604, and goes out as
        // Apple's own vCards say it, or every other address book reads it as 1604.
        if b.starts_with("1604-") {
            line(out, &format!("BDAY;X-APPLE-OMIT-YEAR=1604:{}", escape(b)));
        } else {
            line(out, &format!("BDAY:{}", escape(b)));
        }
    }
    for m in &c.methods {
        let t = type_param(m.label.as_deref());
        let v = escape(&m.value);
        match m.kind.as_str() {
            "phone" => line(out, &format!("TEL{t}:{v}")),
            "email" => line(out, &format!("EMAIL;TYPE=INTERNET{t}:{v}")),
            "url" => line(out, &format!("URL{t}:{v}")),
            // Written whole from its parts below; a card with none kept keeps its lines.
            "address" if !addresses.is_empty() => {}
            "address" => line(out, &format!("ADR{t}:;;{v};;;;")),
            other => {
                let name: String = other
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect::<String>()
                    .to_ascii_uppercase();
                let name = if name.is_empty() {
                    "OTHER".into()
                } else {
                    name
                };
                line(out, &format!("X-{name}{t}:{v}"));
            }
        }
    }
    for a in addresses {
        let t = type_param(a.label.as_deref());
        line(
            out,
            &format!(
                "ADR{t}:;;{};{};{};{};{}",
                s(&a.street),
                s(&a.city),
                s(&a.state),
                s(&a.postal_code),
                s(&a.country)
            ),
        );
    }
    if let Some(n) = c.notes.as_deref().filter(|s| !s.is_empty()) {
        line(out, &format!("NOTE:{}", escape(n)));
    }
    line(out, "END:VCARD");
}

fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

// ── The export ─────────────────────────────────────────────────────────────────────────────

/// Clear partial exports a crash left in `dest` — only folders this exporter marked as its own.
fn clear_leftovers(dest: &Path) {
    let Ok(entries) = std::fs::read_dir(dest) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        if name.to_string_lossy().starts_with(".partial-") && e.path().join(MARKER).is_file() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Export everything in `snap` into a new folder in `dest`, which [`check_dest`] has passed.
/// Answers what was written; on a cancel or an error, nothing is left behind.
pub(crate) fn run(
    snap: &Snapshot,
    src: &Sources,
    dest: &Path,
    now: &jiff::Zoned,
    cancel: &AtomicBool,
    tell: &mut dyn FnMut(ExportProgress),
) -> Done<ExportSummary> {
    clear_leftovers(dest);
    let partial = dest.join(format!(".partial-{}", now.timestamp().as_millisecond()));
    std::fs::create_dir(&partial)?;
    std::fs::write(partial.join(MARKER), b"")?;
    let made = (|| {
        let mut summary = write_all(snap, src, &partial, now, cancel, tell)?;
        std::fs::remove_file(partial.join(MARKER))?;
        let stem = format!("Skykeep Export {}", now.strftime("%Y-%m-%d %H.%M"));
        let mut n = 1;
        let fin = loop {
            let name = if n == 1 {
                stem.clone()
            } else {
                format!("{stem} {n}")
            };
            let at = dest.join(name);
            if !at.exists() {
                break at;
            }
            n += 1;
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(Stop::Cancelled);
        }
        // On Windows a folder cannot be renamed while any file in it is open, and the files
        // just written are being scanned (Defender), indexed, or made thumbnails of: the
        // rename is tried again until they let go, rather than the whole export thrown away.
        quince_core::files::rename_within(&partial, &fin, EXPORT_HELD_FOR)?;
        summary.path = fin.to_string_lossy().into_owned();
        Ok(summary)
    })();
    if made.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    made
}

fn write_all(
    snap: &Snapshot,
    src: &Sources,
    root: &Path,
    now: &jiff::Zoned,
    cancel: &AtomicBool,
    tell: &mut dyn FnMut(ExportProgress),
) -> Done<ExportSummary> {
    let tz = now.time_zone().clone();
    let stamp = utc(now.timestamp().as_second());
    let mut t = Ticker {
        step: 0,
        steps: 5 + usize::from(snap.photos.is_some()),
        last: std::time::Instant::now(),
        tell,
        cancel,
    };
    let mut names = Names::default();
    let mut summary = ExportSummary::default();
    let mut readme = Readme::default();

    notes(
        snap,
        src,
        root,
        &mut names,
        &mut t,
        &mut summary,
        &mut readme,
    )?;
    reminders(snap, root, &tz, &stamp, &mut names, &mut t, &mut summary)?;
    contacts(snap, root, &mut t, &mut summary)?;
    calendars(snap, root, &tz, &stamp, &mut names, &mut t, &mut summary)?;
    drive(
        snap,
        src,
        root,
        &mut names,
        &mut t,
        &mut summary,
        &mut readme,
    )?;
    if let Some(photos) = &snap.photos {
        self::photos(photos, src, root, &tz, &mut names, &mut t, &mut summary)?;
    }
    write(&root.join("README.txt"), &readme.text(&summary, snap, now))?;
    Ok(summary)
}

/// What the README says beyond the counts: what was left out, and why.
#[derive(Default)]
struct Readme {
    notes_locked: usize,
    notes_unread: usize,
    attachments_missing: usize,
    drive_missing: usize,
}

impl Readme {
    fn text(&self, s: &ExportSummary, snap: &Snapshot, now: &jiff::Zoned) -> String {
        let mut o = String::new();
        let mut l = |t: &str| {
            o.push_str(t);
            o.push('\n');
        };
        l("Skykeep Export");
        l("===============");
        l("");
        l(&format!(
            "Made {} from the copy of your iCloud kept on this computer.",
            now.strftime("%A %-d %B %Y at %H:%M %Z")
        ));
        l("Everything here is in an open format any app can read. Nothing was sent anywhere.");
        l("");
        let n =
            |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
        let be = |k: usize| if k == 1 { "is" } else { "are" };
        let was = |k: usize| if k == 1 { "wasn’t" } else { "weren’t" };
        l(&format!(
            "Notes/          {} as Markdown (.md), in folders as they are in Notes.",
            n(s.notes, "note", "notes")
        ));
        l("                A note's pictures and files sit beside it when they were on this computer.");
        l(&format!(
            "Reminders/      {}: each list as iCalendar (.ics, to-dos) and as a readable .md.",
            n(s.reminders, "reminder", "reminders")
        ));
        l(&format!(
            "Contacts/       {} in one vCard file (.vcf).",
            n(s.contacts, "contact", "contacts")
        ));
        l(&format!(
            "Calendar/       {}: each calendar as iCalendar (.ics).",
            n(s.events, "event", "events")
        ));
        l(&format!(
            "iCloud Drive/   Listing.csv lists every file and folder Skykeep has seen; Files/ holds the {} downloaded to this computer.",
            n(s.drive_files, "file", "files")
        ));
        if snap.photos.is_some() {
            l(&format!(
                "Photos/         {} by year and month (YYYY/MM).",
                n(s.photos, "photo or video", "photos and videos")
            ));
        } else {
            l("Photos          Not included: photos were left out of this export.");
        }
        l("");
        l("Good to know");
        l("------------");
        l("- Repeating events are written as stored: each occurrence this computer holds is its own event, without the repeat rule. Occurrences outside the range Skykeep keeps are not in the file.");
        l("- In the .ics files times are in UTC, which every calendar app shows in your own time zone; all-day events and reminders are dates.");
        if self.notes_locked > 0 {
            l(&format!(
                "- {} {} not included: a locked note's text is encrypted with a password only you hold.",
                n(self.notes_locked, "locked note", "locked notes"),
                be(self.notes_locked)
            ));
        }
        if self.notes_unread > 0 {
            l(&format!(
                "- {} {} not included: the text hasn’t been downloaded to this computer yet.",
                n(self.notes_unread, "note", "notes"),
                be(self.notes_unread)
            ));
        }
        if self.attachments_missing > 0 {
            l(&format!(
                "- {} {} on this computer, so {} not included. Open those notes in Skykeep to download them.",
                n(self.attachments_missing, "note attachment", "note attachments"),
                was(self.attachments_missing),
                if self.attachments_missing == 1 { "it is" } else { "they are" }
            ));
        }
        if self.drive_missing > 0 {
            l(&format!(
                "- {} {} downloaded to this computer: listed in Listing.csv, not in Files/.",
                n(self.drive_missing, "Drive file", "Drive files"),
                was(self.drive_missing)
            ));
        }
        if s.photos_missing > 0 {
            l(&format!(
                "- {} {} on this computer; Photos/Not Included.txt lists them.",
                n(s.photos_missing, "photo", "photos"),
                was(s.photos_missing)
            ));
        }
        o
    }
}

fn notes(
    snap: &Snapshot,
    src: &Sources,
    root: &Path,
    names: &mut Names,
    t: &mut Ticker<'_>,
    summary: &mut ExportSummary,
    readme: &mut Readme,
) -> Done<()> {
    let total = snap.notes.len() as u64;
    t.start("notes", total)?;
    let base = root.join("Notes");
    std::fs::create_dir(&base)?;
    let by_guid: HashMap<&str, &NoteFolder> =
        snap.folders.iter().map(|f| (f.guid.as_str(), f)).collect();
    let mut dirs: HashMap<String, PathBuf> = HashMap::new();

    for (i, note) in snap.notes.iter().enumerate() {
        t.tick("notes", i as u64, total)?;
        if note.locked {
            readme.notes_locked += 1;
            continue;
        }
        let body = match &note.body {
            Some(b) => b.clone(),
            None if note.title.trim().is_empty()
                && note.snippet.as_deref().unwrap_or("").trim().is_empty() =>
            {
                String::new()
            }
            None => {
                readme.notes_unread += 1;
                continue;
            }
        };
        let dir = match note.folder_guid.as_deref() {
            Some(g) if by_guid.contains_key(g) => {
                folder_dir(g, &by_guid, &base, names, &mut dirs, 0)?
            }
            _ => base.clone(),
        };
        let runs: Vec<quince_icloud::ckdocument::Run> = note
            .runs
            .as_deref()
            .and_then(|r| serde_json::from_str(r).ok())
            .unwrap_or_default();
        let mut md = quince_icloud::markdown::note_to_markdown(&body, &runs);
        let stem = file_name(&note.title, "Untitled");
        let file = names.unique(&dir, &stem, "md");
        let file_stem = split_ext(&file).0.to_string();

        // Its attachments, in the order they stand in the text, copied beside it when they
        // are on this computer and linked from the foot of the note.
        let mut seen = HashSet::new();
        let mut links = Vec::new();
        for id in runs.iter().filter_map(|r| r.attachment.as_deref()) {
            if !seen.insert(id) {
                continue;
            }
            let safe = crate::media_file_id(id);
            let found = ["file", "drawing", "preview"]
                .iter()
                .map(|part| src.note_media.join(format!("{safe}-{part}")))
                .find(|p| p.is_file());
            let Some(from) = found else {
                // A table is an attachment with no file: nothing is missing.
                if !id.to_ascii_lowercase().contains("table") {
                    readme.attachments_missing += 1;
                }
                continue;
            };
            let ext = sniff(&from).unwrap_or("bin");
            let name = names.unique(&dir, &format!("{file_stem} - {}", links.len() + 1), ext);
            copy(&from, &dir.join(&name), t.cancel)?;
            links.push((name, matches!(ext, "jpg" | "png" | "gif" | "heic")));
        }
        if !links.is_empty() {
            md.push_str("\n\n---\n\n");
            for (name, picture) in &links {
                let target = name
                    .replace(' ', "%20")
                    .replace('(', "%28")
                    .replace(')', "%29");
                if *picture {
                    md.push_str(&format!("![{name}]({target})\n\n"));
                } else {
                    md.push_str(&format!("[{name}]({target})\n\n"));
                }
            }
        }
        let path = dir.join(&file);
        write(&path, &md)?;
        date(&path, note.modified_at.or(note.created_at));
        summary.notes += 1;
    }
    t.tick("notes", total, total)
}

/// A note folder's directory under `base`, its parents made first; made once and remembered.
fn folder_dir(
    guid: &str,
    by_guid: &HashMap<&str, &NoteFolder>,
    base: &Path,
    names: &mut Names,
    dirs: &mut HashMap<String, PathBuf>,
    depth: usize,
) -> Done<PathBuf> {
    if let Some(d) = dirs.get(guid) {
        return Ok(d.clone());
    }
    let Some(f) = by_guid.get(guid) else {
        return Ok(base.to_path_buf());
    };
    // A loop in the parents (a folder moved inside its own child, half-synced) stops here.
    let parent = match f.parent_guid.as_deref() {
        Some(p) if depth < 32 && p != guid && by_guid.contains_key(p) => {
            folder_dir(p, by_guid, base, names, dirs, depth + 1)?
        }
        _ => base.to_path_buf(),
    };
    let title = if f.is_trash() {
        "Recently Deleted"
    } else {
        &f.title
    };
    let name = names.unique(&parent, &file_name(title, "Folder"), "");
    let dir = parent.join(name);
    std::fs::create_dir_all(&dir)?;
    dirs.insert(guid.to_string(), dir.clone());
    Ok(dir)
}

fn reminders(
    snap: &Snapshot,
    root: &Path,
    tz: &TimeZone,
    stamp: &str,
    names: &mut Names,
    t: &mut Ticker<'_>,
    summary: &mut ExportSummary,
) -> Done<()> {
    let base = root.join("Reminders");
    std::fs::create_dir(&base)?;
    let mut by_list: HashMap<&str, Vec<&Reminder>> = HashMap::new();
    for r in &snap.reminders {
        by_list.entry(r.list_guid.as_str()).or_default().push(r);
    }
    // Lists in their own order, then reminders in a list this computer doesn't know.
    let mut lists: Vec<(String, &str)> = snap
        .lists
        .iter()
        .map(|l| (l.title.clone(), l.guid.as_str()))
        .collect();
    let known: HashSet<&str> = snap.lists.iter().map(|l| l.guid.as_str()).collect();
    let mut stray: Vec<&str> = by_list
        .keys()
        .copied()
        .filter(|g| !known.contains(g))
        .collect();
    stray.sort_unstable();
    lists.extend(
        stray
            .into_iter()
            .map(|g| ("Other Reminders".to_string(), g)),
    );

    let total = lists.len() as u64;
    t.start("reminders", total)?;
    for (i, (title, guid)) in lists.iter().enumerate() {
        t.tick("reminders", i as u64, total)?;
        let items = by_list.get(guid).map(Vec::as_slice).unwrap_or_default();
        let stem = file_name(title, "Reminders");
        // One stem for both files, so the pair stays together.
        let ics_name = names.unique(&base, &stem, "ics");
        let stem = split_ext(&ics_name).0.to_string();
        let md_name = names.unique(&base, &stem, "md");

        let mut ics = String::new();
        calendar_head(&mut ics, title);
        for r in items {
            todo_ics(&mut ics, r, stamp, tz);
        }
        line(&mut ics, "END:VCALENDAR");
        write(&base.join(ics_name), &ics)?;
        write(&base.join(md_name), &reminders_md(title, items, tz))?;
        summary.reminders += items.len();
    }
    t.tick("reminders", total, total)
}

/// A list as a person reads it: a checklist, subtasks under theirs, done ones ticked.
fn reminders_md(title: &str, items: &[&Reminder], tz: &TimeZone) -> String {
    let ids: HashSet<&str> = items.iter().map(|r| r.guid.as_str()).collect();
    let mut children: HashMap<&str, Vec<&Reminder>> = HashMap::new();
    let mut top = Vec::new();
    for r in items {
        match r.parent_guid.as_deref() {
            Some(p) if ids.contains(p) && p != r.guid => children.entry(p).or_default().push(r),
            _ => top.push(*r),
        }
    }
    let mut o = format!("# {title}\n\n");
    if items.is_empty() {
        o.push_str("No reminders.\n");
    }
    fn one(
        o: &mut String,
        r: &Reminder,
        depth: usize,
        children: &HashMap<&str, Vec<&Reminder>>,
        tz: &TimeZone,
    ) {
        let pad = "  ".repeat(depth);
        let tick = if r.completed_at.is_some() { "x" } else { " " };
        let mut extra = Vec::new();
        if let Some(due) = r.due_at.and_then(|d| Timestamp::from_second(d).ok()) {
            let z = due.to_zoned(zone_of(r.timezone.as_deref(), tz));
            extra.push(if r.all_day {
                format!("due {}", z.strftime("%-d %b %Y"))
            } else {
                format!("due {}", z.strftime("%-d %b %Y, %H:%M"))
            });
        }
        if r.flagged {
            extra.push("flagged".into());
        }
        match r.priority {
            1..=4 => extra.push("high priority".into()),
            5 => extra.push("medium priority".into()),
            6..=9 => extra.push("low priority".into()),
            _ => {}
        }
        if let Some(rule) = rrule(r) {
            let every = rule
                .trim_start_matches("RRULE:FREQ=")
                .split(';')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            extra.push(format!("repeats {every}"));
        }
        let title = r.title.replace('\n', " ");
        let tail = if extra.is_empty() {
            String::new()
        } else {
            format!(" — {}", extra.join(", "))
        };
        o.push_str(&format!("{pad}- [{tick}] {title}{tail}\n"));
        if let Some(n) = r.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            for l in n.lines() {
                o.push_str(&format!("{pad}  {l}\n"));
            }
        }
        if depth < 8 {
            for c in children.get(r.guid.as_str()).into_iter().flatten() {
                one(o, c, depth + 1, children, tz);
            }
        }
    }
    for r in top {
        one(&mut o, r, 0, &children, tz);
    }
    o
}

fn contacts(
    snap: &Snapshot,
    root: &Path,
    t: &mut Ticker<'_>,
    summary: &mut ExportSummary,
) -> Done<()> {
    let total = snap.contacts.len() as u64;
    t.start("contacts", total)?;
    let base = root.join("Contacts");
    std::fs::create_dir(&base)?;
    let mut out = String::new();
    for (i, (c, details, addresses)) in snap.contacts.iter().enumerate() {
        t.tick("contacts", i as u64, total)?;
        vcard(&mut out, c, details, addresses);
    }
    write(&base.join("Contacts.vcf"), &out)?;
    summary.contacts = snap.contacts.len();
    t.tick("contacts", total, total)
}

fn calendars(
    snap: &Snapshot,
    root: &Path,
    tz: &TimeZone,
    stamp: &str,
    names: &mut Names,
    t: &mut Ticker<'_>,
    summary: &mut ExportSummary,
) -> Done<()> {
    let base = root.join("Calendar");
    std::fs::create_dir(&base)?;
    let mut by_cal: HashMap<&str, Vec<&Event>> = HashMap::new();
    for e in &snap.events {
        by_cal.entry(e.calendar_guid.as_str()).or_default().push(e);
    }
    let mut cals: Vec<(String, &str)> = snap
        .calendars
        .iter()
        .map(|c| (c.title.clone(), c.guid.as_str()))
        .collect();
    let known: HashSet<&str> = snap.calendars.iter().map(|c| c.guid.as_str()).collect();
    let mut stray: Vec<&str> = by_cal
        .keys()
        .copied()
        .filter(|g| !known.contains(g))
        .collect();
    stray.sort_unstable();
    cals.extend(stray.into_iter().map(|g| ("Other Events".to_string(), g)));

    let total = cals.len() as u64;
    t.start("calendar", total)?;
    for (i, (title, guid)) in cals.iter().enumerate() {
        t.tick("calendar", i as u64, total)?;
        let items = by_cal.get(guid).map(Vec::as_slice).unwrap_or_default();
        let mut ics = String::new();
        calendar_head(&mut ics, title);
        for e in items {
            event_ics(&mut ics, e, stamp, tz);
        }
        line(&mut ics, "END:VCALENDAR");
        let name = names.unique(&base, &file_name(title, "Calendar"), "ics");
        write(&base.join(name), &ics)?;
        summary.events += items.len();
    }
    t.tick("calendar", total, total)
}

/// A Drive item's path under the Drive's root, folder names made safe, from its parents.
fn drive_path(item: &DriveItem, by_id: &HashMap<&str, &DriveItem>) -> Vec<String> {
    let mut parts = vec![file_name(&item.display_name(), "Untitled")];
    let mut at = item.parent_id.as_deref();
    let mut guard = 0;
    while let Some(p) = at.and_then(|id| by_id.get(id)) {
        if p.drivewsid == quince_core::drive::ROOT || guard > 64 {
            break;
        }
        parts.push(file_name(&p.display_name(), "Folder"));
        at = p.parent_id.as_deref();
        guard += 1;
    }
    parts.reverse();
    parts
}

/// Where the Drive cache keeps a file's bytes, and whether every one of them is there — as
/// `quince_vfs::Cache` names and marks them, read without opening (or making) the cache.
fn drive_cached(dir: &Path, item: &DriveItem) -> Option<PathBuf> {
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    };
    let path = dir.join(format!(
        "{}~{}",
        safe(&item.drivewsid),
        safe(item.etag.as_deref().unwrap_or("-"))
    ));
    let size = u64::try_from(item.size.unwrap_or(0)).unwrap_or(0);
    if size == 0 {
        return None;
    }
    let chunks = usize::try_from(size.div_ceil(quince_vfs::CHUNK)).unwrap_or(usize::MAX);
    let have = std::fs::read(path.with_extension("have")).ok()?;
    let whole = have.len() >= chunks && have[..chunks].iter().all(|&b| b == 1);
    let len = std::fs::metadata(&path).ok()?.len();
    (whole && len >= size).then_some(path)
}

fn drive(
    snap: &Snapshot,
    src: &Sources,
    root: &Path,
    names: &mut Names,
    t: &mut Ticker<'_>,
    summary: &mut ExportSummary,
    readme: &mut Readme,
) -> Done<()> {
    let total = snap.drive.len() as u64;
    t.start("drive", total)?;
    let base = root.join("iCloud Drive");
    let files = base.join("Files");
    std::fs::create_dir_all(&files)?;
    let by_id: HashMap<&str, &DriveItem> = snap
        .drive
        .iter()
        .map(|d| (d.drivewsid.as_str(), d))
        .collect();
    let mut rows: Vec<(String, &DriveItem, bool)> = Vec::new();
    let time = |s: Option<i64>| {
        s.and_then(|s| Timestamp::from_second(s).ok())
            .map(|t| t.to_string())
            .unwrap_or_default()
    };
    for (i, item) in snap.drive.iter().enumerate() {
        t.tick("drive", i as u64, total)?;
        if item.drivewsid == quince_core::drive::ROOT {
            continue;
        }
        let parts = drive_path(item, &by_id);
        let shown = parts.join("/");
        let size = u64::try_from(item.size.unwrap_or(0)).unwrap_or(0);
        // An empty file is whole without any bytes; any other only when the cache has all.
        let cached = drive_cached(&src.drive_cache, item);
        let copied = !item.is_container() && (size == 0 || cached.is_some());
        if !item.is_container() && !copied {
            readme.drive_missing += 1;
        }
        if copied {
            let mut dir = files.clone();
            for p in &parts[..parts.len() - 1] {
                dir.push(p);
            }
            std::fs::create_dir_all(&dir)?;
            let (stem, ext) = split_ext(&parts[parts.len() - 1]);
            let to = dir.join(names.unique(&dir, stem, ext));
            match cached {
                Some(from) => {
                    copy(&from, &to, t.cancel)?;
                    // The cache's file may run past the end of the file it holds.
                    std::fs::OpenOptions::new()
                        .write(true)
                        .open(&to)?
                        .set_len(size)?;
                }
                None => {
                    std::fs::File::create(&to)?;
                }
            }
            date(&to, item.modified_at);
            summary.drive_files += 1;
        }
        rows.push((shown, item, copied));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut csv = String::from("Path,Kind,Size,Created,Modified,In This Export\r\n");
    for (path, item, copied) in rows {
        let kind = match item.kind.as_str() {
            "FOLDER" => "Folder",
            "APP_LIBRARY" => "App Folder",
            _ => "File",
        };
        csv.push_str(&format!(
            "{},{},{},{},{},{}\r\n",
            csv_field(&path),
            kind,
            item.size.map(|s| s.to_string()).unwrap_or_default(),
            time(item.created_at),
            time(item.modified_at),
            if item.is_container() {
                ""
            } else if copied {
                "Yes"
            } else {
                "No"
            }
        ));
    }
    write(&base.join("Listing.csv"), &csv)?;
    t.tick("drive", total, total)
}

fn photos(
    list: &[ExportPhoto],
    src: &Sources,
    root: &Path,
    tz: &TimeZone,
    names: &mut Names,
    t: &mut Ticker<'_>,
    summary: &mut ExportSummary,
) -> Done<()> {
    let total = list.len() as u64;
    t.start("photos", total)?;
    let base = root.join("Photos");
    std::fs::create_dir(&base)?;
    let mut missing = Vec::new();
    for (i, p) in list.iter().enumerate() {
        t.tick("photos", i as u64, total)?;
        let video = p.kind == "video";
        // The original first; a video's streaming copy, or a picture's whole JPEG, when it is
        // what this computer has.
        let order: &[&str] = if video {
            &["original", "video"]
        } else {
            &["original", "full"]
        };
        let found = order
            .iter()
            .map(|s| src.photos.join(s).join(&*crate::photos::cache_name(&p.id)))
            .find(|f| f.is_file());
        let when = Timestamp::from_second(p.taken_at)
            .unwrap_or(Timestamp::UNIX_EPOCH)
            .to_zoned(tz.clone());
        let given = p.filename.clone().unwrap_or_else(|| {
            format!(
                "{} {}",
                if video { "Video" } else { "Photo" },
                when.strftime("%Y-%m-%d %H.%M.%S")
            )
        });
        let Some(from) = found else {
            missing.push(format!("{}\t{}", when.strftime("%Y-%m-%d %H:%M"), given));
            continue;
        };
        let dir = base
            .join(when.strftime("%Y").to_string())
            .join(when.strftime("%m").to_string());
        std::fs::create_dir_all(&dir)?;
        let safe = file_name(&given, "Photo");
        let (stem, named_ext) = split_ext(&safe);
        let ext = sniff(&from).map_or_else(|| named_ext.to_string(), str::to_string);
        // A name's own spelling of the same type is kept (IMG_1.JPEG stays .JPEG).
        let ext = if named_ext.eq_ignore_ascii_case(&ext)
            || (ext == "jpg" && named_ext.eq_ignore_ascii_case("jpeg"))
        {
            named_ext.to_string()
        } else {
            ext
        };
        let name = names.unique(&dir, stem, &ext);
        let to = dir.join(&name);
        copy(&from, &to, t.cancel)?;
        date(&to, Some(p.taken_at));
        // A Live Photo's moving part, beside it under the same name.
        let live = src
            .photos
            .join("live")
            .join(&*crate::photos::cache_name(&p.id));
        if p.kind == "live" && live.is_file() {
            let movie = names.unique(&dir, split_ext(&name).0, "MOV");
            copy(&live, &dir.join(&movie), t.cancel)?;
            date(&dir.join(&movie), Some(p.taken_at));
        }
        summary.photos += 1;
    }
    summary.photos_missing = missing.len();
    if !missing.is_empty() {
        let mut text = String::from(
            "These photos and videos weren't on this computer, so they aren't in this export.\nOpen them in Skykeep (or download them) and export again to include them.\n\n",
        );
        for m in &missing {
            text.push_str(m);
            text.push('\n');
        }
        write(&base.join("Not Included.txt"), &text)?;
    }
    t.tick("photos", total, total)
}

// ── Commands ───────────────────────────────────────────────────────────────────────────────

/// Clears the running flag however the export ends.
struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
    }
}

/// Export everything into a new folder inside `dest`, telling [`EVENT`] as it goes. Photos only
/// when `photos` is true: they can be very large.
#[tauri::command]
pub async fn library_export_start(
    dest: String,
    photos: Option<bool>,
    app: tauri::AppHandle,
    data: State<'_, Data>,
) -> UiResult<ExportSummary> {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Err("An export is already under way.".into());
    }
    let _running = Running;
    crate::grants::take_write(&dest)?;
    CANCEL.store(false, Ordering::Release);
    let data_dir = crate::state_dir(&app);
    // Checked and read in one hold of the library, then let go of before a file is written.
    let (snap, dest) = {
        let lib = data.0.lock().await;
        let synced: Vec<PathBuf> = ["drive_folder_path", "photos_folder_path"]
            .iter()
            .filter_map(|k| lib.state(k).ok().flatten())
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect();
        let dest = check_dest(Path::new(&dest), &data_dir, &synced)?;
        (gather(&lib, photos.unwrap_or(false))?, dest)
    };
    let sources = Sources::at(&data_dir);
    let handle = app.clone();
    let now = jiff::Zoned::now();
    let done = tokio::task::spawn_blocking(move || {
        run(&snap, &sources, &dest, &now, &CANCEL, &mut |p| {
            let _ = handle.emit(EVENT, p);
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    match done {
        Ok(summary) => {
            if let Ok(mut last) = LAST.lock() {
                *last = Some(PathBuf::from(&summary.path));
            }
            Ok(summary)
        }
        Err(Stop::Cancelled) => Err(CANCELLED.into()),
        Err(Stop::Failed(why)) => Err(why),
    }
}

/// Stop the export under way; what it wrote so far is removed.
#[tauri::command]
pub fn library_export_cancel() {
    if RUNNING.load(Ordering::Acquire) {
        CANCEL.store(true, Ordering::Release);
    }
}

/// Show the last finished export in the file manager — that folder only, never a path the page
/// names.
#[tauri::command]
pub fn library_export_reveal(app: tauri::AppHandle) -> UiResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let last = LAST.lock().ok().and_then(|l| l.clone());
    let path = last
        .filter(|p| p.is_dir())
        .ok_or_else(|| "That export isn’t there any more.".to_string())?;
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::contacts::ContactMethod;
    use quince_icloud::ckdocument::Run;

    /// Apple's phone labels go out as the types other address books read.
    #[test]
    fn phone_labels_are_written_as_vcard_types() {
        assert_eq!(type_param(Some("_$!<Mobile>!$_")), ";TYPE=CELL");
        assert_eq!(type_param(Some("HOME FAX")), ";TYPE=HOME,FAX");
        assert_eq!(type_param(Some("iPhone")), ";TYPE=CELL,IPHONE");
        assert_eq!(type_param(Some("home")), ";TYPE=home");
        assert_eq!(type_param(Some("Apple Watch")), ";TYPE=\"Apple Watch\"");
        assert_eq!(type_param(None), "");
    }

    /// A card's details go out with it: job title, department, nickname, prefix and suffix and
    /// the phonetic names. Export Everything left them all out, so a backup of a book lost them.
    #[test]
    fn a_cards_details_are_exported_with_it() {
        let c = Contact {
            guid: "C8".into(),
            first_name: Some("Hana".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Mori".into()),
            company: Some("Example Works".into()),
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: Vec::new(),
        };
        let details = ContactDetails {
            prefix: Some("Dr.".into()),
            suffix: Some("PhD".into()),
            nickname: Some("Hani".into()),
            phonetic_first_name: Some("\u{306f}\u{306a}".into()),
            phonetic_last_name: Some("\u{3082}\u{308a}".into()),
            phonetic_company: None,
            job_title: Some("Architect".into()),
            department: Some("Design".into()),
        };
        let mut out = String::new();
        vcard(&mut out, &c, &details, &[]);
        for want in [
            "N:Mori;Hana;;Dr.;PhD\r\n",
            "NICKNAME:Hani\r\n",
            "ORG:Example Works;Design\r\n",
            "TITLE:Architect\r\n",
            "X-PHONETIC-FIRST-NAME:\u{306f}\u{306a}\r\n",
            "X-PHONETIC-LAST-NAME:\u{3082}\u{308a}\r\n",
        ] {
            assert!(out.contains(want), "{want:?} in {out}");
        }
    }

    /// A birthday given without a year is stored by Apple in the year 1604; exported as it is,
    /// every other address book showed that person as four hundred years old. It goes out as
    /// Apple's own vCards write it, saying the year is left out.
    #[test]
    fn a_birthday_without_a_year_is_exported_as_one() {
        let mut c = Contact {
            guid: "C9".into(),
            first_name: Some("Mira".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Okafor".into()),
            company: None,
            is_company: false,
            birthday: Some("1604-03-14".into()),
            notes: None,
            etag: None,
            methods: Vec::new(),
        };
        let mut out = String::new();
        vcard(&mut out, &c, &ContactDetails::default(), &[]);
        assert!(
            out.contains("BDAY;X-APPLE-OMIT-YEAR=1604:1604-03-14\r\n"),
            "{out}"
        );
        c.birthday = Some("1990-03-14".into());
        let mut out = String::new();
        vcard(&mut out, &c, &ContactDetails::default(), &[]);
        assert!(out.contains("BDAY:1990-03-14\r\n"), "{out}");
    }

    /// A library on disk with a little of everything, and its data folder.
    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        lib.put_note_folders(&[
            NoteFolder {
                guid: "F1".into(),
                title: "Home/Work: plans".into(),
                sort_order: 0,
                parent_guid: None,
            },
            NoteFolder {
                guid: "F2".into(),
                title: "Recipes".into(),
                sort_order: 1,
                parent_guid: Some("F1".into()),
            },
        ])
        .unwrap();
        let runs = serde_json::to_string(&vec![
            Run {
                length: 9,
                style: Some(0),
                ..Default::default()
            },
            Run {
                length: 1,
                attachment: Some("ATT-1".into()),
                ..Default::default()
            },
            Run {
                length: 6,
                ..Default::default()
            },
        ])
        .unwrap();
        let note = |guid: &str, folder: Option<&str>, title: &str, body: Option<&str>| Note {
            guid: guid.into(),
            folder_guid: folder.map(Into::into),
            title: title.into(),
            snippet: None,
            body: body.map(Into::into),
            locked: false,
            pinned: false,
            created_at: Some(1_700_000_000),
            modified_at: Some(1_700_000_100),
            editable: true,
            etag: None,
            runs: None,
        };
        lib.put_notes(&[
            Note {
                runs: Some(runs),
                ..note(
                    "N1",
                    Some("F2"),
                    "Pancakes",
                    Some("Pancakes\n\u{FFFC}\nflour"),
                )
            },
            note("N2", Some("F2"), "Pancakes", Some("Pancakes\nagain")),
            note(
                "N3",
                None,
                "../../etc/passwd",
                Some("../../etc/passwd\nnot a path"),
            ),
            Note {
                locked: true,
                ..note("N4", None, "Secret", None)
            },
        ])
        .unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "L1".into(),
            title: "Errands; today".into(),
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        lib.put_reminders(&[
            Reminder {
                guid: "R1".into(),
                list_guid: "L1".into(),
                title: "Buy milk, eggs".into(),
                notes: Some("two litres\nfull fat".into()),
                due_at: Some(1_790_000_000),
                priority: 1,
                flagged: true,
                recur_frequency: Some(1),
                recur_interval: Some(2),
                ..Default::default()
            },
            Reminder {
                guid: "R2".into(),
                list_guid: "L1".into(),
                title: "Done one".into(),
                completed_at: Some(1_780_000_000),
                parent_guid: Some("R1".into()),
                ..Default::default()
            },
        ])
        .unwrap();
        lib.put_contacts(&[Contact {
            guid: "C1".into(),
            first_name: Some("Zoya".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Ahuja".into()),
            company: Some("Acme, Inc.".into()),
            is_company: false,
            birthday: Some("1990-04-02".into()),
            notes: Some("met at; the park".into()),
            etag: None,
            methods: vec![
                ContactMethod {
                    kind: "phone".into(),
                    label: Some("_$!<Mobile>!$_".into()),
                    value: "+91 90000 00001".into(),
                },
                ContactMethod {
                    kind: "email".into(),
                    label: Some("home".into()),
                    value: "zoya@example.com".into(),
                },
            ],
        }])
        .unwrap();
        lib.put_addresses(
            &[(
                "C1".into(),
                vec![PostalAddress {
                    label: Some("home".into()),
                    street: Some("1 Long Road\nFlat 2".into()),
                    city: Some("Pune".into()),
                    country: Some("India".into()),
                    ..Default::default()
                }],
            )],
            true,
        )
        .unwrap();
        lib.put_calendars(&[Calendar {
            guid: "CAL".into(),
            title: "Work".into(),
            enabled: false,
            writable: true,
            ..Default::default()
        }])
        .unwrap();
        lib.put_events(&[
            Event {
                guid: "E1".into(),
                calendar_guid: "CAL".into(),
                title: "Review, with notes: a very long title that has to be folded across lines in the file".into(),
                location: Some("Room 2".into()),
                starts_at: 1_790_000_000,
                ends_at: 1_790_003_600,
                notes: Some("Bring the deck;\nand coffee".into()),
                ..Default::default()
            },
            Event {
                guid: "E2".into(),
                calendar_guid: "CAL".into(),
                title: "Holiday".into(),
                starts_at: 1_789_948_800,
                ends_at: 1_790_035_200,
                all_day: true,
                timezone: Some("UTC".into()),
                ..Default::default()
            },
        ])
        .unwrap();
        lib.put_drive_children(
            quince_core::drive::ROOT,
            &[DriveItem {
                drivewsid: "FOLDER::x::docs".into(),
                parent_id: Some(quince_core::drive::ROOT.into()),
                name: "Docs".into(),
                extension: None,
                kind: "FOLDER".into(),
                size: None,
                child_count: Some(2),
                created_at: None,
                modified_at: None,
                etag: None,
            }],
        )
        .unwrap();
        lib.put_drive_children(
            "FOLDER::x::docs",
            &[
                DriveItem {
                    drivewsid: "FILE::x::cv".into(),
                    parent_id: Some("FOLDER::x::docs".into()),
                    name: "CV, final".into(),
                    extension: Some("pdf".into()),
                    kind: "FILE".into(),
                    size: Some(5),
                    child_count: None,
                    created_at: Some(1_700_000_000),
                    modified_at: Some(1_700_000_000),
                    etag: Some("e1".into()),
                },
                DriveItem {
                    drivewsid: "FILE::x::big".into(),
                    parent_id: Some("FOLDER::x::docs".into()),
                    name: "Big".into(),
                    extension: Some("mov".into()),
                    kind: "FILE".into(),
                    size: Some(10_000_000),
                    child_count: None,
                    created_at: None,
                    modified_at: None,
                    etag: Some("e2".into()),
                },
            ],
        )
        .unwrap();
        // Caches, as the app lays them out.
        let d = dir.path();
        std::fs::create_dir_all(d.join("note-media")).unwrap();
        std::fs::write(d.join("note-media/ATT-1-file"), b"\x89PNG\r\n\x1a\nxxxx").unwrap();
        std::fs::create_dir_all(d.join("drive-cache")).unwrap();
        std::fs::write(d.join("drive-cache/FILE__x__cv~e1"), b"%PDF-").unwrap();
        std::fs::write(d.join("drive-cache/FILE__x__cv~e1.have"), [1u8]).unwrap();
        (dir, lib)
    }

    fn photo(id: &str, taken: i64, filename: &str) -> quince_core::photos::Photo {
        quince_core::photos::Photo {
            id: id.into(),
            master: format!("M{id}"),
            taken_at: taken,
            added_at: None,
            kind: quince_core::photos::Kind::Image,
            width: 1,
            height: 1,
            duration: None,
            filename: Some(filename.into()),
            size: None,
            uti: None,
            favorite: false,
            hidden: false,
            deleted: false,
            deleted_at: None,
            expunge_at: None,
            title: None,
            caption: None,
            rating: 0,
            keywords: Vec::new(),
            latitude: None,
            longitude: None,
            rendered: 0,
            subtype: 0,
            burst: None,
            burst_pick: false,
            burst_count: 0,
            live_size: None,
            change_tag: None,
        }
    }

    fn now() -> jiff::Zoned {
        "2026-10-21T10:30:00+00:00[UTC]".parse().unwrap()
    }

    /// A tiny iCalendar / vCard reader: lines unfolded, each component's properties with
    /// their parameters dropped and values unescaped.
    fn parse(text: &str, kind: &str) -> Vec<HashMap<String, Vec<String>>> {
        assert!(text.ends_with("\r\n"), "lines end in CRLF");
        let mut lines: Vec<String> = Vec::new();
        for raw in text.split("\r\n") {
            assert!(raw.len() <= 75, "folded at 75 octets: {raw:?}");
            if let Some(rest) = raw.strip_prefix(' ') {
                lines.last_mut().unwrap().push_str(rest);
            } else if !raw.is_empty() {
                lines.push(raw.to_string());
            }
        }
        let mut out = Vec::new();
        let mut cur: Option<HashMap<String, Vec<String>>> = None;
        for l in lines {
            if l == format!("BEGIN:{kind}") {
                cur = Some(HashMap::new());
                continue;
            }
            if l == format!("END:{kind}") {
                out.push(cur.take().unwrap());
                continue;
            }
            let Some(c) = cur.as_mut() else { continue };
            let (head, value) = l.split_once(':').expect("name:value");
            let name = head.split(';').next().unwrap().to_string();
            let mut v = String::new();
            let mut chars = value.chars();
            while let Some(ch) = chars.next() {
                if ch == '\\' {
                    match chars.next() {
                        Some('n') | Some('N') => v.push('\n'),
                        Some(o) => v.push(o),
                        None => {}
                    }
                } else {
                    v.push(ch);
                }
            }
            c.entry(name).or_default().push(v);
        }
        assert!(cur.is_none(), "every component is closed");
        out
    }

    fn one<'a>(c: &'a HashMap<String, Vec<String>>, k: &str) -> &'a str {
        &c.get(k).unwrap_or_else(|| panic!("{k} missing"))[0]
    }

    fn export(
        dir: &Path,
        lib: &Library,
        photos: bool,
    ) -> (PathBuf, ExportSummary, tempfile::TempDir) {
        let out = tempfile::tempdir().unwrap();
        let snap = gather(lib, photos).unwrap();
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let summary = run(
            &snap,
            &Sources::at(dir),
            out.path(),
            &now(),
            &cancel,
            &mut |p| seen.push(p),
        )
        .unwrap();
        assert!(seen
            .iter()
            .any(|p| p.service == "drive" && p.done == p.total));
        assert!(
            seen.windows(2).all(|w| w[0].step <= w[1].step),
            "steps only go forward"
        );
        (PathBuf::from(&summary.path), summary, out)
    }

    #[test]
    fn writes_every_service_in_open_formats() {
        let (dir, lib) = library();
        let (root, s, _out) = export(dir.path(), &lib, false);
        assert_eq!(
            root.file_name().unwrap().to_string_lossy(),
            "Skykeep Export 2026-10-21 10.30"
        );
        assert_eq!(
            (s.notes, s.reminders, s.contacts, s.events, s.drive_files),
            (3, 2, 1, 2, 1)
        );

        // Notes: nested folders, names made safe and kept apart, attachments beside.
        let recipes = root.join("Notes/HomeWork plans/Recipes");
        let first = std::fs::read_to_string(recipes.join("Pancakes.md")).unwrap();
        assert!(first.starts_with("# Pancakes"), "{first}");
        assert!(
            first.contains("![Pancakes - 1.png](Pancakes%20-%201.png)"),
            "{first}"
        );
        assert!(recipes.join("Pancakes - 1.png").is_file());
        assert!(
            recipes.join("Pancakes 2.md").is_file(),
            "same title, both kept"
        );
        assert!(
            root.join("Notes/etcpasswd.md").is_file(),
            "a title is never a path"
        );
        assert!(
            !root.join("Notes/Secret.md").exists(),
            "a locked note has no text to write"
        );

        // Reminders.
        let ics = std::fs::read_to_string(root.join("Reminders/Errands; today.ics")).unwrap();
        let todos = parse(&ics, "VTODO");
        assert_eq!(todos.len(), 2);
        let milk = todos.iter().find(|t| one(t, "UID") == "R1").unwrap();
        assert_eq!(one(milk, "SUMMARY"), "Buy milk, eggs");
        assert_eq!(one(milk, "DESCRIPTION"), "two litres\nfull fat");
        assert_eq!(one(milk, "DUE"), "20260921T141320Z");
        assert_eq!(one(milk, "PRIORITY"), "1");
        assert_eq!(one(milk, "CATEGORIES"), "Flagged");
        assert_eq!(one(milk, "RRULE"), "FREQ=WEEKLY;INTERVAL=2");
        let done = todos.iter().find(|t| one(t, "UID") == "R2").unwrap();
        assert_eq!(one(done, "STATUS"), "COMPLETED");
        assert_eq!(one(done, "RELATED-TO"), "R1");
        let md = std::fs::read_to_string(root.join("Reminders/Errands; today.md")).unwrap();
        assert!(md.contains("- [ ] Buy milk, eggs — due"), "{md}");
        assert!(
            md.contains("\n  - [x] Done one\n"),
            "subtask under its parent: {md}"
        );

        // Contacts.
        let vcf = std::fs::read_to_string(root.join("Contacts/Contacts.vcf")).unwrap();
        let cards = parse(&vcf, "VCARD");
        assert_eq!(cards.len(), 1);
        assert_eq!(one(&cards[0], "FN"), "Zoya Ahuja");
        assert_eq!(one(&cards[0], "ORG"), "Acme, Inc.");
        assert_eq!(one(&cards[0], "TEL"), "+91 90000 00001");
        assert!(
            vcf.contains("TEL;TYPE=CELL:"),
            "Apple's Mobile label as vCard's CELL"
        );
        assert_eq!(one(&cards[0], "EMAIL"), "zoya@example.com");
        assert_eq!(one(&cards[0], "ADR"), ";;1 Long Road\nFlat 2;Pune;;;India");
        assert_eq!(one(&cards[0], "NOTE"), "met at; the park");

        // Calendar: every event, from a calendar hidden on screen too.
        let cal = std::fs::read_to_string(root.join("Calendar/Work.ics")).unwrap();
        let events = parse(&cal, "VEVENT");
        assert_eq!(events.len(), 2);
        let review = events.iter().find(|e| one(e, "UID") == "E1").unwrap();
        assert!(one(review, "SUMMARY").ends_with("folded across lines in the file"));
        assert_eq!(one(review, "DESCRIPTION"), "Bring the deck;\nand coffee");
        assert_eq!(one(review, "DTSTART"), "20260921T141320Z");
        let holiday = events.iter().find(|e| one(e, "UID") == "E2").unwrap();
        assert_eq!(one(holiday, "DTSTART"), "20260921");
        assert_eq!(one(holiday, "DTEND"), "20260922");

        // Drive: the listing whole, the cached file only.
        let csv = std::fs::read_to_string(root.join("iCloud Drive/Listing.csv")).unwrap();
        assert!(csv.contains("\"Docs/CV, final.pdf\",File,5,"), "{csv}");
        assert!(csv.contains("Docs/Big.mov,File,10000000,,,No"), "{csv}");
        assert_eq!(
            std::fs::read(root.join("iCloud Drive/Files/Docs/CV, final.pdf")).unwrap(),
            b"%PDF-"
        );
        assert!(!root.join("iCloud Drive/Files/Docs/Big.mov").exists());

        let readme = std::fs::read_to_string(root.join("README.txt")).unwrap();
        assert!(readme.contains("Wednesday 21 October 2026"), "{readme}");
        assert!(readme.contains("Repeating events are written as stored"));
        assert!(readme.contains("1 locked note is not included"));
        assert!(readme.contains("Photos          Not included"));
        assert!(!root.join("Photos").exists(), "photos only when asked for");
        assert!(!root.join(MARKER).exists());
        // Nothing in the data folder was touched.
        assert!(!dir.path().join("photos").exists());
    }

    #[test]
    fn photos_by_month_and_what_was_not_cached_listed() {
        let (dir, mut lib) = library();
        lib.put_photos(&[
            photo("P1", 1_790_000_000, "IMG_0001.HEIC"),
            photo("P2", 1_700_000_000, "IMG_0002.JPG"),
            photo("P3", 1_700_000_000, "IMG_0003.JPG"),
        ])
        .unwrap();
        let d = dir.path().join("photos");
        std::fs::create_dir_all(d.join("original")).unwrap();
        std::fs::create_dir_all(d.join("full")).unwrap();
        std::fs::write(d.join("original/P1"), b"\0\0\0\x18ftypheic....").unwrap();
        // Only the whole JPEG of this one: named as what it is.
        std::fs::write(d.join("full/P2"), b"\xFF\xD8\xFF\xE0jpeg").unwrap();
        let (root, s, _out) = export(dir.path(), &lib, true);
        assert_eq!((s.photos, s.photos_missing), (2, 1));
        assert!(root.join("Photos/2026/09/IMG_0001.HEIC").is_file());
        assert!(root.join("Photos/2023/11/IMG_0002.JPG").is_file());
        let missing = std::fs::read_to_string(root.join("Photos/Not Included.txt")).unwrap();
        assert!(missing.contains("IMG_0003.JPG") && !missing.contains("IMG_0002"));
    }

    #[test]
    fn a_cancel_leaves_nothing_behind() {
        let (dir, lib) = library();
        let out = tempfile::tempdir().unwrap();
        let snap = gather(&lib, false).unwrap();
        let cancel = AtomicBool::new(false);
        let got = run(
            &snap,
            &Sources::at(dir.path()),
            out.path(),
            &now(),
            &cancel,
            &mut |p| {
                if p.service == "contacts" {
                    cancel.store(true, Ordering::Relaxed);
                }
            },
        );
        assert_eq!(got, Err(Stop::Cancelled));
        assert_eq!(
            std::fs::read_dir(out.path()).unwrap().count(),
            0,
            "no partial left"
        );
    }

    #[test]
    fn a_crashed_partial_is_cleared_and_only_ours() {
        let (dir, lib) = library();
        let out = tempfile::tempdir().unwrap();
        std::fs::create_dir(out.path().join(".partial-1")).unwrap();
        std::fs::write(out.path().join(".partial-1").join(MARKER), b"").unwrap();
        std::fs::create_dir(out.path().join(".partial-theirs")).unwrap();
        let snap = gather(&lib, false).unwrap();
        run(
            &snap,
            &Sources::at(dir.path()),
            out.path(),
            &now(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(!out.path().join(".partial-1").exists());
        assert!(
            out.path().join(".partial-theirs").exists(),
            "not ours: left alone"
        );
        // A second export the same minute is put beside the first, never over it.
        run(
            &snap,
            &Sources::at(dir.path()),
            out.path(),
            &now(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(out
            .path()
            .join("Skykeep Export 2026-10-21 10.30 2")
            .is_dir());
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(file_name("a/b\\c:d", "x"), "abcd");
        assert_eq!(file_name("../..", "Untitled"), "Untitled");
        assert_eq!(file_name("", "Untitled"), "Untitled");
        assert_eq!(file_name(".hidden", "x"), "hidden");
        assert_eq!(file_name("CON", "x"), "_CON");
        assert_eq!(file_name("exe\u{202E}fdp.txt", "x"), "exefdp.txt");
        assert_eq!(file_name(&"long ".repeat(60), "x").chars().count(), 119);
        let mut n = Names::default();
        let d = Path::new("/x");
        assert_eq!(n.unique(d, "Note", "md"), "Note.md");
        assert_eq!(
            n.unique(d, "note", "md"),
            "note 2.md",
            "case-insensitively distinct"
        );
        assert_eq!(n.unique(Path::new("/y"), "Note", "md"), "Note.md");
    }

    #[test]
    fn refuses_the_data_folder_icloud_folders_and_system_folders() {
        let data = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let drive = home.path().join("iCloud Drive");
        std::fs::create_dir_all(drive.join("sub")).unwrap();
        std::fs::create_dir_all(data.path().join("photos")).unwrap();
        let synced = [drive.clone()];
        assert!(check_dest(home.path(), data.path(), &synced).is_ok());
        assert!(
            check_dest(&data.path().join("photos"), data.path(), &synced)
                .unwrap_err()
                .contains("Skykeep’s own data")
        );
        assert!(check_dest(&drive.join("sub"), data.path(), &synced)
            .unwrap_err()
            .contains("in iCloud"));
        assert!(check_dest(Path::new("relative"), data.path(), &synced).is_err());
        assert!(check_dest(&home.path().join("gone"), data.path(), &synced).is_err());
        #[cfg(unix)]
        {
            for sys in ["/", "/etc", "/usr/share", "/var/lib", "/proc"] {
                assert!(
                    check_dest(Path::new(sys), data.path(), &synced)
                        .unwrap_err()
                        .contains("system folder"),
                    "{sys}"
                );
            }
            assert!(!is_system(Path::new("/run/media/me/USB")));
            assert!(!is_system(Path::new("/var/home/me")));
        }
    }

    #[test]
    fn folding_never_splits_a_character() {
        let mut out = String::new();
        line(&mut out, &format!("SUMMARY:{}", "é".repeat(80)));
        assert!(out.split("\r\n").all(|l| l.len() <= 75));
        let back: String = out
            .split("\r\n")
            .enumerate()
            .map(|(i, l)| {
                if i == 0 {
                    l
                } else {
                    l.strip_prefix(' ').unwrap_or(l)
                }
            })
            .collect();
        assert_eq!(back, format!("SUMMARY:{}", "é".repeat(80)));
    }
}
