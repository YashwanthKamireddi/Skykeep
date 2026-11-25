//! One search across everything the library holds — notes, reminders, events, contacts,
//! files and photos — offline and at once, as neither icloud.com nor a Mac offers: each app
//! searches only itself there.
//!
//! Words are looked up in one full-text index (migration 45, kept current by triggers in the
//! same transaction as every write), so a search reads only what holds its words rather than
//! reading and folding every row: every word has to begin a word of the record, in any order,
//! case and accents aside — as Spotlight and Apple's own apps find things. Within an app the
//! best come first: a title that begins with what was typed, then a title that holds it all,
//! then the rest; ties go to the most recent (a note edited lately, a reminder due soonest, an
//! event nearest now) and contacts to their place in the address book. Photos are searched as
//! Photos searches them, by the names of the dates they were taken too, which no index holds.
//!
//! What crosses into the window is small: per app a few hits, each a title and one line of
//! detail with where the typed words fall in them — never a note's text.

use crate::{
    calendar::Event,
    contacts::{Contact, ContactMethod},
    drive::DriveItem,
    notes::Note,
    photos::Photo,
    reminders::Reminder,
    Library, Result,
};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

/// The apps, in the sidebar's order, by the key a screen and a reveal use.
pub const APPS: [&str; 6] = [
    "notes",
    "reminders",
    "calendar",
    "contacts",
    "drivews",
    "photos",
];
/// What the index calls each app's records, in the order of [`APPS`].
const KINDS: [&str; 6] = ["note", "reminder", "event", "contact", "file", "photo"];
/// The most words of a query looked for; past it, the rest change nothing worth the time.
const MOST_WORDS: usize = 12;

/// What one search found, the first few of each kind, and how many of each there were.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Everything {
    pub notes: Vec<Note>,
    pub reminders: Vec<Reminder>,
    pub events: Vec<Event>,
    pub contacts: Vec<Contact>,
    pub files: Vec<DriveItem>,
    pub photos: Vec<Photo>,
    /// How many each kind found in all (up to what was looked through), in the order above.
    pub counts: [usize; 6],
}

/// One thing found, as the window lists it: what it is called, one line more, and where the
/// typed words fall in each — offsets in UTF-16 code units, as JavaScript counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    /// The record: a note's or reminder's guid, a file's drivewsid, a photo's id.
    pub id: String,
    /// What holds it, for opening it there: a reminder's list, an event's calendar, a file's
    /// folder.
    pub within: Option<String>,
    pub title: String,
    pub title_marks: Vec<[u32; 2]>,
    /// The line under the title: where in a note's text the words are, a reminder's notes, an
    /// event's place, a contact's number or address, a file's kind.
    pub detail: Option<String>,
    pub detail_marks: Vec<[u32; 2]>,
    /// The time shown with it, Unix seconds: due, starting, modified or taken.
    pub when: Option<i64>,
    pub all_day: bool,
    /// It has a picture of its own (a contact's photo, a photo's thumbnail).
    pub picture: bool,
    /// A locked note: its text is not shown.
    pub locked: bool,
}

/// One app's part of what was found.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// The app, as [`APPS`] names it.
    pub app: &'static str,
    /// How many it found in all.
    pub count: usize,
    pub hits: Vec<Hit>,
}

/// The words of `query` as the index is asked for them; empty when there is nothing to look
/// for. A word without a letter or digit in it ("-", "&") would match nothing and is dropped.
fn query_words(query: &str) -> Vec<&str> {
    query
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .take(MOST_WORDS)
        .collect()
}

/// A word as an FTS5 prefix phrase. Quoted, so nothing typed is ever read as the query
/// language; a word with punctuation inside ("don't", "a.pdf") is a phrase of its parts.
fn phrase(word: &str) -> String {
    format!("\"{}\"*", word.replace('"', "\"\""))
}

/// The three questions a search asks the index: every word anywhere; every word in the title
/// with the first beginning it; every word in the title.
struct Questions {
    all: String,
    starts: String,
    in_title: String,
}

fn questions(query: &str) -> Option<Questions> {
    let words = query_words(query);
    if words.is_empty() {
        return None;
    }
    let all = words
        .iter()
        .map(|w| phrase(w))
        .collect::<Vec<_>>()
        .join(" ");
    Some(Questions {
        starts: format!("{{title}} : (^ {all})"),
        in_title: format!("{{title}} : ({all})"),
        all,
    })
}

