//! Calendars and events in the local library.
//!
//! Writes are upserts inside a transaction: a sync either lands whole or not at all, so an
//! interrupted refresh can never leave the interface showing half a calendar.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    pub guid: String,
    pub title: String,
    pub colour: Option<String>,
    /// Apple's collection tag. Unchanged means nothing inside moved.
    pub ctag: Option<String>,
    pub sort_order: i64,
    pub enabled: bool,
    /// False for a subscribed feed or a calendar shared for reading only. Nothing may be
    /// written to one, so nothing should offer to.
    #[serde(default = "yes")]
    pub writable: bool,
    /// The one Apple puts a new event in. Named outright rather than guessed at.
    #[serde(default)]
    pub is_default: bool,
}

const fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub guid: String,
    pub calendar_guid: String,
    pub title: String,
    pub location: Option<String>,
    /// Unix seconds, UTC.
    pub starts_at: i64,
    pub ends_at: i64,
    pub all_day: bool,
    pub timezone: Option<String>,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub etag: Option<String>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// The columns every event is read with, `floating` last; see [`Library::event_from_row`].
pub(crate) const EVENT_COLUMNS: &str =
    "guid, calendar_guid, title, location, starts_at, ends_at, all_day,
                    timezone, url, notes, etag, floating";

/// Whether an event is a day on the calendar or a time on the clock rather than an instant:
/// all-day, or timed with no zone (Calendar's "floating"). Kept as the wall-clock time, it is
/// read in whatever zone the computer is in when it is shown.
#[must_use]
pub fn floats(e: &Event) -> bool {
    e.all_day || e.timezone.is_none()
}

/// An instant as the wall-clock time it is in `zone`, counted as if that time were UTC.
#[must_use]
pub fn wall_clock(seconds: i64, zone: &jiff::tz::TimeZone) -> i64 {
    jiff::Timestamp::from_second(seconds).map_or(seconds, |t| {
        t.to_zoned(zone.clone())
            .datetime()
            .to_zoned(jiff::tz::TimeZone::UTC)
            .map_or(seconds, |z| z.timestamp().as_second())
    })
}

/// [`wall_clock`] turned back: the instant that wall-clock time is in `zone`. A time the
/// clocks skip is read as the hour after, as Calendar reads it.
#[must_use]
pub fn instant_of(wall: i64, zone: &jiff::tz::TimeZone) -> i64 {
    jiff::Timestamp::from_second(wall).map_or(wall, |t| {
        t.to_zoned(jiff::tz::TimeZone::UTC)
            .datetime()
            .to_zoned(zone.clone())
            .map_or(wall, |z| z.timestamp().as_second())
    })
}