/// The FTS5 query for every word of `query`, each as a prefix; `None` when there is nothing to
/// look for. What Notes' own search asks too.
pub(crate) fn match_all(query: &str) -> Option<String> {
    questions(query).map(|q| q.all)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A character inside a word, as the index's tokenizer has it: letters, digits, and the
/// marks that sit on a letter (the vowel signs of Indic scripts, an accent written apart).
fn in_word(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c,
            '\u{0300}'..='\u{036F}'
            | '\u{0483}'..='\u{0489}'
            | '\u{0591}'..='\u{05C7}'
            | '\u{0610}'..='\u{061A}'
            | '\u{064B}'..='\u{065F}'
            | '\u{0900}'..='\u{0963}'
            | '\u{0966}'..='\u{0DFF}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}')
}

/// The words of `text` with their byte ranges.
fn words_of(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut start = None;
    let mut chars = text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')));
    std::iter::from_fn(move || {
        for (i, c) in chars.by_ref() {
            match (start, in_word(c) && i < text.len()) {
                (None, true) => start = Some(i),
                (Some(s), false) => {
                    start = None;
                    return Some((s, &text[s..i]));
                }
                _ => {}
            }
        }
        None
    })
}

/// The typed words, folded and split as the index splits them.
pub(crate) fn folded_words(query: &str) -> Vec<String> {
    let mut out: Vec<String> = query_words(query)
        .into_iter()
        .flat_map(|w| {
            words_of(w)
                .map(|(_, p)| crate::fold::fold(p))
                .collect::<Vec<_>>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    // Longest first, so "spring" lights all of "Springfield" where "s" and "spring" both begin it.
    out.sort_by_key(|w| std::cmp::Reverse(w.len()));
    out
}

/// How many bytes of `word` it takes for its folded form to reach `len` bytes; `None` when
/// the folded word does not begin with `prefix`.
fn prefix_end(word: &str, prefix: &str) -> Option<usize> {
    let mut folded = String::new();
    for (i, c) in word.char_indices() {
        if folded.len() >= prefix.len() {
            return folded.starts_with(prefix).then_some(i);
        }
        let mut buf = [0u8; 4];
        folded.push_str(&crate::fold::fold(c.encode_utf8(&mut buf)));
    }
    folded.starts_with(prefix).then_some(word.len())
}

fn utf16(text: &str, byte: usize) -> u32 {
    u32::try_from(text[..byte].encode_utf16().count()).unwrap_or(u32::MAX)
}

/// Where in `text` a word begins with one of `words`, as UTF-16 ranges.
fn marks(text: &str, words: &[String]) -> Vec<[u32; 2]> {
    let mut out = Vec::new();
    if words.is_empty() {
        return out;
    }
    for (at, word) in words_of(text) {
        if let Some(end) = words.iter().find_map(|w| prefix_end(word, w)) {
            if end > 0 {
                out.push([utf16(text, at), utf16(text, at + end)]);
            }
        }
    }
    out
}

/// One line of `text` around the first place one of `words` begins a word: from a word
/// boundary a little before it, its spaces and line breaks run together. The start of the
/// text when no word is in it.
fn excerpt(text: &str, words: &[String]) -> Option<String> {
    const BEFORE: usize = 32;
    const LONG: usize = 160;
    let hit = words_of(text)
        .find(|(_, word)| words.iter().any(|w| prefix_end(word, w).is_some()))
        .map_or(0, |(at, _)| at);
    // Back a few words from the hit, never mid-word.
    let mut start = hit;
    let mut back = 0;
    for (i, c) in text[..hit].char_indices().rev() {
        back += 1;
        if back > BEFORE {
            break;
        }
        if c == '\n' {
            start = i + 1;
            break;
        }
        if c.is_whitespace() {
            start = i + 1;
        }
    }
    let line: String = text[start..]
        .chars()
        .filter(|&c| c != '\u{FFFC}')
        .take(LONG * 2)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(LONG)
        .collect();
    (!line.is_empty()).then_some(line)
}

/// The line a found note shows under its title, and where the typed words fall in it: from
/// the note's text past its title line, around the first word found there, as Notes shows a
/// search's results; its own preview when the words are only in the title.
pub(crate) fn note_line(
    title: &str,
    body: Option<String>,
    snippet: Option<String>,
    words: &[String],
) -> (Option<String>, Vec<[u32; 2]>) {
    let line = |text: Option<String>| -> (Option<String>, Vec<[u32; 2]>) {
        let line = text.as_deref().and_then(|t| excerpt(t, words));
        let m = line.as_deref().map(|l| marks(l, words)).unwrap_or_default();
        (line, m)
    };
    // The first line of a note is its title: the line shown is from the rest.
    let rest = body.map(|b| match b.split_once('\n') {
        Some((first, rest)) if first.trim() == title.trim() => rest.to_string(),
        _ => b,
    });
    let (d, m) = line(rest.filter(|r| !r.trim().is_empty()));
    if d.is_some() {
        (d, m)
    } else {
        line(snippet)
    }
}

/// The ranking query: what `ranked` asks, written once.
fn ranking_sql() -> &'static str {
    static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SQL.get_or_init(|| {
        format!(
            "WITH hits AS (
                 SELECT d.kind, d.key, d.at, d.sort,
                        CASE WHEN search_text.rowid IN
                                  (SELECT rowid FROM search_text WHERE search_text MATCH ?2) THEN 0
                             WHEN search_text.rowid IN
                                  (SELECT rowid FROM search_text WHERE search_text MATCH ?3) THEN 1
                             ELSE 2 END AS tier
                 FROM search_text JOIN search_docs d ON d.id = search_text.rowid
                 WHERE search_text MATCH ?1 AND (?4 IS NULL OR d.kind = ?4)
                   AND (d.kind != 'reminder' OR EXISTS (
                        SELECT 1 FROM reminders r JOIN reminder_lists l ON l.guid = r.list_guid
                        WHERE r.guid = d.key AND l.enabled = 1))
                   AND (d.kind != 'photo' OR EXISTS (
                        SELECT 1 FROM photos WHERE photos.id = d.key AND {burst}))
             ),
             ranked AS (
                 SELECT kind, key,
                        row_number() OVER (PARTITION BY kind ORDER BY tier,
                            CASE kind WHEN 'reminder' THEN coalesce(at, 9223372036854775807)
                                      WHEN 'event' THEN abs(coalesce(at, 0) - ?5)
                                      ELSE -coalesce(at, 0) END,
                            sort, key) AS place,
                        count(*) OVER (PARTITION BY kind) AS total
                 FROM hits
             )
             SELECT kind, key, total FROM ranked WHERE place > ?7 AND place <= ?7 + ?6
             ORDER BY kind, place",
            burst = crate::photos::ONE_PER_BURST
        )
    })
}