/// [`EVENT_COLUMNS`], each as `table.column`.
fn prefixed(table: &str) -> String {
    EVENT_COLUMNS
        .split(',')
        .map(|c| format!("{table}{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Furthest any zone's clock is from UTC, and then some: the margin a window over stored
/// wall-clock times is widened by before each is read in the zone and judged exactly.
const WIDEST_OFFSET: i64 = 86_400;

impl Library {
    /// The zone floating events are read and stored in now.
    pub(crate) fn local_zone(&self) -> jiff::tz::TimeZone {
        self.zone.clone().unwrap_or_else(jiff::tz::TimeZone::system)
    }

    /// Read floating events as if this computer were in `zone`. Tests only, and a check.
    pub fn set_zone(&mut self, zone: Option<jiff::tz::TimeZone>) {
        self.zone = zone;
    }

    /// An event as stored, with a floating one's wall-clock times read in this zone now.
    pub(crate) fn event_from_row(
        row: &rusqlite::Row<'_>,
        zone: &jiff::tz::TimeZone,
    ) -> rusqlite::Result<Event> {
        let floating = row.get::<_, i64>(11)? != 0;
        let read = |at: i64| if floating { instant_of(at, zone) } else { at };
        Ok(Event {
            guid: row.get(0)?,
            calendar_guid: row.get(1)?,
            title: row.get(2)?,
            location: row.get(3)?,
            starts_at: read(row.get(4)?),
            ends_at: read(row.get(5)?),
            all_day: row.get::<_, i64>(6)? != 0,
            timezone: row.get(7)?,
            url: row.get(8)?,
            notes: row.get(9)?,
            etag: row.get(10)?,
        })
    }

    /// Replace the set of calendars, dropping any Apple no longer reports.
    ///
    /// Removal cascades to their events, which is what should happen when a calendar is
    /// deleted on another device.
    pub fn put_calendars(&mut self, calendars: &[Calendar]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO calendars (guid, title, colour, ctag, sort_order, enabled,
                                        writable, is_default, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title,
                   colour = excluded.colour,
                   ctag = excluded.ctag,
                   sort_order = excluded.sort_order,
                   enabled = excluded.enabled,
                   writable = excluded.writable,
                   is_default = excluded.is_default,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for c in calendars {
                stmt.execute(params![
                    c.guid,
                    c.title,
                    c.colour,
                    c.ctag,
                    c.sort_order,
                    i64::from(c.enabled),
                    i64::from(c.writable),
                    i64::from(c.is_default),
                    stamp
                ])?;
            }

            // Anything Apple stopped reporting was deleted elsewhere. A calendar of another
            // account was never Apple's to report (dav.rs).
            let keep: Vec<&str> = calendars.iter().map(|c| c.guid.as_str()).collect();
            if keep.is_empty() {
                tx.execute(
                    "DELETE FROM calendars WHERE substr(guid, 1, length(?1)) != ?1",
                    params![crate::dav::PREFIX],
                )?;
            } else {
                tx.execute(
                    "DELETE FROM calendars WHERE guid NOT IN (SELECT value FROM json_each(?1))
                       AND substr(guid, 1, length(?2)) != ?2",
                    params![crate::id_list(&keep), crate::dav::PREFIX],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Upsert events. Existing rows for the same guid are updated in place.
    pub fn put_events(&mut self, events: &[Event]) -> Result<()> {
        let zone = self.local_zone();
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO events (guid, calendar_guid, title, location, starts_at, ends_at,
                                     all_day, timezone, url, notes, etag, updated_at, floating)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(guid) DO UPDATE SET
                   calendar_guid = excluded.calendar_guid,
                   title = excluded.title,
                   location = excluded.location,
                   starts_at = excluded.starts_at,
                   ends_at = excluded.ends_at,
                   all_day = excluded.all_day,
                   timezone = excluded.timezone,
                   url = excluded.url,
                   notes = excluded.notes,
                   etag = excluded.etag,
                   updated_at = excluded.updated_at,
                   floating = excluded.floating",
            )?;
            let stamp = now();
            for e in events {
                let floating = floats(e);
                let keep = |at: i64| if floating { wall_clock(at, &zone) } else { at };
                stmt.execute(params![
                    e.guid,
                    e.calendar_guid,
                    e.title,
                    e.location,
                    keep(e.starts_at),
                    keep(e.ends_at),
                    i64::from(e.all_day),
                    e.timezone,
                    e.url,
                    e.notes,
                    e.etag,
                    stamp,
                    i64::from(floating)
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn calendars(&self) -> Result<Vec<Calendar>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, title, colour, ctag, sort_order, enabled, writable, is_default
             FROM calendars ORDER BY sort_order, title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Calendar {
                guid: row.get(0)?,
                title: row.get(1)?,
                colour: row.get(2)?,
                ctag: row.get(3)?,
                sort_order: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                writable: row.get::<_, i64>(6)? != 0,
                is_default: row.get::<_, i64>(7)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Events overlapping `[from, to)`, from enabled calendars only.
    ///
    /// Overlap rather than containment: a meeting that started before the window but is
    /// still running belongs in today's view.
    pub fn events_between(&self, from: i64, to: i64) -> Result<Vec<Event>> {
        // A floating event's stored times are its wall clock: the window is widened by the
        // furthest a zone is from UTC to find it, and each judged exactly once read.
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM events e
             JOIN calendars c ON c.guid = e.calendar_guid
             WHERE c.enabled = 1
               AND ((e.floating = 0 AND e.starts_at < ?2 AND e.ends_at > ?1)
                 OR (e.floating = 1 AND e.starts_at < ?2 + ?3 AND e.ends_at > ?1 - ?3))",
            prefixed("e.")
        ))?;
        let zone = self.local_zone();
        let rows = stmt.query_map(params![from, to, WIDEST_OFFSET], |row| {
            Self::event_from_row(row, &zone)
        })?;
        let mut found = Vec::new();
        for e in rows {
            let e = e?;
            if e.starts_at < to && e.ends_at > from {
                found.push(e);
            }
        }
        found.sort_by(|a, b| (a.starts_at, &a.title).cmp(&(b.starts_at, &b.title)));
        Ok(found)
    }

    /// Events whose title, place or notes hold every word of `query`, accent- and
    /// case-insensitively, soonest first — what Calendar's search field finds.
    pub fn search_events(&self, query: &str) -> Result<Vec<Event>> {
        self.search_events_near(query, now())
    }

    /// The same, read in one pass over the table — a lookup per event made a large account's
    /// search wait — and, when there are more than are kept, the ones nearest `now`: kept
    /// oldest first, an account with years behind it found nothing that is still to come.
    pub fn search_events_near(&self, query: &str, now: i64) -> Result<Vec<Event>> {
        const KEPT: usize = 300;
        let words = crate::fold::words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {EVENT_COLUMNS} FROM events"))?;
        let zone = self.local_zone();
        let rows = stmt.query_map([], |row| Self::event_from_row(row, &zone))?;
        let mut found = Vec::new();
        for e in rows {
            let e = e?;
            let text = [
                Some(e.title.as_str()),
                e.location.as_deref(),
                e.notes.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            if crate::fold::holds_all(&crate::fold::fold(&text), &words) {
                found.push(e);
            }
        }
        if found.len() > KEPT {
            found.sort_by_key(|e| (e.ends_at.max(e.starts_at) - now).abs());
            found.truncate(KEPT);
        }
        found.sort_by_key(|e| e.starts_at);
        Ok(found)
    }

    /// One event by its guid, if the library still holds it.
    pub fn event(&self, guid: &str) -> Result<Option<Event>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM events WHERE guid = ?1"
        ))?;
        let zone = self.local_zone();
        let mut rows = stmt.query_map(params![guid], |row| Self::event_from_row(row, &zone))?;
        rows.next().transpose().map_err(Into::into)
    }

    /// Every stored event starting in `[from, to)`, as (guid, calendar), whatever its
    /// calendar's switch — what a pass over that window has to account for.
    pub fn events_starting_between(&self, from: i64, to: i64) -> Result<Vec<(String, String)>> {
        // Widened for floating events, then judged on each one's start as read in this zone
        // now: a pass that prunes what Apple did not return over a window must not reach
        // past it.
        let mut stmt = self.conn.prepare(
            "SELECT guid, calendar_guid, starts_at, floating FROM events
              WHERE (floating = 0 AND starts_at >= ?1 AND starts_at < ?2)
                 OR (floating = 1 AND starts_at >= ?1 - ?3 AND starts_at < ?2 + ?3)",
        )?;
        let zone = self.local_zone();
        let rows = stmt.query_map(params![from, to, WIDEST_OFFSET], |row| {
            let at: i64 = row.get(2)?;
            let at = if row.get::<_, i64>(3)? != 0 {
                instant_of(at, &zone)
            } else {
                at
            };
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, at))
        })?;
        let mut found = Vec::new();
        for row in rows {
            let (guid, calendar, at) = row?;
            if (from..to).contains(&at) {
                found.push((guid, calendar));
            }
        }
        Ok(found)
    }

    /// Drop events Apple no longer has.
    pub fn forget_events(&mut self, guids: &[String]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare("DELETE FROM events WHERE guid = ?1")?;
            for guid in guids {
                stmt.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Record that a service synced successfully.
    pub fn mark_synced(&self, service: &str, token: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_state (service, token, last_synced_at, last_error)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(service) DO UPDATE SET
               token = excluded.token,
               last_synced_at = excluded.last_synced_at,
               last_error = NULL",
            params![service, token, now()],
        )?;
        Ok(())
    }

    /// When a service last synced, if it ever has.
    pub fn last_synced(&self, service: &str) -> Result<Option<i64>> {
        let found = self
            .conn
            .query_row(
                "SELECT last_synced_at FROM sync_state WHERE service = ?1",
                params![service],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten();
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(guid: &str, title: &str) -> Calendar {
        Calendar {
            guid: guid.into(),
            title: title.into(),
            colour: Some("#C8912A".into()),
            ctag: Some("ctag-1".into()),
            sort_order: 0,
            enabled: true,
            writable: true,
            is_default: false,
        }
    }

    fn event(guid: &str, cal: &str, starts_at: i64, ends_at: i64) -> Event {
        Event {
            guid: guid.into(),
            calendar_guid: cal.into(),
            title: format!("Event {guid}"),
            location: None,
            starts_at,
            ends_at,
            all_day: false,
            timezone: Some("Asia/Kolkata".into()),
            url: None,
            notes: None,
            etag: None,
        }
    }

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_calendars(&[calendar("cal-1", "Work"), calendar("cal-2", "Home")])
            .unwrap();
        lib
    }

    fn zone(name: &str) -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::get(name).unwrap()
    }

    /// `y-m-d h:m` in `zone`, as an instant.
    fn at(name: &str, y: i16, m: i8, d: i8, h: i8, min: i8) -> i64 {
        jiff::civil::DateTime::new(y, m, d, h, min, 0, 0)
            .unwrap()
            .to_zoned(zone(name))
            .unwrap()
            .timestamp()
            .as_second()
    }

    fn wall(name: &str, seconds: i64) -> String {
        jiff::Timestamp::from_second(seconds)
            .unwrap()
            .to_zoned(zone(name))
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }

    fn all_day(guid: &str, starts_at: i64, ends_at: i64) -> Event {
        Event {
            all_day: true,
            timezone: None,
            ..event(guid, "cal-1", starts_at, ends_at)
        }
    }

    /// Flown from London to New York: a birthday stays on its day, and a floating 9:00 at
    /// 9:00, read in the zone the computer is in now. Kept as instants, both drew on the day
    /// before until the next full pass, and an edit wrote that day back to Apple.
    #[test]
    fn all_day_and_floating_events_stay_put_when_the_zone_changes() {
        let mut lib = seeded();
        lib.set_zone(Some(zone("Europe/London")));
        let birthday = all_day(
            "B",
            at("Europe/London", 2026, 11, 2, 0, 0),
            at("Europe/London", 2026, 11, 3, 0, 0),
        );
        let floating = Event {
            timezone: None,
            ..event(
                "F",
                "cal-1",
                at("Europe/London", 2026, 11, 2, 9, 0),
                at("Europe/London", 2026, 11, 2, 10, 0),
            )
        };
        let timed = Event {
            timezone: Some("Europe/London".into()),
            ..event(
                "T",
                "cal-1",
                at("Europe/London", 2026, 11, 2, 9, 0),
                at("Europe/London", 2026, 11, 2, 10, 0),
            )
        };
        lib.put_events(&[birthday, floating, timed.clone()])
            .unwrap();

        lib.set_zone(Some(zone("America/New_York")));
        let b = lib.event("B").unwrap().unwrap();
        assert_eq!(wall("America/New_York", b.starts_at), "2026-11-02 00:00");
        assert_eq!(wall("America/New_York", b.ends_at), "2026-11-03 00:00");
        let f = lib.event("F").unwrap().unwrap();
        assert_eq!(wall("America/New_York", f.starts_at), "2026-11-02 09:00");
        // A timed event with a zone is the instant it always was: 4 AM in New York.
        assert_eq!(lib.event("T").unwrap().unwrap().starts_at, timed.starts_at);

        // The day's window in New York finds the birthday, the day before's does not.
        let day = |d: i8| {
            lib.events_between(
                at("America/New_York", 2026, 11, d, 0, 0),
                at("America/New_York", 2026, 11, d + 1, 0, 0),
            )
            .unwrap()
            .into_iter()
            .map(|e| e.guid)
            .filter(|g| g == "B")
            .count()
        };
        assert_eq!(day(2), 1);
        assert_eq!(day(1), 0, "never on the day before");
        // A pass over a window prunes only what starts in it, read in this zone.
        let starting = lib
            .events_starting_between(
                at("America/New_York", 2026, 11, 1, 0, 0),
                at("America/New_York", 2026, 11, 2, 0, 0),
            )
            .unwrap();
        assert!(starting.iter().all(|(g, _)| g != "B" && g != "F"));

        // Edited there, it is stored as the day it shows: back home it is still that day.
        let mut edited = lib.event("B").unwrap().unwrap();
        edited.title = "Birthday".into();
        lib.put_events(&[edited]).unwrap();
        lib.set_zone(Some(zone("Europe/London")));
        let b = lib.event("B").unwrap().unwrap();
        assert_eq!(wall("Europe/London", b.starts_at), "2026-11-02 00:00");
    }

    /// A floating 9:00 stays 9:00 across a change of the clocks, and on the day itself.
    #[test]
    fn a_floating_time_holds_across_the_clocks_changing() {
        let mut lib = seeded();
        lib.set_zone(Some(zone("America/New_York")));
        let before = at("America/New_York", 2026, 3, 7, 9, 0);
        let on_the_day = at("America/New_York", 2026, 3, 8, 9, 0);
        let mk = |g: &str, t: i64| Event {
            timezone: None,
            ..event(g, "cal-1", t, t + 3600)
        };
        lib.put_events(&[mk("A", before), mk("B", on_the_day)])
            .unwrap();
        lib.set_zone(Some(zone("Europe/Berlin")));
        for g in ["A", "B"] {
            let e = lib.event(g).unwrap().unwrap();
            assert!(wall("Europe/Berlin", e.starts_at).ends_with("09:00"), "{g}");
        }
        assert_eq!(
            wall("Europe/Berlin", lib.event("B").unwrap().unwrap().starts_at),
            "2026-03-08 09:00"
        );
    }

    /// Events stored before the change are turned over in place, in the zone they were
    /// stored in, so nothing moves by being migrated.
    #[test]
    fn events_stored_before_are_kept_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let system = jiff::tz::TimeZone::system();
        let zone_then = if system.iana_name().is_some() {
            system
        } else {
            jiff::tz::TimeZone::UTC
        };
        let midnight = jiff::civil::date(2026, 12, 25)
            .to_zoned(zone_then.clone())
            .unwrap()
            .timestamp()
            .as_second();
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            let before = &crate::schema::MIGRATIONS[..48];
            crate::migrate_with(&mut conn, before, &[]).unwrap();
            conn.execute_batch(
                "INSERT INTO calendars (guid, title, sort_order, enabled, updated_at) VALUES ('cal-1', 'Home', 0, 1, 0);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (guid, calendar_guid, title, starts_at, ends_at, all_day,
                                     timezone, updated_at)
                 VALUES ('X', 'cal-1', 'Christmas', ?1, ?2, 1, NULL, 0),
                        ('T', 'cal-1', 'Call', 1000, 2000, 0, 'Asia/Kolkata', 0)",
                rusqlite::params![midnight, midnight + 86_400],
            )
            .unwrap();
        }
        let mut lib = Library::open(dir.path()).unwrap();
        lib.set_zone(Some(zone_then));
        let x = lib.event("X").unwrap().unwrap();
        assert_eq!((x.starts_at, x.ends_at), (midnight, midnight + 86_400));
        assert_eq!(lib.event("T").unwrap().unwrap().starts_at, 1000);
        // And from then on it is the day, wherever the computer goes.
        lib.set_zone(Some(zone("Pacific/Auckland")));
        let x = lib.event("X").unwrap().unwrap();
        assert_eq!(wall("Pacific/Auckland", x.starts_at), "2026-12-25 00:00");
    }

    #[test]
    fn calendars_round_trip_and_stay_ordered() {
        let lib = seeded();
        let found = lib.calendars().unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].title, "Home",
            "ordering is by sort order then title"
        );
        assert_eq!(found[0].colour.as_deref(), Some("#C8912A"));
    }

    /// A calendar deleted on another device must disappear here, with its events.
    #[test]
    fn calendars_apple_stops_reporting_are_removed() {
        let mut lib = seeded();
        lib.put_events(&[event("ev-1", "cal-2", 100, 200)]).unwrap();

        lib.put_calendars(&[calendar("cal-1", "Work")]).unwrap();

        assert_eq!(lib.calendars().unwrap().len(), 1);
        assert!(
            lib.events_between(0, 1_000).unwrap().is_empty(),
            "events of a removed calendar must go with it"
        );
    }

    /// Overlap, not containment: a meeting already under way belongs in the view.
    #[test]
    fn the_range_query_includes_events_already_running() {
        let mut lib = seeded();
        lib.put_events(&[
            event("before", "cal-1", 0, 50),       // ends before the window
            event("straddling", "cal-1", 50, 150), // started earlier, still running
            event("inside", "cal-1", 110, 120),
            event("after", "cal-1", 300, 400), // starts after the window
        ])
        .unwrap();

        let found = lib.events_between(100, 200).unwrap();
        let guids: Vec<&str> = found.iter().map(|e| e.guid.as_str()).collect();
        assert_eq!(guids, vec!["straddling", "inside"]);
    }

    #[test]
    fn disabled_calendars_are_hidden_from_the_view() {
        let mut lib = seeded();
        lib.put_events(&[event("ev-1", "cal-1", 100, 200)]).unwrap();
        assert_eq!(lib.events_between(0, 1_000).unwrap().len(), 1);

        let mut hidden = calendar("cal-1", "Work");
        hidden.enabled = false;
        lib.put_calendars(&[hidden, calendar("cal-2", "Home")])
            .unwrap();

        assert!(lib.events_between(0, 1_000).unwrap().is_empty());
    }

    #[test]
    fn re_syncing_updates_events_rather_than_duplicating_them() {
        let mut lib = seeded();
        lib.put_events(&[event("ev-1", "cal-1", 100, 200)]).unwrap();

        let mut moved = event("ev-1", "cal-1", 500, 600);
        moved.title = "Moved".into();
        lib.put_events(&[moved]).unwrap();

        let all = lib.events_between(0, 10_000).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "Moved");
        assert_eq!(all[0].starts_at, 500);
    }

    #[test]
    fn sync_state_is_recorded() {
        let lib = seeded();
        assert!(lib.last_synced("calendar").unwrap().is_none());
        lib.mark_synced("calendar", Some("token-1")).unwrap();
        assert!(lib.last_synced("calendar").unwrap().is_some());
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn search_finds_every_word_in_title_place_or_notes_ignoring_accents() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_calendars(&[Calendar {
            guid: "c".into(),
            title: "Work".into(),
            enabled: true,
            writable: true,
            ..Default::default()
        }])
        .unwrap();
        let ev = |g: &str, t: &str, place: Option<&str>, at: i64| Event {
            guid: g.into(),
            calendar_guid: "c".into(),
            title: t.into(),
            location: place.map(Into::into),
            starts_at: at,
            ends_at: at + 60,
            ..Default::default()
        };
        lib.put_events(&[
            ev("a", "Dentist", Some("Café Road"), 200),
            ev("b", "Design review", None, 100),
            ev("c", "Dentist", None, 50),
        ])
        .unwrap();
        let titles = |q: &str| {
            lib.search_events(q)
                .unwrap()
                .into_iter()
                .map(|e| e.guid)
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("dentist"), ["c", "a"], "soonest first");
        assert_eq!(
            titles("dentist cafe"),
            ["a"],
            "every word, accents folded, place included"
        );
        assert!(titles("  ").is_empty());
    }

    /// Years of a weekly meeting: what is kept is around now, not the oldest few hundred.
    #[test]
    fn a_long_history_keeps_the_matches_nearest_now() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_calendars(&[Calendar {
            guid: "c".into(),
            title: "Work".into(),
            enabled: true,
            writable: true,
            ..Default::default()
        }])
        .unwrap();
        let week = 7 * 86_400;
        let events: Vec<Event> = (0..1000)
            .map(|i| Event {
                guid: format!("s{i}"),
                calendar_guid: "c".into(),
                title: "Standup".into(),
                starts_at: i * week,
                ends_at: i * week + 900,
                ..Default::default()
            })
            .collect();
        lib.put_events(&events).unwrap();
        let now = 900 * week;
        let found = lib.search_events_near("standup", now).unwrap();
        assert_eq!(found.len(), 300);
        assert!(
            found.iter().any(|e| e.starts_at == now),
            "this week's is found"
        );
        assert!(
            found.iter().any(|e| e.starts_at == 999 * week),
            "the last one is found"
        );
        assert!(found.windows(2).all(|w| w[0].starts_at <= w[1].starts_at));
    }
}