impl Library {
    /// The keys each app found for `query`, best first — `each` of a kind past the first
    /// `skip` — and how many each found in all. `only` keeps to one app.
    fn ranked(
        &self,
        q: &Questions,
        each: usize,
        skip: usize,
        only: Option<usize>,
    ) -> Result<[(usize, Vec<String>); 6]> {
        let mut stmt = self.conn.prepare_cached(ranking_sql())?;
        let kind = only.and_then(|i| KINDS.get(i).copied());
        let n = |v: usize| i64::try_from(v).unwrap_or(i64::MAX / 2);
        let rows = stmt.query_map(
            params![q.all, q.starts, q.in_title, kind, now(), n(each), n(skip)],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )?;
        let mut out: [(usize, Vec<String>); 6] = Default::default();
        for row in rows {
            let (kind, key, total) = row?;
            if let Some(i) = KINDS.iter().position(|k| *k == kind) {
                out[i].0 = usize::try_from(total).unwrap_or(0);
                out[i].1.push(key);
            }
        }
        Ok(out)
    }

    /// What `query` finds, app by app in the sidebar's order — only the apps that found
    /// something — at most `each` of an app, best first. `only` (an app of [`APPS`]) asks for
    /// that one app alone, from its `skip`th on: "Show All", and the rows past those as they
    /// are scrolled to.
    pub fn search(
        &self,
        query: &str,
        each: usize,
        only: Option<&str>,
        skip: usize,
    ) -> Result<Vec<Group>> {
        let Some(q) = questions(query) else {
            return Ok(Vec::new());
        };
        let words = folded_words(query);
        let only = match only {
            Some(app) => match APPS.iter().position(|x| *x == app) {
                Some(i) => Some(i),
                None => return Ok(Vec::new()),
            },
            None => None,
        };
        let skip = if only.is_some() { skip } else { 0 };
        let mut groups = Vec::new();
        for (i, (count, keys)) in self.ranked(&q, each, skip, only)?.into_iter().enumerate() {
            if count == 0 {
                continue;
            }
            let mut hits = Vec::with_capacity(keys.len());
            for key in &keys {
                hits.extend(self.hit(i, key, &words)?);
            }
            groups.push(Group {
                app: APPS[i],
                count,
                hits,
            });
        }
        Ok(groups)
    }

    /// One found record of app `app`, as the window lists it.
    fn hit(&self, app: usize, key: &str, words: &[String]) -> Result<Option<Hit>> {
        let conn = &self.conn;
        let line = |text: Option<String>| -> (Option<String>, Vec<[u32; 2]>) {
            let line = text.as_deref().and_then(|t| excerpt(t, words));
            let m = line.as_deref().map(|l| marks(l, words)).unwrap_or_default();
            (line, m)
        };
        let hit = match app {
            0 => conn
                .prepare_cached(
                    "SELECT title, locked, modified_at, body, snippet FROM notes WHERE guid = ?1",
                )?
                .query_row(params![key], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)? != 0,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })
                .optional()?
                .map(|(title, locked, at, body, snippet)| {
                    let (detail, detail_marks) = if locked {
                        (None, Vec::new())
                    } else {
                        note_line(&title, body, snippet, words)
                    };
                    Hit {
                        id: key.into(),
                        title_marks: marks(&title, words),
                        title,
                        detail,
                        detail_marks,
                        when: at,
                        locked,
                        ..Hit::default()
                    }
                }),
            1 => conn
                .prepare_cached(
                    "SELECT list_guid, title, notes, due_at, all_day FROM reminders WHERE guid = ?1",
                )?
                .query_row(params![key], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, i64>(4)? != 0,
                    ))
                })
                .optional()?
                .map(|(list, title, notes, due, all_day)| {
                    let (detail, detail_marks) = line(notes);
                    Hit {
                        id: key.into(),
                        within: Some(list),
                        title_marks: marks(&title, words),
                        title,
                        detail,
                        detail_marks,
                        when: due,
                        all_day,
                        ..Hit::default()
                    }
                }),
            2 => conn
                .prepare_cached(
                    "SELECT calendar_guid, title, location, notes, starts_at, all_day, floating
                     FROM events WHERE guid = ?1",
                )?
                .query_row(params![key], |r| {
                    let at = r.get::<_, i64>(4)?;
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        // A floating event's stored time is its wall clock: read in this zone.
                        if r.get::<_, i64>(6)? != 0 {
                            crate::calendar::instant_of(at, &self.local_zone())
                        } else {
                            at
                        },
                        r.get::<_, i64>(5)? != 0,
                    ))
                })
                .optional()?
                .map(|(calendar, title, location, notes, at, all_day)| {
                    let place = location.filter(|l| !l.trim().is_empty());
                    let (detail, detail_marks) = line(place.or(notes));
                    Hit {
                        id: key.into(),
                        within: Some(calendar),
                        title_marks: marks(&title, words),
                        title,
                        detail,
                        detail_marks,
                        when: Some(at),
                        all_day,
                        ..Hit::default()
                    }
                }),
            3 => self.contacts_by_guid(&[key])?.pop().map(|c| {
                let title = c.display_name();
                // The number or address the words were in, or else the first one.
                let method = c
                    .methods
                    .iter()
                    .find(|m| !marks(&m.value, words).is_empty())
                    .or_else(|| c.methods.first())
                    .map(|m| m.value.clone())
                    .or_else(|| c.company.clone().filter(|_| !c.is_company));
                let detail_marks = method
                    .as_deref()
                    .map(|m| marks(m, words))
                    .unwrap_or_default();
                Hit {
                    id: key.into(),
                    title_marks: marks(&title, words),
                    title,
                    detail: method,
                    detail_marks,
                    picture: c.photo_url.is_some(),
                    ..Hit::default()
                }
            }),
            4 => conn
                .prepare_cached(
                    "SELECT parent_id, name, extension, modified_at FROM drive_items
                     WHERE drivewsid = ?1",
                )?
                .query_row(params![key], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                })
                .optional()?
                .map(|(parent, name, ext, at)| {
                    let title = match ext.filter(|e| !e.is_empty()) {
                        Some(e) => format!("{name}.{e}"),
                        None => name,
                    };
                    Hit {
                        id: key.into(),
                        within: parent,
                        title_marks: marks(&title, words),
                        title,
                        when: at,
                        all_day: true,
                        ..Hit::default()
                    }
                }),
            5 => conn
                .prepare_cached("SELECT title, filename, taken_at FROM photos WHERE id = ?1")?
                .query_row(params![key], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .optional()?
                .map(|(title, filename, at)| {
                    let title = title
                        .filter(|t| !t.trim().is_empty())
                        .or(filename)
                        .unwrap_or_else(|| "Photo".into());
                    Hit {
                        id: key.into(),
                        title_marks: marks(&title, words),
                        title,
                        when: Some(at),
                        all_day: true,
                        picture: true,
                        ..Hit::default()
                    }
                }),
            _ => None,
        };
        Ok(hit)
    }

    /// Contacts by guid, with their numbers and addresses, in the order asked.
    fn contacts_by_guid(&self, guids: &[&str]) -> Result<Vec<Contact>> {
        let mut out = Vec::with_capacity(guids.len());
        let mut person = self.conn.prepare_cached(
            "SELECT guid, first_name, middle_name, last_name, company, is_company, birthday,
                    notes, photo_url, photo_signature, photo_crop, etag
             FROM contacts WHERE guid = ?1",
        )?;
        let mut methods = self.conn.prepare_cached(
            "SELECT kind, label, value FROM contact_methods WHERE contact_guid = ?1
             ORDER BY position",
        )?;
        for guid in guids {
            let found = person
                .query_row(params![guid], |r| {
                    Ok(Contact {
                        guid: r.get(0)?,
                        first_name: r.get(1)?,
                        middle_name: r.get(2)?,
                        last_name: r.get(3)?,
                        company: r.get(4)?,
                        is_company: r.get::<_, i64>(5)? != 0,
                        birthday: r.get(6)?,
                        notes: r.get(7)?,
                        photo_url: r.get(8)?,
                        photo_signature: r.get(9)?,
                        photo_crop: r.get(10)?,
                        etag: r.get(11)?,
                        methods: Vec::new(),
                    })
                })
                .optional()?;
            if let Some(mut c) = found {
                c.methods = methods
                    .query_map(params![guid], |r| {
                        Ok(ContactMethod {
                            kind: r.get(0)?,
                            label: r.get(1)?,
                            value: r.get(2)?,
                        })
                    })?
                    .collect::<rusqlite::Result<_>>()?;
                out.push(c);
            }
        }
        Ok(out)
    }

    /// Everything that holds every word of `query`, at most `each` of a kind, whole records:
    /// what the desktop's own search (GNOME Shell, KRunner) describes its results from.
    pub fn search_everything(&self, query: &str, each: usize) -> Result<Everything> {
        let Some(q) = questions(query) else {
            return Ok(Everything::default());
        };
        let [notes, reminders, events, contacts, files, photos] = self.ranked(&q, each, 0, None)?;
        let counts = [
            notes.0,
            reminders.0,
            events.0,
            contacts.0,
            files.0,
            photos.0,
        ];
        let mut found = Everything {
            counts,
            ..Everything::default()
        };
        for key in &notes.1 {
            found.notes.extend(self.note(key)?);
        }
        for key in &reminders.1 {
            found.reminders.extend(self.reminder(key)?);
        }
        for key in &events.1 {
            found.events.extend(self.event(key)?);
        }
        let keys: Vec<&str> = contacts.1.iter().map(String::as_str).collect();
        found.contacts = self.contacts_by_guid(&keys)?;
        for key in &files.1 {
            found.files.extend(self.drive_item(key)?);
        }
        for key in &photos.1 {
            found.photos.extend(self.photo(key)?);
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{NoteFolder, TRASH_FOLDER};

    fn file(id: &str, name: &str, modified: i64) -> DriveItem {
        DriveItem {
            drivewsid: id.into(),
            parent_id: Some("root".into()),
            name: name.into(),
            extension: Some("pdf".into()),
            kind: "FILE".into(),
            size: Some(1),
            child_count: None,
            created_at: None,
            modified_at: Some(modified),
            etag: None,
            share: None,
        }
    }

    fn person(guid: &str, first: &str, last: &str, phone: Option<&str>) -> Contact {
        Contact {
            guid: guid.into(),
            first_name: Some(first.into()),
            middle_name: None,
            last_name: Some(last.into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            etag: None,
            methods: phone
                .map(|p| {
                    vec![ContactMethod {
                        kind: "phone".into(),
                        label: Some("mobile".into()),
                        value: p.into(),
                    }]
                })
                .unwrap_or_default(),
        }
    }

    fn note(guid: &str, folder: &str, title: &str, body: &str, modified: i64) -> Note {
        Note {
            guid: guid.into(),
            folder_guid: Some(folder.into()),
            title: title.into(),
            snippet: None,
            body: Some(body.into()),
            locked: false,
            editable: true,
            pinned: false,
            created_at: Some(1),
            modified_at: Some(modified),
            etag: None,
            runs: None,
        }
    }

    fn with_folders() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_note_folders(&[
            NoteFolder {
                guid: "F".into(),
                title: "Notes".into(),
                sort_order: 0,
                parent_guid: None,
            },
            NoteFolder {
                guid: TRASH_FOLDER.into(),
                title: "Recently Deleted".into(),
                sort_order: 1,
                parent_guid: None,
            },
        ])
        .unwrap();
        lib
    }

    fn ids(groups: &[Group], app: &str) -> Vec<String> {
        groups
            .iter()
            .find(|g| g.app == app)
            .map(|g| g.hits.iter().map(|h| h.id.clone()).collect())
            .unwrap_or_default()
    }

    /// One word finds it wherever it is — a file and a contact here — case and accents aside;
    /// a word nothing holds finds nothing; each kind is kept to its few.
    #[test]
    fn one_search_finds_it_in_every_app() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_drive_children(
            "root",
            &[
                file("F1", "Résumé Springfield", 1),
                file("F2", "Springfield map", 2),
                file("F3", "Budget", 3),
            ],
        )
        .unwrap();
        lib.put_contacts(&[person("C1", "Asha", "Springfield", None)])
            .unwrap();
        let found = lib.search_everything("SPRINGFIELD", 1).unwrap();
        assert_eq!(found.counts[4], 2, "both files found");
        assert_eq!(found.files.len(), 1, "and only the first kept");
        assert_eq!(found.contacts.len(), 1);
        assert_eq!(
            lib.search_everything("resume springfield", 5)
                .unwrap()
                .files
                .len(),
            1,
            "accents aside, every word"
        );
        assert_eq!(lib.search_everything("zebra", 5).unwrap().counts, [0; 6]);
        assert_eq!(lib.search_everything("   ", 5).unwrap().counts, [0; 6]);
        assert!(lib.search("  -  ", 5, None, 0).unwrap().is_empty());
    }

    /// A word is found by how it begins, whatever the case or accents on either side, and in
    /// scripts with marks of their own; quotes and the query language's words are only words.
    #[test]
    fn prefixes_accents_and_scripts() {
        let mut lib = with_folders();
        lib.put_notes(&[
            note("A", "F", "Café list", "Flat white, then a croissant", 1),
            note("B", "F", "తెలుగు", "Notes written in Telugu", 2),
            note(
                "C",
                "F",
                "Plans",
                "NEAR the \"old\" OR new mill; don't forget",
                3,
            ),
            note("D", "F", "Ελληνικά", "ΣΗΜΕΙΩΣΕΙΣ", 4),
        ])
        .unwrap();
        let notes = |q: &str| ids(&lib.search(q, 10, None, 0).unwrap(), "notes");
        assert_eq!(notes("caf"), vec!["A"]);
        assert_eq!(notes("CAFÉ"), vec!["A"]);
        assert_eq!(notes("croiss"), vec!["A"]);
        assert_eq!(notes("తెలు"), vec!["B"]);
        assert_eq!(notes("σημει"), vec!["D"], "case folded beyond ASCII");
        assert_eq!(notes("\"old\""), vec!["C"]);
        assert_eq!(notes("NEAR OR"), vec!["C"]);
        assert_eq!(notes("don't"), vec!["C"]);
        assert!(
            notes("roissant").is_empty(),
            "words are found by how they begin"
        );
        assert!(notes("flat croissant zebra").is_empty(), "every word");
    }

    /// A title that begins with what was typed first, then one that holds it, then the rest;
    /// most recent first among equals.
    #[test]
    fn titles_first_then_recency() {
        let mut lib = with_folders();
        lib.put_notes(&[
            note("body-new", "F", "Groceries", "garden tools", 9),
            note("body-old", "F", "Errands", "garden hose", 1),
            note("title-in", "F", "Old garden", "", 2),
            note("title-starts", "F", "Garden plan", "", 3),
        ])
        .unwrap();
        assert_eq!(
            ids(&lib.search("gard", 10, None, 0).unwrap(), "notes"),
            vec!["title-starts", "title-in", "body-new", "body-old"]
        );
        let found = lib.search("gard", 2, None, 0).unwrap();
        assert_eq!(found[0].count, 4, "the count is of all, the hits a few");
        assert_eq!(found[0].hits.len(), 2);
        assert_eq!(
            ids(&lib.search("gard", 10, Some("notes"), 0).unwrap(), "notes").len(),
            4
        );
        assert!(lib
            .search("gard", 10, Some("drivews"), 0)
            .unwrap()
            .is_empty());
        assert_eq!(
            ids(&lib.search("gard", 2, Some("notes"), 2).unwrap(), "notes"),
            vec!["body-new", "body-old"],
            "the rows past the first, as they are scrolled to"
        );
    }

    /// The index follows every write: a record stored, changed, moved where search does not
    /// look (Recently Deleted, done), and deleted — and a whole sync pass rolled back.
    #[test]
    fn the_index_follows_every_write() {
        let mut lib = with_folders();
        let notes = |lib: &Library, q: &str| ids(&lib.search(q, 10, None, 0).unwrap(), "notes");
        lib.put_notes(&[note("N", "F", "Trip", "passport and tickets", 1)])
            .unwrap();
        assert_eq!(notes(&lib, "passport"), vec!["N"]);
        lib.put_notes(&[note("N", "F", "Trip", "visa and tickets", 2)])
            .unwrap();
        assert!(notes(&lib, "passport").is_empty(), "the old words go");
        assert_eq!(notes(&lib, "visa"), vec!["N"]);
        lib.put_notes(&[note("N", TRASH_FOLDER, "Trip", "visa", 3)])
            .unwrap();
        assert!(
            notes(&lib, "visa").is_empty(),
            "Recently Deleted is not searched"
        );
        lib.put_notes(&[note("N", "F", "Trip", "visa", 4)]).unwrap();
        assert_eq!(notes(&lib, "visa"), vec!["N"], "and back out of it");
        lib.forget_notes(&["N".into()]).unwrap();
        assert!(notes(&lib, "visa").is_empty(), "a deleted note is gone");

        // A pass that fails part way leaves the index as the records: as it was.
        let failed: std::result::Result<(), crate::Error> = lib.atomically(|lib| {
            lib.put_notes(&[note("M", "F", "Harbour", "", 5)])?;
            Err(crate::Error::Io(std::io::Error::other("network")))
        });
        assert!(failed.is_err());
        assert!(notes(&lib, "harbour").is_empty());

        // A contact's numbers, as typed or digits alone; a removed number no longer finds them.
        lib.put_contacts(&[person("P", "Zoya", "Khan", Some("+91 98765-43210"))])
            .unwrap();
        let people = |lib: &Library, q: &str| ids(&lib.search(q, 10, None, 0).unwrap(), "contacts");
        assert_eq!(people(&lib, "98765 432"), vec!["P"]);
        assert_eq!(people(&lib, "9876543210"), vec!["P"]);
        assert_eq!(people(&lib, "zoy kh"), vec!["P"]);
        lib.put_contacts(&[person("P", "Zoya", "Khan", None)])
            .unwrap();
        assert!(people(&lib, "98765").is_empty());
        lib.put_contacts(&[]).unwrap();
        assert!(people(&lib, "zoya").is_empty());

        // Forgetting the account forgets what it found.
        lib.put_drive_children("root", &[file("F1", "Ledger", 1)])
            .unwrap();
        assert_eq!(
            ids(&lib.search("led", 5, None, 0).unwrap(), "drivews"),
            vec!["F1"]
        );
        lib.forget_account().unwrap();
        assert!(lib.search("led", 5, None, 0).unwrap().is_empty());
        lib.put_drive_children("root", &[file("F1", "Ledger", 1)])
            .unwrap();
        assert_eq!(
            ids(&lib.search("led", 5, None, 0).unwrap(), "drivews"),
            vec!["F1"],
            "and indexes the next one's"
        );
    }

    /// What crosses into the window: no text of a note but a line around the words, and where
    /// the words fall, counted as JavaScript counts (UTF-16).
    #[test]
    fn small_hits_with_marks() {
        let mut lib = with_folders();
        let long = format!(
            "Packing\n{}the passport is in the blue drawer{}",
            "filler words here ".repeat(200),
            " more".repeat(500)
        );
        lib.put_notes(&[note("N", "F", "Packing", &long, 1)])
            .unwrap();
        let hit = &lib.search("passp", 5, None, 0).unwrap()[0].hits[0];
        let line = hit.detail.as_deref().unwrap();
        assert!(line.len() <= 640 && line.contains("passport"));
        assert!(!line.contains('\n'));
        let [a, b] = hit.detail_marks[0];
        let units: Vec<u16> = line.encode_utf16().collect();
        assert_eq!(
            String::from_utf16(&units[a as usize..b as usize]).unwrap(),
            "passp"
        );
        assert_eq!(marks("😀 Émile", &folded_words("emi")), vec![[3, 6]]);
        assert_eq!(
            marks("Springfield", &folded_words("s spring")),
            vec![[0, 6]]
        );
    }

    /// Photos by title, file name, caption, keyword, or the year and month they were taken;
    /// hidden and deleted ones are not found; a burst is found as its chosen photo.
    #[test]
    fn photos_by_words_and_dates() {
        let shot = |id: &str, taken: i64| Photo {
            id: id.into(),
            master: format!("m-{id}"),
            taken_at: taken,
            added_at: Some(taken),
            kind: crate::photos::Kind::Image,
            width: 4,
            height: 3,
            duration: None,
            filename: Some(format!("IMG_{id}.HEIC")),
            size: Some(1),
            uti: None,
            favorite: false,
            hidden: false,
            deleted: false,
            deleted_at: None,
            subtype: 0,
            change_tag: None,
            expunge_at: None,
            title: None,
            caption: None,
            rating: 0,
            keywords: Vec::new(),
            latitude: None,
            longitude: None,
            rendered: 0,
            burst: None,
            burst_pick: false,
            burst_count: 0,
            live_size: None,
        };
        let mut lib = Library::in_memory().unwrap();
        // Mid-month, so no timezone moves them into another month.
        let march_2024 = 1_710_460_800;
        let mut beach = shot("1", march_2024);
        beach.caption = Some("Sunset at the beach".into());
        let mut hidden = shot("2", march_2024);
        hidden.caption = Some("beach".into());
        hidden.hidden = true;
        let mut tagged = shot("3", march_2024 + 86_400 * 120);
        tagged.keywords = vec!["Family".into()];
        let mut burst_a = shot("4", march_2024);
        burst_a.burst = Some("B".into());
        burst_a.caption = Some("Kite".into());
        let mut burst_b = burst_a.clone();
        burst_b.id = "5".into();
        burst_b.burst_pick = true;
        lib.put_photos(&[beach, hidden, tagged, burst_a, burst_b])
            .unwrap();
        let photos = |lib: &Library, q: &str| ids(&lib.search(q, 10, None, 0).unwrap(), "photos");
        assert_eq!(photos(&lib, "beach"), vec!["1"]);
        assert_eq!(photos(&lib, "fam"), vec!["3"]);
        assert_eq!(photos(&lib, "img_3"), vec!["3"]);
        assert_eq!(
            photos(&lib, "kite"),
            vec!["5"],
            "a burst as its chosen photo"
        );
        let mut march = photos(&lib, "march 2024");
        march.sort();
        assert_eq!(march, vec!["1", "5"]);
        lib.forget_photos(&["1".into()]).unwrap();
        assert!(photos(&lib, "beach").is_empty());
    }

    /// The search reader reads beside the library that writes: what was committed, at once,
    /// while a pass is still writing — never waiting for it, never seeing half of it.
    #[test]
    fn the_reader_reads_beside_a_pass_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open(dir.path()).unwrap();
        lib.put_drive_children("root", &[file("F1", "Ledger", 1)])
            .unwrap();
        let reader = Library::open_reader(dir.path()).unwrap();
        lib.conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        lib.put_drive_children(
            "root",
            &[file("F1", "Ledger", 1), file("F2", "Ledger two", 2)],
        )
        .unwrap();
        let t = std::time::Instant::now();
        let found = reader.search("ledger", 5, None, 0).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_millis(100));
        assert_eq!(found[0].count, 1, "the pass not yet committed is not seen");
        lib.conn.execute_batch("COMMIT").unwrap();
        assert_eq!(reader.search("ledger", 5, None, 0).unwrap()[0].count, 2);
        assert!(
            reader
                .conn
                .execute_batch("DELETE FROM drive_items")
                .is_err(),
            "it only reads"
        );
    }
}
