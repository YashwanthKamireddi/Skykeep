//! Reminder lists and reminders in the local library.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// A heading inside a reminders list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderSection {
    pub guid: String,
    pub list_guid: String,
    pub title: String,
    pub created_at: Option<i64>,
}

/// The most reminders any one view hands to the window at once.
///
/// The plan rules out unbounded payloads crossing into the webview, and "All" on an account
/// ten years deep is genuinely unbounded. A thousand is far more than fits on a screen and
/// still weighs only a couple of hundred kilobytes; the true totals are counted separately, so
/// a view that reaches this can say plainly that it has.
pub const MOST_AT_ONCE: usize = 1_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderList {
    pub guid: String,
    pub title: String,
    /// `#RRGGBB`, whatever form Apple sent it in.
    pub colour: Option<String>,
    pub ctag: Option<String>,
    pub sort_order: i64,
    pub enabled: bool,
    /// Apple's name for the list's glyph, such as `fitness` or `education1`.
    #[serde(default)]
    pub emblem: Option<String>,
    /// The group this list is filed under, where it is in one.
    #[serde(default)]
    pub group_guid: Option<String>,
    /// Shown as a tile beside Today and Scheduled.
    #[serde(default)]
    pub pinned: bool,
    /// How the list orders its reminders: `manual`, `displayDate_asc`, and so on.
    #[serde(default)]
    pub sorting: Option<String>,
    /// Every reminder in the list by id, in the order the phone keeps them when sorting by
    /// hand. Read here, never sent to the window: a busy list holds hundreds.
    #[serde(skip)]
    pub reminder_order: Vec<String>,
}

/// Put a list's reminders in the order its own setting asks for.
///
/// Apple keeps the setting on the list as `SortingStyle`: `manual`, or a key and a direction
/// such as `displayDate_asc`. By hand means the list's `ReminderIDs`; a reminder missing
/// from those — one made somewhere that did not add it — goes after them, oldest first, which
/// is where a new one lands. Whatever the setting, what is still to do comes before what is
/// done, and ties fall back to the hand order so nothing jumps about between refreshes.
fn sort_as_the_list_does(
    rows: &mut [(Reminder, Option<i64>)],
    sorting: Option<&str>,
    order: &[String],
) {
    let uuid = |guid: &str| guid.rsplit('/').next().unwrap_or(guid).to_ascii_uppercase();
    let place: std::collections::HashMap<String, usize> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.to_ascii_uppercase(), i))
        .collect();
    let by_hand = |(r, created): &(Reminder, Option<i64>)| {
        (
            place.get(&uuid(&r.guid)).copied().unwrap_or(usize::MAX),
            created.unwrap_or(i64::MAX),
        )
    };
    let (key, descending) = match (sorting, sorting.and_then(|s| s.rsplit_once('_'))) {
        (_, Some((key, direction))) => (key, direction == "desc"),
        // A list that states no order and holds none by hand — one from the older web
        // service — is sorted by date, as it always was here.
        (None, None) if order.is_empty() => ("displayDate", false),
        _ => ("manual", false),
    };
    rows.sort_by(|a, b| {
        use std::cmp::Ordering;
        let done = a.0.completed_at.is_some().cmp(&b.0.completed_at.is_some());
        let chosen = match key {
            // Undated ones go last whichever way the dates run, as they do on the phone.
            "displayDate" | "dueDate" => match (a.0.due_at, b.0.due_at) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            "creationDate" => a.1.cmp(&b.1),
            "title" => crate::fold::natural(&a.0.title, &b.0.title),
            // Apple's scale runs 1 (high) to 9 (low), with 0 meaning none at all.
            "priority" => {
                let rank = |p: i64| if p == 0 { 10 } else { p };
                rank(b.0.priority).cmp(&rank(a.0.priority))
            }
            _ => Ordering::Equal,
        };
        let chosen = if descending && key != "displayDate" && key != "dueDate" {
            chosen.reverse()
        } else if descending {
            // Descending dates still keep the undated at the end.
            match (a.0.due_at, b.0.due_at) {
                (Some(_), Some(_)) => chosen.reverse(),
                _ => chosen,
            }
        } else {
            chosen
        };
        done.then(chosen).then_with(|| by_hand(a).cmp(&by_hand(b)))
    });
}

fn order_json(order: &[String]) -> Option<String> {
    (!order.is_empty()).then(|| serde_json::to_string(order).unwrap_or_default())
}

fn order_from_json(stored: Option<String>) -> Vec<String> {
    stored
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// A folder of lists. It holds no reminders of its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderGroup {
    pub guid: String,
    pub title: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub guid: String,
    pub list_guid: String,
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<i64>,
    /// `None` while outstanding.
    pub completed_at: Option<i64>,
    pub priority: i64,
    #[serde(default)]
    pub flagged: bool,
    /// The reminder this one is a subtask of, if any.
    #[serde(default)]
    pub parent_guid: Option<String>,
    /// Apple's repeat frequency: 0 daily, 1 weekly, 2 monthly, 3 yearly. `None` if it does
    /// not repeat.
    #[serde(default)]
    pub recur_frequency: Option<i64>,
    #[serde(default)]
    pub recur_interval: Option<i64>,
    pub etag: Option<String>,
    /// Apple's original JSON for this reminder, kept verbatim so a write can echo every
    /// field back rather than only the ones we model.
    ///
    /// Never sent to the window. It averages five kilobytes a reminder on a real account,
    /// against about fourteen bytes of the title and notes anyone actually reads — so a list
    /// of seven hundred carried three and a half megabytes of it, and twenty thousand would
    /// have carried ninety. The interface has never looked at a byte of it: writes name a
    /// reminder by its guid and the record is read back from the library on this side.
    #[serde(default, skip_serializing)]
    pub raw: Option<String>,
    /// The named zone Apple keeps beside the due date. Nine in the morning is nine in the
    /// morning wherever the person is, and that is not something a timestamp can say.
    #[serde(default)]
    pub timezone: Option<String>,
    /// The section of its list this reminder sits in, if any. Set from the list's membership
    /// file, never from the reminder — Apple does not record it on the reminder at all.
    #[serde(default)]
    pub section_guid: Option<String>,
    /// Due on a day, not at a time — Apple's `AllDay`. Its `DueDate` is that day's midnight,
    /// and it rings no alarm.
    #[serde(default)]
    pub all_day: bool,
}

/// What every entry in the sidebar is holding.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderCounts {
    pub today: usize,
    pub scheduled: usize,
    pub flagged: usize,
    pub all: usize,
    /// Every reminder ticked off, in every list shown.
    pub completed: usize,
    /// Outstanding reminders per list, keyed by list guid.
    pub lists: Vec<(String, usize)>,
}

/// What the Completed list holds, for its tile's count and for opening it alike.
const COMPLETED: &str = "r.completed_at IS NOT NULL";

/// Apple's built-in views over every list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmartList {
    Today,
    Scheduled,
    Flagged,
    All,
    /// Every reminder ticked off, the most recently completed first, as Reminders' own
    /// Completed list shows them.
    Completed,
}

impl Reminder {
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.completed_at.is_some()
    }

    /// True when this repeats, so completing it advances rather than closes it.
    #[must_use]
    pub fn repeats(&self) -> bool {
        self.recur_frequency.is_some()
    }

    /// The next occurrence this reminder is due, given that today's was just ticked off.
    ///
    /// Apple's frequency is 0 daily, 1 weekly, 2 monthly, 3 yearly. Months and years are
    /// added as calendar units rather than fixed spans, so a monthly reminder on the 31st
    /// lands where a person expects rather than drifting.
    #[must_use]
    pub fn next_occurrence(&self) -> Option<i64> {
        self.next_occurrence_after(now())
    }

    /// The first occurrence strictly after `after`.
    ///
    /// It always advances at least one interval, then keeps advancing while the result is
    /// still in the past: tick a daily reminder you skipped for a week and it lands on
    /// tomorrow, not on the morning after the one you missed — which would leave it
    /// overdue and looking as though the tick did nothing.
    ///
    /// The arithmetic runs in UTC. Apple stores the due date as an absolute instant, so a
    /// daily repeat means exactly twenty-four hours later regardless of where the machine
    /// running this happens to be.
    #[must_use]
    pub fn next_occurrence_after(&self, after: i64) -> Option<i64> {
        let due = self.due_at?;
        let frequency = self.recur_frequency?;
        // An interval outside this range is not a schedule anyone keeps, and unbounded
        // values are how a span constructor is made to fail.
        let interval = self.recur_interval.unwrap_or(1).clamp(1, 1_000);

        // One step of the rule, `k` steps on: counted from where the series is anchored, not
        // added to the last date. Added to the last date, every month clamped where the one
        // before had been — the 31st became the 28th in February and stayed the 28th for
        // ever after, and a yearly 29 February became the 28th and never came back.
        let step = |k: i64| -> Option<jiff::Span> {
            let n = interval.checked_mul(k)?;
            Some(match frequency {
                0 => jiff::Span::new().try_days(n).ok()?,
                1 => jiff::Span::new().try_weeks(n).ok()?,
                2 => jiff::Span::new().try_months(n).ok()?,
                3 => jiff::Span::new().try_years(n).ok()?,
                4 => jiff::Span::new().try_hours(n).ok()?,
                _ => return None,
            })
        };
        step(1)?;
        // Stepped in the reminder's own zone, not in UTC. A day in UTC is always exactly
        // twenty-four hours, so on the two days a year a clock moves, stepping there carried a
        // nine o'clock reminder to ten and six months later back to nine. In a real zone a day
        // is whatever that day is, and the hour holds.
        let zone = self
            .timezone
            .as_deref()
            .and_then(|name| jiff::tz::TimeZone::get(name).ok())
            .unwrap_or_else(jiff::tz::TimeZone::system);
        let anchor = jiff::Timestamp::from_second(self.series_anchor(due))
            .ok()?
            .to_zoned(zone);

        // Each date clamped for its own month only: the 31st is the last day of a shorter
        // month and the 31st again after it; 29 February is the 28th in a common year and the
        // 29th again in a leap year. The cap stops a daily reminder abandoned a decade ago
        // from spinning; eleven years of daily steps is far past any real backlog.
        let mut next = None;
        for k in 1..=MAX_RECURRENCE_STEPS as i64 {
            let Some(at) = step(k).and_then(|span| anchor.checked_add(span).ok()) else {
                break;
            };
            let at = at.timestamp().as_second();
            next = Some(at);
            if at > after && at > due {
                break;
            }
        }
        next
    }

    /// Where a monthly or yearly series counts its dates from: the date Apple's own copy of
    /// the record is due (`raw`), when that is on the way to the date shown here — a tick
    /// laid here offline moves the date shown, clamped, and Apple's still carries the day the
    /// series falls on. Otherwise, and for every other rule, the date due.
    fn series_anchor(&self, due: i64) -> i64 {
        if !matches!(self.recur_frequency, Some(2 | 3)) {
            return due;
        }
        self.raw
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|r| {
                r.pointer("/fields/DueDate/value")
                    .and_then(serde_json::Value::as_i64)
            })
            .map(|ms| ms.div_euclid(1000))
            .filter(|&apple| apple <= due)
            .unwrap_or(due)
    }
}

impl Reminder {
    /// Whether a tick at `at` is the last of the series: its repeat ends (`end_at`, End Repeat
    /// On Date) before the next occurrence. Ticked then, it is completed, as on the iPhone,
    /// rather than moved on to a date past its end.
    #[must_use]
    pub fn tick_ends_series(&self, end_at: Option<i64>, at: i64) -> bool {
        self.tick_ends_series_on(end_at, &[], at)
    }

    /// [`Self::tick_ends_series`] for a rule that falls on chosen weekdays.
    #[must_use]
    pub fn tick_ends_series_on(&self, end_at: Option<i64>, days: &[i64], at: i64) -> bool {
        let Some(end) = end_at else {
            return false;
        };
        self.repeats()
            && self
                .next_occurrence_on(days, at)
                .is_some_and(|next| next > end)
    }

    /// The next occurrence after `at` of a rule that falls on chosen weekdays (`days`, Apple's
    /// numbering: 1 Sunday … 7 Saturday) — Weekdays, Weekends, or days picked in Custom. Weeks
    /// count from Sunday, as the phone's `FirstDayOfTheWeek` 0 says, and every `interval`th
    /// week is one it falls in. Any other rule steps as [`Self::next_occurrence_after`] does.
    #[must_use]
    pub fn next_occurrence_on(&self, days: &[i64], after: i64) -> Option<i64> {
        if days.is_empty() || self.recur_frequency != Some(1) {
            return self.next_occurrence_after(after);
        }
        let due = self.due_at?;
        let interval = self.recur_interval.unwrap_or(1).clamp(1, 1_000);
        let zone = self
            .timezone
            .as_deref()
            .and_then(|name| jiff::tz::TimeZone::get(name).ok())
            .unwrap_or_else(jiff::tz::TimeZone::system);
        let start = jiff::Timestamp::from_second(due).ok()?.to_zoned(zone);
        // Sunday of the week it was due in, as a day count.
        let sunday = |d: jiff::civil::Date| {
            let back = i64::from(d.weekday().to_sunday_zero_offset());
            d.checked_sub(jiff::Span::new().days(back)).ok()
        };
        let first_week = sunday(start.date())?;
        for step in 1..=(MAX_RECURRENCE_STEPS as i64 * 7) {
            let candidate = start.checked_add(jiff::Span::new().days(step)).ok()?;
            let weekday = i64::from(candidate.date().weekday().to_sunday_one_offset());
            if !days.contains(&weekday) {
                continue;
            }
            let weeks = first_week.until(sunday(candidate.date())?).ok()?.get_days() / 7;
            if i64::from(weeks) % interval != 0 {
                continue;
            }
            let at = candidate.timestamp().as_second();
            if at > after {
                return Some(at);
            }
        }
        None
    }
}

/// The weekdays a rule's `DaysOfTheWeek` holds — base64 of JSON as the phone writes it,
/// `[{"dayOfTheWeek":2,"weekNumber":0},…]`, 1 Sunday … 7 Saturday — sorted. `None` when it
/// does not read, or names a week of the month ("the first Monday"), which is not a weekday
/// choice.
#[must_use]
pub fn days_of_week_from(base64_json: &str) -> Option<Vec<i64>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_json.trim())
        .ok()?;
    let entries: Vec<serde_json::Value> = serde_json::from_slice(&bytes).ok()?;
    let mut days = Vec::with_capacity(entries.len());
    for e in entries {
        let day = e.get("dayOfTheWeek")?.as_i64()?;
        if !(1..=7).contains(&day)
            || e.get("weekNumber")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0)
                != 0
        {
            return None;
        }
        days.push(day);
    }
    days.sort_unstable();
    days.dedup();
    Some(days)
}

/// Weekdays as the phone writes them in a rule's `DaysOfTheWeek`: base64 of compact JSON,
/// in day order, each `{"dayOfTheWeek":n,"weekNumber":0}` — the key order of the phone's
/// own Weekdays rule.
#[must_use]
pub fn days_of_week_to(days: &[i64]) -> String {
    use base64::Engine;
    let mut days: Vec<i64> = days
        .iter()
        .copied()
        .filter(|d| (1..=7).contains(d))
        .collect();
    days.sort_unstable();
    days.dedup();
    let json = days
        .iter()
        .map(|d| format!(r#"{{"dayOfTheWeek":{d},"weekNumber":0}}"#))
        .collect::<Vec<_>>()
        .join(",");
    base64::engine::general_purpose::STANDARD.encode(format!("[{json}]"))
}

/// How far `next_occurrence_after` will walk forward before giving up.
const MAX_RECURRENCE_STEPS: usize = 4_000;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// What Today holds: open reminders due by the end of today — overdue counts, as Apple
/// shows it there. A timed reminder is compared as a moment, against the end of the local
/// day (`?1`); an all-day one as a date, against the next day's UTC midnight (`?2`), since
/// Apple keeps an all-day date as that day's UTC midnight. Compared as a moment, west of
/// Greenwich tomorrow's all-day reminder fell before tonight's local midnight and showed in
/// Today.
const TODAY: &str = "r.completed_at IS NULL AND r.due_at IS NOT NULL
     AND ((r.all_day = 0 AND r.due_at < ?1) OR (r.all_day = 1 AND r.due_at < ?2))";

/// UTC midnight of the day after the local day that ends at `until`.
fn all_day_bound(until: i64) -> i64 {
    let zone = jiff::tz::TimeZone::system();
    jiff::Timestamp::from_second(until - 1)
        .ok()
        .map(|t| t.to_zoned(zone).date())
        .and_then(|d| d.tomorrow().ok())
        .and_then(|d| d.to_zoned(jiff::tz::TimeZone::UTC).ok())
        .map_or(until, |z| z.timestamp().as_second())
}

impl Library {
    /// Upsert lists without removing anything.
    ///
    /// An incremental sync returns only what changed, so absence means "unchanged", not
    /// "deleted". Using the replacing variant there would wipe the whole library on the
    /// first incremental refresh.
    pub fn merge_reminder_lists(&mut self, lists: &[ReminderList]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminder_lists (guid, title, colour, ctag, sort_order, enabled,
                                             emblem, group_guid, pinned, sorting,
                                             reminder_order, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title, colour = excluded.colour, ctag = excluded.ctag,
                   sort_order = excluded.sort_order, enabled = excluded.enabled,
                   emblem = excluded.emblem, group_guid = excluded.group_guid,
                   pinned = excluded.pinned, sorting = excluded.sorting,
                   reminder_order = excluded.reminder_order,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for l in lists {
                stmt.execute(params![
                    l.guid,
                    l.title,
                    l.colour,
                    l.ctag,
                    l.sort_order,
                    i64::from(l.enabled),
                    l.emblem,
                    l.group_guid,
                    i64::from(l.pinned),
                    l.sorting,
                    order_json(&l.reminder_order),
                    stamp
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// A list deleted (Apple has taken the delete): the list gone from the library with every
    /// reminder in it, done ones included, so no count or smart list shows them meanwhile.
    pub fn forget_reminder_list(&mut self, list_guid: &str) -> Result<()> {
        let guids: Vec<String> = self
            .reminders_in(list_guid, true)?
            .into_iter()
            .map(|r| r.guid)
            .collect();
        self.forget_reminders(&guids, &[list_guid.to_string()])
    }

    /// Remove reminders and lists the server reports as gone.
    pub fn forget_reminders(
        &mut self,
        reminder_guids: &[String],
        list_guids: &[String],
    ) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut drop_reminder = tx.prepare("DELETE FROM reminders WHERE guid = ?1")?;
            for guid in reminder_guids {
                drop_reminder.execute(params![guid])?;
            }
            // Removing a list takes its reminders with it, by foreign key.
            let mut drop_list = tx.prepare("DELETE FROM reminder_lists WHERE guid = ?1")?;
            for guid in list_guids {
                drop_list.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Dated reminders falling inside `[from, to)`, for the calendar.
    ///
    /// Apple's Calendar shows reminders alongside events, which is most of what fills a
    /// month for someone who lives in Reminders rather than Calendar.
    /// Store sections as Apple has them now, and drop the ones Apple says are gone.
    pub fn put_reminder_sections(
        &mut self,
        sections: &[ReminderSection],
        gone: &[String],
    ) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut put = tx.prepare(
                "INSERT INTO reminder_sections (guid, list_guid, title, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(guid) DO UPDATE SET
                   list_guid = excluded.list_guid, title = excluded.title,
                   created_at = excluded.created_at, updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for section in sections {
                put.execute(params![
                    section.guid,
                    section.list_guid,
                    section.title,
                    section.created_at,
                    stamp
                ])?;
            }
            let mut drop = tx.prepare("DELETE FROM reminder_sections WHERE guid = ?1")?;
            let mut loose =
                tx.prepare("UPDATE reminders SET section_guid = NULL WHERE section_guid = ?1")?;
            for guid in gone {
                drop.execute(params![guid])?;
                // A deleted section's reminders stay in the list; they just have no heading.
                loose.execute(params![guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Which section each reminder in a list sits in, as the list's membership file says.
    ///
    /// The file is the whole truth for that list, so a reminder it does not place is set
    /// outside every section rather than left where it was. A placement naming a section
    /// this library does not hold — one deleted since — is treated the same way, because
    /// a heading nobody can see is not somewhere to put anything.
    pub fn set_reminder_sections(
        &mut self,
        list_guid: &str,
        placed: &[(String, String)],
    ) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            tx.execute(
                "UPDATE reminders SET section_guid = NULL WHERE list_guid = ?1",
                params![list_guid],
            )?;
            let mut set = tx.prepare(
                "UPDATE reminders SET section_guid = ?2
                 WHERE guid = ?1 AND list_guid = ?3
                   AND EXISTS (SELECT 1 FROM reminder_sections WHERE guid = ?2)",
            )?;
            for (reminder, section) in placed {
                set.execute(params![reminder, section, list_guid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// A list's sections, in the order they were made.
    pub fn reminder_sections(&self, list_guid: &str) -> Result<Vec<ReminderSection>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, list_guid, title, created_at FROM reminder_sections
             WHERE list_guid = ?1 ORDER BY created_at, title",
        )?;
        let rows = stmt.query_map(params![list_guid], |row| {
            Ok(ReminderSection {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Apple's whole record for one reminder, for the write path that echoes it back.
    ///
    /// Fetched on its own rather than carried on every listed reminder. It averages five
    /// kilobytes a row, so reading it for twenty thousand of them to answer a question about
    /// titles and dates cost about a hundred megabytes of nothing.
    pub fn reminder_raw(&self, guid: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT raw FROM reminders WHERE guid = ?1")?;
        let mut rows = stmt.query(params![guid])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    }

    pub fn reminders_due_between(&self, from: i64, to: i64) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at,
                    r.priority, r.flagged, r.parent_guid, r.recur_frequency, r.recur_interval,
                    r.etag, r.timezone, r.section_guid, r.all_day
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.due_at IS NOT NULL
               AND r.due_at >= ?1 AND r.due_at < ?2
             -- Open ones first, so a busy year's finished history can never use up the cap and
             -- leave reminders still to do off the Year view; each placed by its date anyway.
             ORDER BY r.completed_at IS NOT NULL, r.due_at
             LIMIT {MOST_AT_ONCE}"
        ))?;
        let rows = stmt.query_map(params![from, to], |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                parent_guid: row.get(8)?,
                recur_frequency: row.get(9)?,
                recur_interval: row.get(10)?,
                etag: row.get(11)?,
                raw: None,
                timezone: row.get(12)?,
                section_guid: row.get(13)?,
                all_day: row.get::<_, i64>(14)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// One of Apple's smart lists.
    ///
    /// These are the entry points people actually use: what is due today, everything with a
    /// date, and whatever has been flagged. They are queries rather than stored lists, which
    /// is what lets them stay correct as the clock moves.
    /// How many reminders each smart list and each list is holding.
    ///
    /// Apple puts a count on every entry in the sidebar, and it is the number that makes
    /// the sidebar worth looking at. Counting in SQL rather than by fetching each list and
    /// measuring it is the difference between one cheap query and a dozen full reads.
    pub fn reminder_counts(&self, until: i64) -> Result<ReminderCounts> {
        // The same conditions the smart lists themselves use, so a tile can never disagree
        // with what opening it shows.
        let one = |clause: &str, args: &[i64]| -> Result<usize> {
            let sql = format!(
                "SELECT COUNT(*) FROM reminders r
                 JOIN reminder_lists l ON l.guid = r.list_guid
                 WHERE l.enabled = 1 AND {clause}"
            );
            let count: i64 = self.conn.query_row(
                &sql,
                rusqlite::params_from_iter(args.iter().copied()),
                |row| row.get(0),
            )?;
            Ok(usize::try_from(count).unwrap_or(0))
        };

        let mut stmt = self.conn.prepare(
            "SELECT l.guid, COUNT(r.guid)
             FROM reminder_lists l
             LEFT JOIN reminders r ON r.list_guid = l.guid AND r.completed_at IS NULL
             GROUP BY l.guid",
        )?;
        let lists = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(guid, n)| (guid, usize::try_from(n).unwrap_or(0)))
            .collect();

        Ok(ReminderCounts {
            today: one(TODAY, &[until, all_day_bound(until)])?,
            scheduled: one("r.completed_at IS NULL AND r.due_at IS NOT NULL", &[])?,
            flagged: one("r.completed_at IS NULL AND r.flagged = 1", &[])?,
            all: one("r.completed_at IS NULL", &[])?,
            completed: one(COMPLETED, &[])?,
            lists,
        })
    }

    pub fn reminders_smart(&self, kind: SmartList, until: i64) -> Result<Vec<Reminder>> {
        // Only Today is bounded by the clock, so only Today binds a value — binding one
        // regardless is what SQLite rejects as a parameter-count mismatch.
        let (clause, args): (&str, Vec<i64>) = match kind {
            // Overdue counts as today: Apple shows it there rather than hiding it in the past.
            SmartList::Today => (TODAY, vec![until, all_day_bound(until)]),
            SmartList::Scheduled => ("r.completed_at IS NULL AND r.due_at IS NOT NULL", vec![]),
            SmartList::Flagged => ("r.completed_at IS NULL AND r.flagged = 1", vec![]),
            SmartList::All => ("r.completed_at IS NULL", vec![]),
            SmartList::Completed => (COMPLETED, vec![]),
        };
        // Completed reminders come latest first; the rest by when they are due.
        let order = if kind == SmartList::Completed {
            "r.completed_at DESC, r.title"
        } else {
            "r.due_at IS NULL, r.due_at, r.priority DESC, r.title"
        };
        let sql = format!(
            "SELECT r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at,
                    r.priority, r.flagged, r.parent_guid, r.recur_frequency, r.recur_interval,
                    r.etag, r.timezone, r.section_guid, r.all_day
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND {clause}
             ORDER BY {order}
             LIMIT {MOST_AT_ONCE}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args), |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                parent_guid: row.get(8)?,
                recur_frequency: row.get(9)?,
                recur_interval: row.get(10)?,
                etag: row.get(11)?,
                raw: None,
                timezone: row.get(12)?,
                section_guid: row.get(13)?,
                all_day: row.get::<_, i64>(14)? != 0,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Where a service's last sync got to, for resuming.
    pub fn sync_token(&self, service: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT token FROM sync_state WHERE service = ?1",
                params![service],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten())
    }

    /// Replace the set of lists, dropping any Apple no longer reports.
    pub fn put_reminder_lists(&mut self, lists: &[ReminderList]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminder_lists (guid, title, colour, ctag, sort_order, enabled,
                                             emblem, group_guid, pinned, sorting,
                                             reminder_order, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(guid) DO UPDATE SET
                   title = excluded.title, colour = excluded.colour, ctag = excluded.ctag,
                   sort_order = excluded.sort_order, enabled = excluded.enabled,
                   emblem = excluded.emblem, group_guid = excluded.group_guid,
                   pinned = excluded.pinned, sorting = excluded.sorting,
                   reminder_order = excluded.reminder_order,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for l in lists {
                stmt.execute(params![
                    l.guid,
                    l.title,
                    l.colour,
                    l.ctag,
                    l.sort_order,
                    i64::from(l.enabled),
                    l.emblem,
                    l.group_guid,
                    i64::from(l.pinned),
                    l.sorting,
                    order_json(&l.reminder_order),
                    stamp
                ])?;
            }
            let keep: Vec<&str> = lists.iter().map(|l| l.guid.as_str()).collect();
            if keep.is_empty() {
                tx.execute("DELETE FROM reminder_lists", [])?;
            } else {
                tx.execute(
                    "DELETE FROM reminder_lists WHERE guid NOT IN (SELECT value FROM json_each(?1))",
                    params![crate::id_list(&keep)],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Keep only these reminders, dropping everything else.
    ///
    /// A full sync is the whole truth: Apple reported every reminder it has, so anything
    /// still here that is not in that set no longer exists. Tombstones cover the usual
    /// case, but a tombstone is only ever delivered once — if a sync consumed one before
    /// the code knew what to do with it, that record would otherwise stay on screen
    /// forever. This is the floor under that.
    pub fn retain_reminders(&mut self, keep: &[String]) -> Result<usize> {
        let tx = self.conn.savepoint()?;
        let removed = {
            let mut known: std::collections::HashSet<&str> =
                keep.iter().map(String::as_str).collect();
            // A reminder with no list would fail its foreign key anyway; leave those alone.
            let mut stmt = tx.prepare("SELECT guid FROM reminders")?;
            let all: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            let mut delete = tx.prepare("DELETE FROM reminders WHERE guid = ?1")?;
            let mut count = 0;
            for guid in &all {
                if !known.remove(guid.as_str()) {
                    delete.execute(params![guid])?;
                    count += 1;
                }
            }
            count
        };
        tx.commit()?;
        Ok(removed)
    }

    /// Every stored reminder's repeat — frequency, then interval — by guid.
    pub fn reminder_repeats(
        &self,
    ) -> Result<std::collections::HashMap<String, (i64, Option<i64>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, recur_frequency, recur_interval FROM reminders WHERE recur_frequency IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (row.get::<_, i64>(1)?, row.get::<_, Option<i64>>(2)?),
            ))
        })?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(Into::into)
    }

    /// A reminder's repeat, changed by a rule that arrived without the reminder.
    pub fn set_reminder_repeat(&mut self, guid: &str, frequency: i64, interval: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE reminders SET recur_frequency = ?2, recur_interval = ?3 WHERE guid = ?1",
            params![guid, frequency, interval],
        )?;
        Ok(())
    }

    /// A reminder as changed (or made) here, before Apple has it: the fields a person edits,
    /// leaving Apple's own copy of the record (`raw`) and its change tag as they were, since
    /// those are what the write to Apple is made against.
    pub fn put_reminder_local(&mut self, r: &Reminder) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reminders (guid, list_guid, title, notes, due_at, completed_at,
                                    priority, flagged, parent_guid, timezone, updated_at, all_day)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(guid) DO UPDATE SET
               list_guid = excluded.list_guid, title = excluded.title,
               notes = excluded.notes, due_at = excluded.due_at,
               completed_at = excluded.completed_at, priority = excluded.priority,
               flagged = excluded.flagged, parent_guid = excluded.parent_guid,
               timezone = excluded.timezone, all_day = excluded.all_day,
               updated_at = excluded.updated_at",
            params![
                r.guid,
                r.list_guid,
                r.title,
                r.notes,
                r.due_at,
                r.completed_at,
                r.priority,
                i64::from(r.flagged),
                r.parent_guid,
                r.timezone,
                now(),
                i64::from(r.all_day)
            ],
        )?;
        Ok(())
    }

    /// Floating reminders — a time and no zone — made to ring on `zone`'s clock.
    ///
    /// Apple keeps a floating time as clock time written as UTC (#23); the reader stores it as
    /// the moment that clock time is on this computer. When the computer's zone changes (a
    /// trip, a setting), eight in the evening must still be eight in the evening, so each is
    /// read again from Apple's own `DueDate` in its record. Only rows whose moment moved are
    /// written. Run with every Reminders pass, before changes still on their way to Apple are
    /// laid back over it, so a time just set here is never overwritten. Answers how many moved.
    pub fn refloat_reminders(&mut self, zone: &jiff::tz::TimeZone) -> Result<usize> {
        let tx = self.conn.savepoint()?;
        let mut moved = 0;
        {
            let mut stmt = tx.prepare(
                "SELECT guid, due_at, json_extract(raw, '$.fields.DueDate.value') FROM reminders
                  WHERE due_at IS NOT NULL AND all_day = 0
                    AND (timezone IS NULL OR timezone = '') AND json_valid(raw)",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut update = tx.prepare("UPDATE reminders SET due_at = ?2 WHERE guid = ?1")?;
            for (guid, stored, kept_ms) in rows {
                let Some(kept_ms) = kept_ms else { continue };
                let due = crate::calendar::instant_of(kept_ms.div_euclid(1000), zone);
                if due != stored {
                    update.execute(params![guid, due])?;
                    moved += 1;
                }
            }
        }
        tx.commit()?;
        Ok(moved)
    }

    pub fn put_reminders(&mut self, reminders: &[Reminder]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminders (guid, list_guid, title, notes, due_at, completed_at,
                                        priority, flagged, parent_guid, recur_frequency,
                                        recur_interval, etag, raw, timezone, updated_at, all_day)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                 ON CONFLICT(guid) DO UPDATE SET
                   list_guid = excluded.list_guid, title = excluded.title,
                   notes = excluded.notes, due_at = excluded.due_at,
                   completed_at = excluded.completed_at, priority = excluded.priority,
                   flagged = excluded.flagged, parent_guid = excluded.parent_guid,
                   recur_frequency = excluded.recur_frequency,
                   recur_interval = excluded.recur_interval,
                   etag = excluded.etag, raw = excluded.raw,
                   timezone = excluded.timezone,
                   all_day = excluded.all_day,
                   updated_at = excluded.updated_at",
            )?;
            let stamp = now();
            for r in reminders {
                stmt.execute(params![
                    r.guid,
                    r.list_guid,
                    r.title,
                    r.notes,
                    r.due_at,
                    r.completed_at,
                    r.priority,
                    i64::from(r.flagged),
                    r.parent_guid,
                    r.recur_frequency,
                    r.recur_interval,
                    r.etag,
                    r.raw,
                    r.timezone,
                    stamp,
                    i64::from(r.all_day)
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn reminder_lists(&self) -> Result<Vec<ReminderList>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, title, colour, ctag, sort_order, enabled,
                    emblem, group_guid, pinned, sorting, reminder_order
             FROM reminder_lists ORDER BY sort_order, title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ReminderList {
                guid: row.get(0)?,
                title: row.get(1)?,
                colour: row.get(2)?,
                ctag: row.get(3)?,
                sort_order: row.get(4)?,
                enabled: row.get::<_, i64>(5)? != 0,
                emblem: row.get(6)?,
                group_guid: row.get(7)?,
                pinned: row.get::<_, i64>(8)? != 0,
                sorting: row.get(9)?,
                reminder_order: order_from_json(row.get(10)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Replace the folders lists are filed under.
    pub fn put_reminder_groups(&mut self, groups: &[ReminderGroup]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        tx.execute("DELETE FROM reminder_groups", [])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO reminder_groups (guid, title, sort_order, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            let stamp = now();
            for g in groups {
                stmt.execute(params![g.guid, g.title, g.sort_order, stamp])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The folders lists are filed under, in the order they are shown.
    pub fn reminder_groups(&self) -> Result<Vec<ReminderGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, title, sort_order FROM reminder_groups ORDER BY sort_order, title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ReminderGroup {
                guid: row.get(0)?,
                title: row.get(1)?,
                sort_order: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Reminders in a list, outstanding first, then by due date.
    ///
    /// Ordering matters: a list that buries today's task under last month's completed ones
    /// is useless.
    /// Reminders still to do whose title or notes hold every word of `query`, across every
    /// enabled list.
    ///
    /// Matched in Rust rather than SQL for the reason Notes search is: SQLite folds case for
    /// ASCII alone. Bounded like every other view, so a search that matches everything on a
    /// ten-year account does not carry all of it into the window.
    pub fn search_reminders(&self, query: &str) -> Result<Vec<Reminder>> {
        let words = crate::fold::words(query);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at, r.priority,
                    r.flagged, r.parent_guid, r.recur_frequency, r.recur_interval, r.etag,
                    r.timezone, r.section_guid, r.all_day
             FROM reminders r
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.completed_at IS NULL
             ORDER BY r.due_at IS NULL, r.due_at, r.priority DESC, r.title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Reminder {
                guid: row.get(0)?,
                list_guid: row.get(1)?,
                title: row.get(2)?,
                notes: row.get(3)?,
                due_at: row.get(4)?,
                completed_at: row.get(5)?,
                priority: row.get(6)?,
                flagged: row.get::<_, i64>(7)? != 0,
                parent_guid: row.get(8)?,
                recur_frequency: row.get(9)?,
                recur_interval: row.get(10)?,
                etag: row.get(11)?,
                raw: None,
                timezone: row.get(12)?,
                section_guid: row.get(13)?,
                all_day: row.get::<_, i64>(14)? != 0,
            })
        })?;
        let mut found = Vec::new();
        for row in rows {
            let r = row?;
            let haystack = format!("{}\n{}", r.title, r.notes.as_deref().unwrap_or(""));
            if crate::fold::holds_all(&crate::fold::fold(&haystack), &words) {
                found.push(r);
                if found.len() == MOST_AT_ONCE {
                    break;
                }
            }
        }
        Ok(found)
    }

    /// One reminder by its id, wherever it is and however long its list: what a change waiting
    /// to be sent is checked against. (Through a list, one past the most a list returns was
    /// taken for deleted and its change dropped unsent.)
    pub fn reminder(&self, guid: &str) -> Result<Option<Reminder>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {REMINDER_COLUMNS} FROM reminders WHERE guid = ?1"),
                params![guid],
                reminder_row,
            )
            .optional()?
            .map(|(r, _)| r))
    }

    /// A reminder's subtasks, every one, wherever they are.
    pub fn subtasks_of(&self, parent: &str) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {REMINDER_COLUMNS} FROM reminders WHERE parent_guid = ?1"
        ))?;
        let rows = stmt.query_map(params![parent], reminder_row)?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(r, _)| r)
            .collect())
    }

    pub fn reminders_in(&self, list_guid: &str, include_done: bool) -> Result<Vec<Reminder>> {
        let list = self
            .reminder_lists()?
            .into_iter()
            .find(|l| l.guid == list_guid);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {REMINDER_COLUMNS} FROM reminders
             WHERE list_guid = ?1 AND (?2 = 1 OR completed_at IS NULL)"
        ))?;
        let rows = stmt.query_map(params![list_guid, i64::from(include_done)], reminder_row)?;
        let mut found = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let (sorting, order) = list
            .map(|l| (l.sorting, l.reminder_order))
            .unwrap_or_default();
        sort_as_the_list_does(&mut found, sorting.as_deref(), &order);
        found.truncate(MOST_AT_ONCE);
        Ok(found.into_iter().map(|(r, _)| r).collect())
    }
}

/// The columns a reminder is read from, in the order [`reminder_row`] takes them.
const REMINDER_COLUMNS: &str =
    "guid, list_guid, title, notes, due_at, completed_at, priority, flagged,
    parent_guid, recur_frequency, recur_interval, etag, timezone, section_guid, all_day,
    CASE WHEN json_valid(raw) THEN json_extract(raw, '$.fields.CreationDate.value') END";

/// A reminder, and when it was made, from a row of [`REMINDER_COLUMNS`].
fn reminder_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Reminder, Option<i64>)> {
    Ok((
        Reminder {
            guid: row.get(0)?,
            list_guid: row.get(1)?,
            title: row.get(2)?,
            notes: row.get(3)?,
            due_at: row.get(4)?,
            completed_at: row.get(5)?,
            priority: row.get(6)?,
            flagged: row.get::<_, i64>(7)? != 0,
            parent_guid: row.get(8)?,
            recur_frequency: row.get(9)?,
            recur_interval: row.get(10)?,
            etag: row.get(11)?,
            raw: None,
            timezone: row.get(12)?,
            section_guid: row.get(13)?,
            all_day: row.get::<_, i64>(14)? != 0,
        },
        row.get::<_, Option<i64>>(15).unwrap_or(None),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(guid: &str) -> ReminderList {
        ReminderList {
            guid: guid.into(),
            title: format!("List {guid}"),
            colour: Some("#cc73e1".into()),
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }
    }

    fn reminder(guid: &str, due: Option<i64>, done: Option<i64>) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "l1".into(),
            title: guid.into(),
            notes: None,
            due_at: due,
            completed_at: done,
            priority: 0,
            parent_guid: None,
            flagged: false,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
            all_day: false,
        }
    }

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[list("l1")]).unwrap();
        lib
    }

    /// A deleted list leaves nothing behind: its reminders, done or not, go with it, and
    /// another list's stay.
    #[test]
    fn a_forgotten_list_takes_its_reminders_and_only_them() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[list("a"), list("b")]).unwrap();
        let mut a1 = reminder("a1", None, None);
        a1.list_guid = "a".into();
        let mut a2 = reminder("a2", None, Some(1));
        a2.list_guid = "a".into();
        let mut b1 = reminder("b1", None, None);
        b1.list_guid = "b".into();
        lib.put_reminders(&[a1, a2, b1]).unwrap();
        lib.forget_reminder_list("a").unwrap();
        assert!(lib.reminders_in("a", true).unwrap().is_empty());
        assert_eq!(lib.reminders_in("b", true).unwrap().len(), 1);
        assert!(lib.reminder_lists().unwrap().iter().all(|x| x.guid != "a"));
    }

    #[test]
    fn outstanding_reminders_come_before_completed_ones() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("done", Some(100), Some(150)),
            reminder("undated", None, None),
            reminder("due-soon", Some(200), None),
        ])
        .unwrap();

        let visible = lib.reminders_in("l1", false).unwrap();
        assert_eq!(
            visible.iter().map(|r| r.guid.as_str()).collect::<Vec<_>>(),
            vec!["due-soon", "undated"],
            "dated items first, and completed ones hidden"
        );

        let all = lib.reminders_in("l1", true).unwrap();
        assert_eq!(
            all.last().unwrap().guid,
            "done",
            "completed items sink to the bottom"
        );
    }

    /// The bug this guards: an incremental sync returns only what changed, so treating
    /// absence as deletion would wipe everything already stored.
    #[test]
    fn merging_does_not_remove_what_was_not_mentioned() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", None, None)]).unwrap();

        // A later sync mentions a different list entirely.
        lib.merge_reminder_lists(&[list("l2")]).unwrap();

        assert_eq!(
            lib.reminder_lists().unwrap().len(),
            2,
            "the original list must survive"
        );
        assert_eq!(
            lib.reminders_in("l1", true).unwrap().len(),
            1,
            "and so must its reminders"
        );
    }

    #[test]
    fn forgetting_removes_only_what_was_named() {
        let mut lib = seeded();
        lib.put_reminder_lists(&[list("l1"), list("l2")]).unwrap();
        lib.put_reminders(&[reminder("r1", None, None), reminder("r2", None, None)])
            .unwrap();

        lib.forget_reminders(&["r1".to_string()], &[]).unwrap();
        let left = lib.reminders_in("l1", true).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].guid, "r2");

        lib.forget_reminders(&[], &["l1".to_string()]).unwrap();
        assert!(
            lib.reminders_in("l1", true).unwrap().is_empty(),
            "its reminders go too"
        );
        assert_eq!(lib.reminder_lists().unwrap().len(), 1);
    }

    #[test]
    fn dated_reminders_are_found_by_range() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("before", Some(50), None),
            reminder("inside", Some(150), None),
            reminder("also-inside", Some(199), Some(400)), // completed still shows on its day
            reminder("after", Some(250), None),
            reminder("undated", None, None),
        ])
        .unwrap();

        let found = lib.reminders_due_between(100, 200).unwrap();
        assert_eq!(
            found.iter().map(|r| r.guid.as_str()).collect::<Vec<_>>(),
            vec!["inside", "also-inside"],
            "only dated reminders inside the window, in due order"
        );
    }

    #[test]
    fn a_disabled_list_hides_its_reminders_from_the_calendar() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", Some(150), None)])
            .unwrap();
        assert_eq!(lib.reminders_due_between(0, 1000).unwrap().len(), 1);

        let mut hidden = list("l1");
        hidden.enabled = false;
        lib.put_reminder_lists(&[hidden]).unwrap();
        assert!(lib.reminders_due_between(0, 1000).unwrap().is_empty());
    }

    #[test]
    fn a_sync_token_round_trips() {
        let lib = seeded();
        assert!(lib.sync_token("reminders").unwrap().is_none());
        lib.mark_synced("reminders", Some("token-abc")).unwrap();
        assert_eq!(
            lib.sync_token("reminders").unwrap().as_deref(),
            Some("token-abc")
        );
    }

    #[test]
    fn deleting_a_list_takes_its_reminders() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", None, None)]).unwrap();
        lib.put_reminder_lists(&[]).unwrap();
        assert!(lib.reminders_in("l1", true).unwrap().is_empty());
    }

    #[test]
    fn ticking_a_reminder_updates_it_rather_than_duplicating() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("r1", None, None)]).unwrap();
        lib.put_reminders(&[reminder("r1", None, Some(999))])
            .unwrap();

        let all = lib.reminders_in("l1", true).unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].is_done());
    }

    #[test]
    fn an_existing_library_migrates_to_the_new_schema() {
        let lib = Library::in_memory().unwrap();
        assert_eq!(
            lib.schema_version().unwrap() as usize,
            crate::schema::MIGRATIONS.len(),
            "every migration must apply"
        );
    }
}

#[cfg(test)]
mod smart_tests {
    use super::*;

    const DAY_START: i64 = 1_757_000_000;
    const DAY_END: i64 = DAY_START + 86_400;

    fn seeded() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "l1".into(),
            title: "Daily".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        lib
    }

    fn reminder(guid: &str, due: Option<i64>, flagged: bool, done: Option<i64>) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "l1".into(),
            title: guid.into(),
            notes: None,
            due_at: due,
            completed_at: done,
            priority: 0,
            parent_guid: None,
            flagged,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
            all_day: false,
        }
    }

    /// #23: a floating reminder rings at its clock time on this computer's clock, and moves
    /// with that clock when its zone changes; one set in a zone, or all-day, never moves.
    #[test]
    fn a_floating_reminder_follows_the_clock_it_is_read_on() {
        let mut lib = seeded();
        // 20:00 kept as Apple keeps a floating time (as UTC), and a zoned 20:00 in Chicago.
        let kept: i64 = 1_794_254_400;
        let raw = |zone: Option<&str>, all_day: i64| {
            let mut f = serde_json::json!({ "DueDate": { "type": "TIMESTAMP", "value": kept * 1000 },
                                            "AllDay": { "value": all_day } });
            if let Some(z) = zone {
                f["TimeZone"] = serde_json::json!({ "value": z });
            }
            Some(serde_json::json!({ "fields": f }).to_string())
        };
        let chicago = jiff::tz::TimeZone::get("America/Chicago").unwrap();
        let tokyo = jiff::tz::TimeZone::get("Asia/Tokyo").unwrap();
        let at_chicago = crate::calendar::instant_of(kept, &chicago);
        lib.put_reminders(&[
            Reminder {
                raw: raw(None, 0),
                ..reminder("floating", Some(at_chicago), false, None)
            },
            Reminder {
                raw: raw(Some("America/Chicago"), 0),
                timezone: Some("America/Chicago".into()),
                ..reminder("zoned", Some(kept), false, None)
            },
            Reminder {
                raw: raw(None, 1),
                all_day: true,
                ..reminder("day", Some(kept), false, None)
            },
        ])
        .unwrap();
        let due = |lib: &Library, g: &str| lib.reminder(g).unwrap().unwrap().due_at;
        // Read on Chicago's clock already: nothing moves.
        assert_eq!(lib.refloat_reminders(&chicago).unwrap(), 0);
        // The computer now in Tokyo: eight in the evening there.
        assert_eq!(lib.refloat_reminders(&tokyo).unwrap(), 1);
        assert_eq!(due(&lib, "floating"), Some(kept - 9 * 3600));
        assert_eq!(due(&lib, "zoned"), Some(kept));
        assert_eq!(due(&lib, "day"), Some(kept));
    }

    /// An all-day reminder is in Today by its date, whatever the zone: today's is, tomorrow's
    /// is not — though tomorrow's UTC midnight falls before tonight's local midnight west of
    /// Greenwich, which is how it used to land in Today there.
    #[test]
    fn an_all_day_reminder_is_in_today_by_its_date() {
        let zone = jiff::tz::TimeZone::system();
        let today = jiff::Zoned::now().with_time_zone(zone.clone()).date();
        let utc_midnight = |d: jiff::civil::Date| {
            d.to_zoned(jiff::tz::TimeZone::UTC)
                .unwrap()
                .timestamp()
                .as_second()
        };
        let end_of_today = today
            .tomorrow()
            .unwrap()
            .to_zoned(zone)
            .unwrap()
            .timestamp()
            .as_second();
        let mut lib = seeded();
        let mut on_day = reminder("all-day-today", Some(utc_midnight(today)), false, None);
        on_day.all_day = true;
        let mut next_day = reminder(
            "all-day-tomorrow",
            Some(utc_midnight(today.tomorrow().unwrap())),
            false,
            None,
        );
        next_day.all_day = true;
        lib.put_reminders(&[on_day, next_day]).unwrap();
        let guids: Vec<String> = lib
            .reminders_smart(SmartList::Today, end_of_today)
            .unwrap()
            .into_iter()
            .map(|r| r.guid)
            .collect();
        assert_eq!(guids, vec!["all-day-today"]);
        assert_eq!(lib.reminder_counts(end_of_today).unwrap().today, 1);
    }

    /// Overdue belongs in Today: Apple shows it there rather than hiding it in the past.
    #[test]
    fn today_includes_overdue_and_excludes_the_future() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("overdue", Some(DAY_START - 200_000), false, None),
            reminder("today", Some(DAY_START + 3600), false, None),
            reminder("tomorrow", Some(DAY_END + 3600), false, None),
            reminder("undated", None, false, None),
            reminder(
                "done-today",
                Some(DAY_START + 10),
                false,
                Some(DAY_START + 20),
            ),
        ])
        .unwrap();

        let guids: Vec<String> = lib
            .reminders_smart(SmartList::Today, DAY_END)
            .unwrap()
            .into_iter()
            .map(|r| r.guid)
            .collect();
        assert_eq!(guids, vec!["overdue", "today"]);
    }

    #[test]
    fn scheduled_is_everything_dated_and_still_open() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("dated", Some(DAY_END + 99_999), false, None),
            reminder("undated", None, false, None),
        ])
        .unwrap();
        let found = lib.reminders_smart(SmartList::Scheduled, DAY_END).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].guid, "dated");
    }

    #[test]
    fn flagged_shows_only_what_is_flagged_and_open() {
        let mut lib = seeded();
        lib.put_reminders(&[
            reminder("flag", None, true, None),
            reminder("flag-done", None, true, Some(1)),
            reminder("plain", None, false, None),
        ])
        .unwrap();
        let found = lib.reminders_smart(SmartList::Flagged, DAY_END).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].guid, "flag");
    }

    /// Hiding a list must hide it from the smart lists too, or the counts disagree.
    #[test]
    fn a_disabled_list_is_excluded_everywhere() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("a", Some(DAY_START), true, None)])
            .unwrap();
        assert_eq!(
            lib.reminders_smart(SmartList::All, DAY_END).unwrap().len(),
            1
        );

        lib.put_reminder_lists(&[ReminderList {
            guid: "l1".into(),
            title: "Daily".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: false,
            ..Default::default()
        }])
        .unwrap();
        for kind in [
            SmartList::Today,
            SmartList::Scheduled,
            SmartList::Flagged,
            SmartList::All,
            SmartList::Completed,
        ] {
            assert!(lib.reminders_smart(kind, DAY_END).unwrap().is_empty());
        }
    }

    #[test]
    fn flagging_round_trips_through_storage() {
        let mut lib = seeded();
        lib.put_reminders(&[reminder("a", None, true, None)])
            .unwrap();
        assert!(lib.reminders_in("l1", true).unwrap()[0].flagged);
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[
            ReminderList {
                guid: "L1".into(),
                title: "Daily".into(),
                colour: None,
                ctag: None,
                sort_order: 0,
                enabled: true,
                ..Default::default()
            },
            ReminderList {
                guid: "L2".into(),
                title: "Off".into(),
                colour: None,
                ctag: None,
                sort_order: 1,
                enabled: false,
                ..Default::default()
            },
        ])
        .unwrap();
        lib
    }

    fn reminder(guid: &str, list: &str, due: Option<i64>, done: bool, flagged: bool) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: list.into(),
            title: guid.into(),
            notes: None,
            due_at: due,
            completed_at: done.then_some(1),
            priority: 0,
            parent_guid: None,
            flagged,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
            all_day: false,
        }
    }

    /// A count on a tile that disagrees with what opening it shows is worse than no count,
    /// so both come from the same conditions. This holds them to it.
    #[test]
    fn every_count_matches_what_opening_it_shows() {
        let mut lib = library();
        let now = 1_000_000;
        lib.put_reminders(&[
            reminder("overdue", "L1", Some(now - 500), false, false),
            reminder("later", "L1", Some(now + 5_000), false, true),
            reminder("undated", "L1", None, false, false),
            reminder("done", "L1", Some(now - 100), true, false),
        ])
        .unwrap();

        let counts = lib.reminder_counts(now).unwrap();
        for (kind, count) in [
            (SmartList::Today, counts.today),
            (SmartList::Scheduled, counts.scheduled),
            (SmartList::Flagged, counts.flagged),
            (SmartList::All, counts.all),
            (SmartList::Completed, counts.completed),
        ] {
            assert_eq!(
                lib.reminders_smart(kind, now).unwrap().len(),
                count,
                "{kind:?} count must equal what the list shows"
            );
        }
        assert_eq!(counts.today, 1, "overdue counts as today");
        assert_eq!(counts.all, 3, "completed does not count");
        assert_eq!(counts.completed, 1, "Completed holds the one ticked off");
    }

    /// Completed shows the reminders ticked off, the latest first.
    #[test]
    fn completed_lists_the_latest_first() {
        let mut lib = library();
        let mut early = reminder("early", "L1", None, true, false);
        early.completed_at = Some(100);
        let mut late = reminder("late", "L1", None, true, false);
        late.completed_at = Some(900);
        lib.put_reminders(&[early, late, reminder("open", "L1", None, false, false)])
            .unwrap();
        let shown: Vec<String> = lib
            .reminders_smart(SmartList::Completed, 0)
            .unwrap()
            .into_iter()
            .map(|r| r.guid)
            .collect();
        assert_eq!(shown, vec!["late", "early"]);
    }

    /// A list nobody has switched on contributes to nothing.
    #[test]
    fn a_disabled_list_is_left_out_of_the_smart_counts() {
        let mut lib = library();
        lib.put_reminders(&[reminder("hidden", "L2", Some(10), false, true)])
            .unwrap();
        let counts = lib.reminder_counts(1_000_000).unwrap();
        assert_eq!(counts.all, 0);
        assert_eq!(counts.flagged, 0);
    }

    /// Every list appears, including the empty ones — a sidebar that hid them would be
    /// missing rows that are really there.
    #[test]
    fn every_list_gets_a_count_even_at_zero() {
        let mut lib = library();
        lib.put_reminders(&[reminder("a", "L1", None, false, false)])
            .unwrap();
        let counts = lib.reminder_counts(1_000_000).unwrap();
        let by_guid: std::collections::HashMap<_, _> = counts.lists.into_iter().collect();
        assert_eq!(by_guid.get("L1"), Some(&1));
        assert_eq!(by_guid.get("L2"), Some(&0), "an empty list still shows");
    }

    #[test]
    fn an_empty_library_counts_zero_rather_than_failing() {
        let lib = library();
        let counts = lib.reminder_counts(1_000_000).unwrap();
        assert_eq!(
            (counts.today, counts.scheduled, counts.flagged, counts.all),
            (0, 0, 0, 0)
        );
    }
}

/// What a screenful of reminders weighs on its way to the window.
#[cfg(test)]
mod payload_tests {
    use super::*;

    /// Apple's record for one reminder runs to about five kilobytes. It is kept so a write can
    /// echo back every field this app does not model, and it is read on this side only — the
    /// interface names a reminder by its guid and has never looked at a byte of it.
    ///
    /// Sending it anyway made a list of seven hundred weigh three and a half megabytes.
    #[test]
    fn apples_own_record_never_leaves_this_side() {
        let apples_record = format!(r#"{{"fields":{{"padding":"{}"}}}}"#, "x".repeat(5_000));
        let reminder = Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/1".into(),
            title: "Buy milk".into(),
            notes: None,
            due_at: Some(1_789_000_000),
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("e1".into()),
            raw: Some(apples_record),
            timezone: None,
            section_guid: None,
            all_day: false,
        };

        let sent = serde_json::to_string(&reminder).unwrap();
        assert!(
            !sent.contains("padding"),
            "Apple's record must not cross into the window"
        );
        assert!(
            sent.len() < 400,
            "a reminder should weigh a few hundred bytes, not a few thousand: {}",
            sent.len()
        );
        assert!(sent.contains("Buy milk"), "what a person reads still goes");
    }

    /// A thousand reminders is an ordinary list on a busy account, and it has to stay small
    /// enough to cross a message channel without anyone noticing.
    #[test]
    fn a_thousand_reminders_stay_under_a_megabyte() {
        let many: Vec<Reminder> = (0..1_000)
            .map(|i| Reminder {
                guid: format!("Reminder/{i}"),
                list_guid: "List/1".into(),
                title: format!("Something to do number {i}"),
                notes: Some("with a note on it".into()),
                due_at: Some(1_789_000_000 + i),
                completed_at: None,
                priority: 0,
                flagged: false,
                parent_guid: None,
                recur_frequency: None,
                recur_interval: None,
                etag: Some(format!("e{i}")),
                raw: Some("y".repeat(5_000)),
                timezone: None,
                section_guid: None,
                all_day: false,
            })
            .collect();
        let bytes = serde_json::to_vec(&many).unwrap().len();
        assert!(
            bytes < 1_048_576,
            "a thousand reminders came to {bytes} bytes"
        );
    }
}

#[cfg(test)]
mod raw_column_tests {
    use super::*;

    fn with_raw(guid: &str) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "List/1".into(),
            title: "Buy milk".into(),
            notes: None,
            due_at: Some(1_789_000_000),
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: Some("e1".into()),
            raw: Some("z".repeat(5_000)),
            timezone: None,
            section_guid: None,
            all_day: false,
        }
    }

    fn stored() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/1".into(),
            title: "Daily".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        lib.put_reminders(&[with_raw("Reminder/1"), with_raw("Reminder/2")])
            .unwrap();
        lib
    }

    /// Apple's record averages five kilobytes a reminder. Reading it for every row to answer a
    /// question about titles and dates cost about a hundred megabytes on a large account, and
    /// nothing outside the write path has ever wanted it.
    #[test]
    fn a_listed_reminder_does_not_drag_apples_record_along() {
        let lib = stored();
        for listed in [
            lib.reminders_in("List/1", true).unwrap(),
            lib.reminders_due_between(0, i64::MAX).unwrap(),
            lib.reminders_smart(SmartList::All, i64::MAX).unwrap(),
        ] {
            assert!(!listed.is_empty(), "the fixture has reminders in it");
            for r in listed {
                assert_eq!(r.raw, None, "{} carried Apple's record", r.guid);
            }
        }
    }

    /// It is still kept, and still there for the write path that echoes it back.
    #[test]
    fn the_record_is_stored_and_can_be_asked_for() {
        let lib = stored();
        assert_eq!(
            lib.reminder_raw("Reminder/1").unwrap().map(|r| r.len()),
            Some(5_000)
        );
        assert_eq!(lib.reminder_raw("nothing-like-this").unwrap(), None);
    }
}

#[cfg(test)]
mod bound_tests {
    use super::*;

    fn many(n: usize) -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/1".into(),
            title: "Everything".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        let rows: Vec<Reminder> = (0..n)
            .map(|i| Reminder {
                guid: format!("Reminder/{i}"),
                list_guid: "List/1".into(),
                title: format!("Task {i}"),
                notes: None,
                due_at: Some(1_789_000_000 + i as i64),
                completed_at: None,
                priority: 0,
                flagged: true,
                parent_guid: None,
                recur_frequency: None,
                recur_interval: None,
                etag: None,
                raw: None,
                timezone: None,
                section_guid: None,
                all_day: false,
            })
            .collect();
        lib.put_reminders(&rows).unwrap();
        lib
    }

    /// Ten years of Reminders is genuinely unbounded, and the whole of it crossing into the
    /// window is what the plan rules out. Every view that can grow without limit is capped.
    #[test]
    fn no_view_hands_over_more_than_it_should() {
        let lib = many(MOST_AT_ONCE + 500);
        assert_eq!(
            lib.reminders_in("List/1", true).unwrap().len(),
            MOST_AT_ONCE
        );
        assert_eq!(
            lib.reminders_due_between(0, i64::MAX).unwrap().len(),
            MOST_AT_ONCE
        );
        assert_eq!(
            lib.reminders_smart(SmartList::All, i64::MAX).unwrap().len(),
            MOST_AT_ONCE
        );
        assert_eq!(
            lib.reminders_smart(SmartList::Flagged, i64::MAX)
                .unwrap()
                .len(),
            MOST_AT_ONCE
        );
    }

    /// The totals are counted rather than measured off the capped list, so a view that reaches
    /// the cap can still say how many there really are.
    #[test]
    fn the_count_is_the_truth_even_when_the_list_is_not_all_of_it() {
        let lib = many(MOST_AT_ONCE + 500);
        let counts = lib.reminder_counts(i64::MAX).unwrap();
        assert_eq!(counts.all, MOST_AT_ONCE + 500);
        assert_eq!(counts.flagged, MOST_AT_ONCE + 500);
    }

    /// An ordinary account is nowhere near the cap and must be unaffected by it.
    #[test]
    fn an_ordinary_account_sees_everything() {
        let lib = many(40);
        assert_eq!(lib.reminders_in("List/1", true).unwrap().len(), 40);
        assert_eq!(lib.reminders_due_between(0, i64::MAX).unwrap().len(), 40);
    }
}

/// Repeating reminders where the clock moves.
#[cfg(test)]
mod recurrence_daylight_tests {
    use super::*;

    fn at(zone: &str, y: i16, m: i8, d: i8, h: i8, min: i8) -> i64 {
        jiff::civil::DateTime::new(y, m, d, h, min, 0, 0)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::get(zone).unwrap())
            .unwrap()
            .timestamp()
            .as_second()
    }

    fn wall(zone: &str, seconds: i64) -> String {
        jiff::Timestamp::from_second(seconds)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::get(zone).unwrap())
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }

    fn daily(zone: &str, due: i64) -> Reminder {
        Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/1".into(),
            title: "Take the tablets".into(),
            notes: None,
            due_at: Some(due),
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: Some(0),
            recur_interval: Some(1),
            etag: None,
            raw: None,
            timezone: Some(zone.into()),
            section_guid: None,
            all_day: false,
        }
    }

    /// Nine in the morning every day means nine in the morning on the day the clocks go
    /// forward too. Stepping a whole number of hours instead of a day moves it to ten, and
    /// back to nine six months later — twice a year, for everyone who does not live somewhere
    /// that keeps one offset all year. This account's zone is one of the few that does, which
    /// is why it was never seen.
    #[test]
    fn a_daily_reminder_keeps_its_hour_across_a_spring_forward() {
        let zone = "America/New_York";
        let before = at(zone, 2026, 3, 7, 9, 0);
        let next = daily(zone, before)
            .next_occurrence_after(before)
            .expect("a daily reminder has a next occurrence");
        assert_eq!(wall(zone, next), "2026-03-08 09:00");
    }

    fn repeating(zone: &str, frequency: i64, due: i64) -> Reminder {
        Reminder {
            recur_frequency: Some(frequency),
            ..daily(zone, due)
        }
    }

    /// Ticks one after another, each from the date the last one left — as ticks laid here
    /// offline are — with Apple's copy (`raw`) still due where the series began.
    fn ticks(r: &Reminder, apple_due: i64, n: usize) -> Vec<String> {
        let zone = r.timezone.clone().unwrap();
        let mut r = r.clone();
        r.raw = Some(format!(
            r#"{{"fields":{{"DueDate":{{"value":{}}}}}}}"#,
            apple_due * 1000
        ));
        (0..n)
            .map(|_| {
                let due = r.due_at.unwrap();
                let next = r.next_occurrence_after(due).unwrap();
                r.due_at = Some(next);
                wall(&zone, next)
            })
            .collect()
    }

    /// The 31st stays the end of the month: clamped in a shorter one, and back on the 31st
    /// after it. Added to the last date, it became the 28th in February and stayed there.
    #[test]
    fn a_monthly_reminder_on_the_31st_never_drifts() {
        let zone = "Europe/London";
        let jan31 = at(zone, 2026, 1, 31, 9, 0);
        assert_eq!(
            ticks(&repeating(zone, 2, jan31), jan31, 4),
            [
                "2026-02-28 09:00",
                "2026-03-31 09:00",
                "2026-04-30 09:00",
                "2026-05-31 09:00"
            ]
        );
        // Caught up over several months at once, from a long way behind: the same days.
        let r = repeating(zone, 2, jan31);
        let next = r.next_occurrence_after(at(zone, 2026, 4, 1, 0, 0)).unwrap();
        assert_eq!(wall(zone, next), "2026-04-30 09:00");
        let next = r.next_occurrence_after(at(zone, 2026, 5, 1, 0, 0)).unwrap();
        assert_eq!(wall(zone, next), "2026-05-31 09:00");
    }

    /// 29 February: the 28th in a common year, the 29th again in a leap one.
    #[test]
    fn a_yearly_reminder_on_the_29th_of_february_comes_back_in_leap_years() {
        let zone = "America/New_York";
        let leap = at(zone, 2028, 2, 29, 8, 30);
        assert_eq!(
            ticks(&repeating(zone, 3, leap), leap, 4),
            [
                "2029-02-28 08:30",
                "2030-02-28 08:30",
                "2031-02-28 08:30",
                "2032-02-29 08:30"
            ]
        );
    }

    /// The month steps keep the hour across a change of the clocks, as the days do.
    #[test]
    fn a_monthly_reminder_keeps_its_hour_across_the_clocks_changing() {
        let zone = "America/New_York";
        let feb = at(zone, 2026, 2, 15, 9, 0);
        assert_eq!(
            ticks(&repeating(zone, 2, feb), feb, 2),
            ["2026-03-15 09:00", "2026-04-15 09:00"]
        );
    }

    /// And on the day they go back.
    #[test]
    fn a_daily_reminder_keeps_its_hour_across_a_fall_back() {
        let zone = "America/New_York";
        let before = at(zone, 2026, 10, 31, 9, 0);
        let next = daily(zone, before).next_occurrence_after(before).unwrap();
        assert_eq!(wall(zone, next), "2026-11-01 09:00");
    }

    /// Southern hemisphere, and a zone whose clock moves by half an hour rather than a whole
    /// one — the case that catches anything reasoning in whole hours.
    #[test]
    fn the_hour_holds_in_the_awkward_zones_too() {
        for (zone, y, m, d) in [
            ("Australia/Sydney", 2026, 10, 3),
            ("Australia/Lord_Howe", 2026, 10, 3),
            ("Europe/London", 2026, 3, 28),
            ("Pacific/Chatham", 2026, 9, 26),
        ] {
            let before = at(zone, y, m, d, 9, 0);
            let next = daily(zone, before).next_occurrence_after(before).unwrap();
            assert!(
                wall(zone, next).ends_with("09:00"),
                "{zone}: {} became {}",
                wall(zone, before),
                wall(zone, next)
            );
        }
    }

    /// A reminder that never said where it was must still advance, in this machine's own zone.
    #[test]
    fn a_reminder_with_no_zone_still_advances_a_whole_day() {
        let mut r = daily("UTC", 1_789_000_000);
        r.timezone = None;
        let next = r.next_occurrence_after(1_789_000_000).unwrap();
        assert!(next > 1_789_000_000);
        assert!((next - 1_789_000_000 - 86_400).abs() <= 3_600);
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;

    fn reminder(guid: &str, list: &str) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: list.into(),
            title: guid.into(),
            notes: None,
            due_at: None,
            completed_at: None,
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
            all_day: false,
        }
    }

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/L".into(),
            title: "Life Goals".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        lib.put_reminders(&[
            reminder("Reminder/A", "List/L"),
            reminder("Reminder/B", "List/L"),
            reminder("Reminder/C", "List/L"),
        ])
        .unwrap();
        lib.put_reminder_sections(
            &[ReminderSection {
                guid: "ListSection/S".into(),
                list_guid: "List/L".into(),
                title: "🌅".into(),
                created_at: Some(1),
            }],
            &[],
        )
        .unwrap();
        lib
    }

    fn section_of(lib: &Library, guid: &str) -> Option<String> {
        lib.reminders_in("List/L", true)
            .unwrap()
            .into_iter()
            .find(|r| r.guid == guid)
            .and_then(|r| r.section_guid)
    }

    /// The membership file is the whole truth for its list: placed reminders go in, and one
    /// it no longer places comes back out.
    #[test]
    fn the_membership_file_decides_and_is_the_whole_truth() {
        let mut lib = library();
        lib.set_reminder_sections(
            "List/L",
            &[
                ("Reminder/A".into(), "ListSection/S".into()),
                ("Reminder/B".into(), "ListSection/S".into()),
            ],
        )
        .unwrap();
        assert_eq!(
            section_of(&lib, "Reminder/A").as_deref(),
            Some("ListSection/S")
        );
        assert_eq!(section_of(&lib, "Reminder/C"), None);

        lib.set_reminder_sections("List/L", &[("Reminder/A".into(), "ListSection/S".into())])
            .unwrap();
        assert_eq!(
            section_of(&lib, "Reminder/B"),
            None,
            "moved out on the phone, out here"
        );
    }

    /// A sync rewrites reminders wholesale and knows nothing about sections; it must not
    /// knock them out of the ones they sit in.
    #[test]
    fn storing_reminders_again_keeps_their_sections() {
        let mut lib = library();
        lib.set_reminder_sections("List/L", &[("Reminder/A".into(), "ListSection/S".into())])
            .unwrap();
        lib.put_reminders(&[reminder("Reminder/A", "List/L")])
            .unwrap();
        assert_eq!(
            section_of(&lib, "Reminder/A").as_deref(),
            Some("ListSection/S")
        );
    }

    /// A placement naming a section deleted since is a heading nobody can see.
    #[test]
    fn a_placement_in_a_section_that_is_gone_places_nothing() {
        let mut lib = library();
        lib.set_reminder_sections(
            "List/L",
            &[("Reminder/A".into(), "ListSection/GONE".into())],
        )
        .unwrap();
        assert_eq!(section_of(&lib, "Reminder/A"), None);
    }

    /// Deleting a section leaves its reminders in the list, without a heading.
    #[test]
    fn deleting_a_section_keeps_its_reminders() {
        let mut lib = library();
        lib.set_reminder_sections("List/L", &[("Reminder/A".into(), "ListSection/S".into())])
            .unwrap();
        lib.put_reminder_sections(&[], &["ListSection/S".into()])
            .unwrap();
        assert!(lib.reminder_sections("List/L").unwrap().is_empty());
        assert_eq!(section_of(&lib, "Reminder/A"), None);
        assert_eq!(lib.reminders_in("List/L", true).unwrap().len(), 3);
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn r(guid: &str, title: &str, notes: Option<&str>, done: bool) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: "List/L".into(),
            title: title.into(),
            notes: notes.map(ToString::to_string),
            due_at: None,
            completed_at: done.then_some(1),
            priority: 0,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
            all_day: false,
        }
    }

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/L".into(),
            title: "GYM".into(),
            colour: None,
            ctag: None,
            sort_order: 0,
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        lib.put_reminders(&[
            r("A", "Evening run", Some("Legs today"), false),
            r("B", "Water the plants", None, false),
            r("C", "Evening walk", None, true),
            r("D", "Café run", None, false),
        ])
        .unwrap();
        lib
    }

    fn found(q: &str) -> Vec<String> {
        library()
            .search_reminders(q)
            .unwrap()
            .into_iter()
            .map(|r| r.guid)
            .collect()
    }

    #[test]
    fn title_and_notes_are_both_searched() {
        assert_eq!(found("plants"), vec!["B"]);
        assert_eq!(found("legs"), vec!["A"]);
    }

    #[test]
    fn finished_reminders_are_not_found() {
        assert_eq!(found("evening"), vec!["A"]);
    }

    #[test]
    fn case_folds_beyond_ascii_and_every_word_counts() {
        assert_eq!(found("CAFÉ"), vec!["D"]);
        assert_eq!(found("cafe"), vec!["D"]);
        assert_eq!(found("run evening"), vec!["A"]);
        assert!(found("evening plants").is_empty());
        assert!(found("  ").is_empty());
    }
}

/// A list's reminders in the order the list itself asks for.
#[cfg(test)]
mod list_sorting_tests {
    use super::*;

    fn reminder(guid: &str, title: &str, due: Option<i64>, priority: i64) -> Reminder {
        Reminder {
            guid: format!("Reminder/{guid}"),
            list_guid: "List/L".into(),
            title: title.into(),
            notes: None,
            due_at: due,
            completed_at: None,
            priority,
            flagged: false,
            parent_guid: None,
            recur_frequency: None,
            recur_interval: None,
            etag: None,
            raw: None,
            timezone: None,
            section_guid: None,
            all_day: false,
        }
    }

    fn library(sorting: &str, order: &[&str]) -> Library {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/L".into(),
            title: "L".into(),
            enabled: true,
            sorting: Some(sorting.into()),
            reminder_order: order.iter().map(ToString::to_string).collect(),
            ..Default::default()
        }])
        .unwrap();
        lib.put_reminders(&[
            reminder("aaa", "Charlie", Some(300), 0),
            reminder("bbb", "alpha", None, 1),
            reminder("ccc", "Bravo", Some(100), 9),
            // Made somewhere that did not add it to the list's order.
            reminder("ddd", "Delta", Some(200), 5),
        ])
        .unwrap();
        lib
    }

    fn titles(lib: &Library) -> Vec<String> {
        lib.reminders_in("List/L", false)
            .unwrap()
            .into_iter()
            .map(|r| r.title)
            .collect()
    }

    /// By hand is the list's own order. It came out sorted by date, which is only right for
    /// a list that asks for that.
    #[test]
    fn a_list_sorted_by_hand_keeps_its_order_and_puts_strays_last() {
        let lib = library("manual", &["CCC", "aaa", "BBB"]);
        assert_eq!(titles(&lib), ["Bravo", "Charlie", "alpha", "Delta"]);
    }

    #[test]
    fn a_list_sorted_by_date_puts_the_undated_last() {
        assert_eq!(
            titles(&library("displayDate_asc", &[])),
            ["Bravo", "Delta", "Charlie", "alpha"]
        );
        assert_eq!(
            titles(&library("displayDate_desc", &[])),
            ["Charlie", "Delta", "Bravo", "alpha"]
        );
    }

    #[test]
    fn title_and_priority_sort_as_the_phone_does() {
        assert_eq!(
            titles(&library("title_asc", &[])),
            ["alpha", "Bravo", "Charlie", "Delta"]
        );
        // Highest first: 1 is high, 5 medium, 9 low, 0 none.
        assert_eq!(
            titles(&library("priority_desc", &[])),
            ["alpha", "Delta", "Bravo", "Charlie"]
        );
    }
}

// ---------------------------------------------------------------------------------------
// Details Apple keeps beside a reminder: tags, a link, early alerts, custom smart lists.
// ---------------------------------------------------------------------------------------

/// One tag on one reminder: Apple's `Hashtag` record. The name is stored without the `#`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderHashtag {
    pub guid: String,
    pub reminder_guid: String,
    pub name: String,
}

/// One attachment on a reminder: Apple's `Attachment` record. Only `URL` ones carry a link;
/// `File` and `Image` are listed so their presence is known, never their bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderAttachment {
    pub guid: String,
    pub reminder_guid: String,
    /// Apple's `Type`: `URL`, `File` or `Image`.
    pub kind: String,
    pub url: Option<String>,
    pub uti: Option<String>,
}

/// An early alert ("remind me 30 minutes early"), one entry of the reminder's
/// `DueDateDeltaAlertsData`. `count` is how far before the due date, as a positive number;
/// Apple stores it negative. `unit` is Apple's: 0 minutes (the only one on the account
/// measured), then, by the order iOS offers them, 1 hours, 2 days, 3 weeks, 4 months.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EarlyAlert {
    pub id: String,
    pub count: i64,
    pub unit: i64,
}

/// Everything about one reminder this section reads, for its detail view.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderDetail {
    pub guid: String,
    /// Tag names, without `#`, sorted as the phone shows them.
    pub tags: Vec<String>,
    pub url: Option<String>,
    pub early_alerts: Vec<EarlyAlert>,
    /// How many file or image attachments it has, pictures included.
    pub other_attachments: usize,
    /// Its pictures' attachment ids (`Attachment/<UUID>`), in the order the phone lists them.
    pub images: Vec<String>,
}

/// A tag across the whole account, and how many reminders still to do carry it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub name: String,
    pub open: usize,
}

/// A `SmartList` record: a built-in one (`today`, `flagged`, `assigned`…) or a custom one,
/// whose filter is kept as Apple's JSON and decoded with [`SmartFilter::decode`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderSmartList {
    pub guid: String,
    /// The last part of Apple's `SmartListType`: `custom`, `today`, `flagged`, `assigned`…
    pub kind: String,
    pub title: Option<String>,
    pub colour: Option<String>,
    pub emblem: Option<String>,
    pub pinned: bool,
    pub sorting: Option<String>,
    /// `FilterData` as Apple wrote it (JSON), for a custom list.
    pub filter: Option<String>,
}

/// What one sync pass read of the details. A whole pass replaces everything stored; an
/// incremental one lays itself over it.
#[derive(Debug, Clone, Default)]
pub struct ReminderDetailsPass {
    pub whole: bool,
    pub hashtags: Vec<ReminderHashtag>,
    pub attachments: Vec<ReminderAttachment>,
    /// Every reminder record in the pass with its early alerts — an empty list included, so a
    /// reminder whose alert was taken off on the phone loses it here — and the account id
    /// Apple writes inside them.
    pub early_alerts: Vec<(String, Option<String>, Vec<EarlyAlert>)>,
    pub smart_lists: Vec<ReminderSmartList>,
    /// Names of records Apple reports gone; any of the four kinds.
    pub gone: Vec<String>,
}

/// A custom smart list's filter, decoded from `FilterData`.
///
/// Only `{"priorities":["medium"]}` has been seen on a real account. The other keys are read
/// from the shapes Apple's filter editor offers (tags, date, priority, flagged, lists, and
/// whether all or any must match), leniently; anything not understood is named in `unknown`
/// rather than guessed at, and [`Library::reminders_in_smart_list`] says so.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SmartFilter {
    /// All criteria must hold (Apple's default), or any one.
    pub match_all: bool,
    pub tags: Option<TagFilter>,
    pub date: Option<DateFilter>,
    /// Apple's scale: 1 high, 5 medium, 9 low, 0 none.
    pub priorities: Vec<i64>,
    pub flagged: Option<bool>,
    /// List record names.
    pub lists: Vec<String>,
    /// Keys, or values, this decoder did not understand.
    pub unknown: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TagFilter {
    /// Lower-cased, without `#`.
    pub names: Vec<String>,
    /// Every named tag (true) or any one of them.
    pub all: bool,
    /// "Any tag": a reminder carrying at least one.
    pub any_tag: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DateFilter {
    /// Has any due date.
    Any,
    /// Due today (overdue counts, as in Today).
    Today,
    /// Due before now and not done.
    Overdue,
    /// Due within `[from, to)`, Unix seconds.
    Range { from: i64, to: i64 },
    /// Due within the next `count` units from the start of today (`past`: the last ones).
    Relative {
        count: i64,
        unit: String,
        past: bool,
    },
}

/// Seconds between the Unix epoch and Apple's.
const APPLE_EPOCH: i64 = 978_307_200;

/// A date in a filter as Unix seconds, whichever clock Apple wrote it in.
fn filter_moment(v: &serde_json::Value) -> Option<i64> {
    #[allow(clippy::cast_possible_truncation)]
    let n = v.as_f64()? as i64;
    Some(if n > 100_000_000_000 {
        n / 1000
    } else if n < 1_200_000_000 {
        // Apple's epoch: seconds since 2001, which stays below this until 2039.
        n + APPLE_EPOCH
    } else {
        n
    })
}

fn priority_level(word: &str) -> Option<i64> {
    match word.to_ascii_lowercase().as_str() {
        "high" => Some(1),
        "medium" => Some(5),
        "low" => Some(9),
        "none" => Some(0),
        _ => None,
    }
}

/// Whether `priority` falls in the band `level` names, as EventKit bands them.
fn in_priority_band(priority: i64, level: i64) -> bool {
    match level {
        1 => (1..=4).contains(&priority),
        5 => priority == 5,
        9 => (6..=9).contains(&priority),
        _ => priority == 0,
    }
}

fn tag_key(name: &str) -> String {
    name.trim().trim_start_matches('#').to_lowercase()
}

fn strings_in(v: &serde_json::Value) -> Vec<String> {
    let one = |x: &serde_json::Value| -> Option<String> {
        x.as_str().map(ToString::to_string).or_else(|| {
            ["name", "identifier", "id", "recordName"]
                .iter()
                .find_map(|k| x.get(*k).and_then(|s| s.as_str()).map(ToString::to_string))
        })
    };
    match v {
        serde_json::Value::Array(items) => items.iter().filter_map(one).collect(),
        other => one(other).into_iter().collect(),
    }
}

/// "and"/"all" is all; "or"/"any" is any.
fn all_or_any(v: &serde_json::Value) -> Option<bool> {
    let word = v.as_str()?.to_ascii_lowercase();
    if word.contains("all") || word == "and" {
        Some(true)
    } else if word.contains("any") || word == "or" {
        Some(false)
    } else {
        None
    }
}

impl SmartFilter {
    /// Decode Apple's `FilterData` JSON. Never fails: what is not understood is listed.
    #[must_use]
    pub fn decode(json: &str) -> Self {
        let mut out = SmartFilter {
            match_all: true,
            ..Default::default()
        };
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(json)
        else {
            out.unknown.push("(not JSON)".into());
            return out;
        };
        for (key, value) in &map {
            match key.as_str() {
                "minimumSupportedVersion" | "version" => {}
                "operation" | "matchingOperation" | "combination" | "match" => {
                    match all_or_any(value) {
                        Some(all) => out.match_all = all,
                        None => out.unknown.push(key.clone()),
                    }
                }
                "priorities" | "priority" => {
                    for word in strings_in(value) {
                        match priority_level(&word) {
                            Some(level) => out.priorities.push(level),
                            None => out.unknown.push(format!("priority {word}")),
                        }
                    }
                    if let Some(n) = value.as_array() {
                        for level in n.iter().filter_map(serde_json::Value::as_i64) {
                            out.priorities.push(level);
                        }
                    }
                }
                "flagged" | "isFlagged" => match value {
                    serde_json::Value::Bool(b) => out.flagged = Some(*b),
                    serde_json::Value::Object(o) => {
                        match o.values().find_map(serde_json::Value::as_bool) {
                            Some(b) => out.flagged = Some(b),
                            None => out.unknown.push(key.clone()),
                        }
                    }
                    serde_json::Value::Number(n) => out.flagged = Some(n.as_i64() != Some(0)),
                    _ => out.unknown.push(key.clone()),
                },
                "lists" | "listIDs" | "listIdentifiers" => {
                    let ids = match value {
                        serde_json::Value::Object(o) => o
                            .iter()
                            .filter(|(k, _)| k.to_ascii_lowercase().contains("list"))
                            .flat_map(|(_, v)| strings_in(v))
                            .collect(),
                        other => strings_in(other),
                    };
                    out.lists.extend(ids.into_iter().map(|id| {
                        if id.contains('/') {
                            id
                        } else {
                            format!("List/{}", id.to_ascii_uppercase())
                        }
                    }));
                }
                "hashtags" | "tags" | "hashtagLabels" => {
                    out.tags = Some(decode_tags(value, &mut out.unknown))
                }
                "date" | "dueDate" => out.date = decode_date(value, &mut out.unknown),
                other => out.unknown.push(other.to_string()),
            }
        }
        out
    }

    /// Whether this filter would show a reminder, given its tags (lower-cased) and `now`.
    fn holds(&self, r: &Reminder, tags: &[String], now: i64) -> bool {
        let mut checks: Vec<bool> = Vec::new();
        if let Some(t) = &self.tags {
            let has = |n: &String| tags.contains(n);
            checks.push(if t.any_tag && t.names.is_empty() {
                !tags.is_empty()
            } else if t.all {
                t.names.iter().all(has)
            } else {
                t.names.iter().any(has)
            });
        }
        if let Some(d) = &self.date {
            checks.push(date_holds(d, r, now));
        }
        if !self.priorities.is_empty() {
            checks.push(
                self.priorities
                    .iter()
                    .any(|&level| in_priority_band(r.priority, level)),
            );
        }
        if let Some(f) = self.flagged {
            checks.push(r.flagged == f);
        }
        if !self.lists.is_empty() {
            checks.push(
                self.lists
                    .iter()
                    .any(|l| l.eq_ignore_ascii_case(&r.list_guid)),
            );
        }
        if checks.is_empty() {
            return false;
        }
        if self.match_all {
            checks.iter().all(|c| *c)
        } else {
            checks.iter().any(|c| *c)
        }
    }
}

fn decode_tags(value: &serde_json::Value, unknown: &mut Vec<String>) -> TagFilter {
    let mut t = TagFilter {
        all: false,
        ..Default::default()
    };
    match value {
        serde_json::Value::Object(o) => {
            for (k, v) in o {
                let lower = k.to_ascii_lowercase();
                if lower.contains("operation") || lower == "mode" || lower == "match" {
                    match all_or_any(v) {
                        Some(all) => t.all = all,
                        None => unknown.push(format!("tags.{k}")),
                    }
                } else if lower.contains("any") && v.as_bool().is_some() {
                    t.any_tag = v.as_bool().unwrap_or(false);
                } else if v.is_array() || v.is_string() {
                    t.names.extend(strings_in(v).iter().map(|n| tag_key(n)));
                } else {
                    unknown.push(format!("tags.{k}"));
                }
            }
        }
        serde_json::Value::Bool(true) => t.any_tag = true,
        other => t.names.extend(strings_in(other).iter().map(|n| tag_key(n))),
    }
    t.names.sort();
    t.names.dedup();
    t
}

fn decode_date(value: &serde_json::Value, unknown: &mut Vec<String>) -> Option<DateFilter> {
    if value.as_bool() == Some(true) {
        return Some(DateFilter::Any);
    }
    let kind = value
        .as_str()
        .or_else(|| {
            ["type", "kind", "dateType"]
                .iter()
                .find_map(|k| value.get(*k)?.as_str())
        })
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let get = |names: &[&str]| {
        names
            .iter()
            .find_map(|k| value.get(*k))
            .and_then(filter_moment)
    };
    let from = get(&["start", "startDate", "from", "lowerBound"]);
    let to = get(&["end", "endDate", "to", "upperBound"]);
    if let (Some(from), Some(to)) = (from, to) {
        return Some(DateFilter::Range { from, to });
    }
    let count = ["count", "value", "amount"]
        .iter()
        .find_map(|k| value.get(*k)?.as_i64());
    let unit = ["unit", "component"]
        .iter()
        .find_map(|k| value.get(*k))
        .and_then(|u| {
            u.as_str().map(str::to_ascii_lowercase).or_else(|| {
                u.as_i64().and_then(|n| {
                    ["day", "week", "month", "year"]
                        .get(usize::try_from(n).ok()?)
                        .map(|s| (*s).to_string())
                })
            })
        });
    match (kind.as_str(), count, unit) {
        (k, Some(count), Some(unit)) if !k.contains("today") => Some(DateFilter::Relative {
            count,
            unit,
            past: k.contains("past") || k.contains("last") || count < 0,
        }),
        (k, _, _) if k.contains("today") => Some(DateFilter::Today),
        (k, _, _) if k.contains("overdue") || k.contains("past") => Some(DateFilter::Overdue),
        (k, _, _) if k.contains("any") || k == "scheduled" || k == "hasdate" => {
            Some(DateFilter::Any)
        }
        _ => {
            unknown.push(format!("date {value}"));
            None
        }
    }
}

fn date_holds(d: &DateFilter, r: &Reminder, now: i64) -> bool {
    let Some(due) = r.due_at else {
        return false;
    };
    let zone = jiff::tz::TimeZone::system();
    let today = jiff::Timestamp::from_second(now)
        .ok()
        .map(|t| t.to_zoned(zone.clone()).date());
    let day_start = |date: jiff::civil::Date| {
        date.to_zoned(zone.clone())
            .map_or(now, |z| z.timestamp().as_second())
    };
    match d {
        DateFilter::Any => true,
        DateFilter::Overdue => due < now,
        DateFilter::Today => {
            let end = today.and_then(|t| t.tomorrow().ok()).map_or(now, day_start);
            if r.all_day {
                due < all_day_bound(end)
            } else {
                due < end
            }
        }
        DateFilter::Range { from, to } => due >= *from && due < *to,
        DateFilter::Relative { count, unit, past } => {
            let Some(today) = today else { return false };
            let n = count.abs().clamp(0, 10_000);
            let span = match unit.as_str() {
                u if u.starts_with("week") => jiff::Span::new().weeks(n),
                u if u.starts_with("month") => jiff::Span::new().months(n),
                u if u.starts_with("year") => jiff::Span::new().years(n),
                _ => jiff::Span::new().days(n),
            };
            let start = day_start(today);
            if *past {
                let from = today.checked_sub(span).map_or(start, day_start);
                due >= from && due < start
            } else {
                let to = today.checked_add(span).map_or(start, day_start);
                due >= start && due < to
            }
        }
    }
}

/// What the Reminders sidebar and rows show beside the lists: each reminder still to do with its
/// tags, and how many reminders each custom smart list holds, by the list's id.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderExtras {
    pub tags_by_reminder: std::collections::HashMap<String, Vec<String>>,
    pub custom_counts: std::collections::HashMap<String, usize>,
}

/// What a custom smart list holds now, and which parts of its filter could not be read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SmartListContents {
    pub reminders: Vec<Reminder>,
    /// Non-empty when part of Apple's filter was not understood: those parts were left out,
    /// so the list may hold more or fewer than the phone shows.
    pub unknown: Vec<String>,
}

impl Library {
    /// Store one pass of reminder details. See [`ReminderDetailsPass`].
    pub fn put_reminder_details(&mut self, pass: &ReminderDetailsPass) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            if pass.whole {
                tx.execute_batch(
                    "DELETE FROM reminder_hashtags; DELETE FROM reminder_attachments;
                     DELETE FROM reminder_smart_lists;",
                )?;
            }
            let stamp = now();
            for gone in &pass.gone {
                tx.execute(
                    "DELETE FROM reminder_hashtags WHERE guid = ?1",
                    params![gone],
                )?;
                tx.execute(
                    "DELETE FROM reminder_attachments WHERE guid = ?1",
                    params![gone],
                )?;
                tx.execute(
                    "DELETE FROM reminder_smart_lists WHERE guid = ?1",
                    params![gone],
                )?;
                tx.execute(
                    "DELETE FROM reminder_early_alerts WHERE reminder_guid = ?1",
                    params![gone],
                )?;
            }
            let mut tag = tx.prepare(
                "INSERT INTO reminder_hashtags (guid, reminder_guid, name, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(guid) DO UPDATE SET reminder_guid = excluded.reminder_guid,
                   name = excluded.name, updated_at = excluded.updated_at",
            )?;
            for h in &pass.hashtags {
                tag.execute(params![h.guid, h.reminder_guid, h.name, stamp])?;
            }
            let mut attach = tx.prepare(
                "INSERT INTO reminder_attachments (guid, reminder_guid, kind, url, uti, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(guid) DO UPDATE SET reminder_guid = excluded.reminder_guid,
                   kind = excluded.kind, url = excluded.url, uti = excluded.uti,
                   updated_at = excluded.updated_at",
            )?;
            for a in &pass.attachments {
                attach.execute(params![
                    a.guid,
                    a.reminder_guid,
                    a.kind,
                    a.url,
                    a.uti,
                    stamp
                ])?;
            }
            let mut clear =
                tx.prepare("DELETE FROM reminder_early_alerts WHERE reminder_guid = ?1")?;
            let mut alert = tx.prepare(
                "INSERT OR REPLACE INTO reminder_early_alerts (id, reminder_guid, count, unit, account_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (reminder, account, alerts) in &pass.early_alerts {
                clear.execute(params![reminder])?;
                for a in alerts {
                    alert.execute(params![a.id, reminder, a.count, a.unit, account])?;
                }
            }
            let mut smart = tx.prepare(
                "INSERT INTO reminder_smart_lists (guid, kind, title, colour, emblem, pinned,
                                                   sorting, filter, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(guid) DO UPDATE SET kind = excluded.kind, title = excluded.title,
                   colour = excluded.colour, emblem = excluded.emblem, pinned = excluded.pinned,
                   sorting = excluded.sorting, filter = excluded.filter,
                   updated_at = excluded.updated_at",
            )?;
            for s in &pass.smart_lists {
                smart.execute(params![
                    s.guid,
                    s.kind,
                    s.title,
                    s.colour,
                    s.emblem,
                    i64::from(s.pinned),
                    s.sorting,
                    s.filter,
                    stamp
                ])?;
            }
            // A reminder that is gone takes its details with it.
            tx.execute_batch(
                "DELETE FROM reminder_hashtags WHERE reminder_guid NOT IN (SELECT guid FROM reminders);
                 DELETE FROM reminder_attachments WHERE reminder_guid NOT IN (SELECT guid FROM reminders);
                 DELETE FROM reminder_early_alerts WHERE reminder_guid NOT IN (SELECT guid FROM reminders);",
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// One reminder's tags, link and early alerts.
    pub fn reminder_detail(&self, guid: &str) -> Result<ReminderDetail> {
        let mut tags: Vec<String> = {
            let mut stmt = self
                .conn
                .prepare("SELECT DISTINCT name FROM reminder_hashtags WHERE reminder_guid = ?1")?;
            let rows = stmt.query_map(params![guid], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        tags.sort_by(|a, b| crate::fold::natural(a, b));
        tags.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        let mut stmt = self.conn.prepare(
            "SELECT kind, url, guid FROM reminder_attachments WHERE reminder_guid = ?1 ORDER BY guid",
        )?;
        let attachments = stmt
            .query_map(params![guid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let url = attachments
            .iter()
            .find(|(kind, url, _)| kind == "URL" && url.is_some())
            .and_then(|(_, url, _)| url.clone());
        let other_attachments = attachments.iter().filter(|(k, _, _)| k != "URL").count();
        let images = attachments
            .iter()
            .filter(|(k, _, _)| k == "Image")
            .map(|(_, _, id)| id.clone())
            .collect();
        let mut stmt = self.conn.prepare(
            "SELECT id, count, unit FROM reminder_early_alerts WHERE reminder_guid = ?1
             ORDER BY unit, count, id",
        )?;
        let early_alerts = stmt
            .query_map(params![guid], |r| {
                Ok(EarlyAlert {
                    id: r.get(0)?,
                    count: r.get(1)?,
                    unit: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ReminderDetail {
            guid: guid.to_string(),
            tags,
            url,
            early_alerts,
            other_attachments,
            images,
        })
    }

    /// One reminder's tag records, for a write that removes one by name.
    pub fn reminder_hashtags(&self, guid: &str) -> Result<Vec<ReminderHashtag>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, reminder_guid, name FROM reminder_hashtags WHERE reminder_guid = ?1",
        )?;
        let rows = stmt.query_map(params![guid], |r| {
            Ok(ReminderHashtag {
                guid: r.get(0)?,
                reminder_guid: r.get(1)?,
                name: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Every tag on the account, as the tag browser lists them: one entry per name however it
    /// is capitalised, with how many reminders still to do carry it.
    pub fn reminder_tags(&self) -> Result<Vec<TagCount>> {
        let mut stmt = self.conn.prepare(
            "SELECT h.name, h.reminder_guid, r.completed_at IS NULL AND l.enabled = 1
             FROM reminder_hashtags h
             JOIN reminders r ON r.guid = h.reminder_guid
             JOIN reminder_lists l ON l.guid = r.list_guid",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)? != 0,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut by: std::collections::BTreeMap<
            String,
            (String, std::collections::HashSet<String>),
        > = std::collections::BTreeMap::new();
        for (name, reminder, open) in rows {
            let entry = by
                .entry(tag_key(&name))
                .or_insert_with(|| (name.clone(), std::collections::HashSet::new()));
            if open {
                entry.1.insert(reminder);
            }
        }
        let mut out: Vec<TagCount> = by
            .into_values()
            .map(|(name, open)| TagCount {
                name,
                open: open.len(),
            })
            .collect();
        out.sort_by(|a, b| crate::fold::natural(&a.name, &b.name));
        Ok(out)
    }

    /// Reminders still to do carrying a tag, matched however it is capitalised.
    pub fn reminders_tagged(&self, name: &str) -> Result<Vec<Reminder>> {
        let key = tag_key(name);
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM reminders r JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.completed_at IS NULL AND r.guid IN
               (SELECT reminder_guid FROM reminder_hashtags WHERE lower(name) = ?1)
             ORDER BY r.due_at IS NULL, r.due_at, r.priority DESC, r.title
             LIMIT {MOST_AT_ONCE}",
            prefixed_columns()
        ))?;
        // SQLite lowers ASCII only; the match is checked again in Rust below for the rest.
        let rows = stmt.query_map(params![key], reminder_row)?;
        let mut found: Vec<Reminder> = rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        if found.is_empty() && !key.is_ascii() {
            let tagged: std::collections::HashSet<String> = self
                .all_tags_by_reminder()?
                .into_iter()
                .filter(|(_, tags)| tags.contains(&key))
                .map(|(r, _)| r)
                .collect();
            found = self
                .open_reminders()?
                .into_iter()
                .filter(|r| tagged.contains(&r.guid))
                .take(MOST_AT_ONCE)
                .collect();
        }
        Ok(found)
    }

    fn all_tags_by_reminder(&self) -> Result<std::collections::HashMap<String, Vec<String>>> {
        let mut stmt = self
            .conn
            .prepare("SELECT reminder_guid, name FROM reminder_hashtags")?;
        let mut by: std::collections::HashMap<String, Vec<String>> = Default::default();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (reminder, name) = row?;
            by.entry(reminder).or_default().push(tag_key(&name));
        }
        Ok(by)
    }

    fn open_reminders(&self) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM reminders r JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.completed_at IS NULL
             ORDER BY r.due_at IS NULL, r.due_at, r.priority DESC, r.title",
            prefixed_columns()
        ))?;
        let rows = stmt.query_map([], reminder_row)?;
        Ok(rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|(r, _)| r)
            .collect())
    }

    /// The account's smart lists, built-in and custom, as Apple holds them.
    pub fn reminder_smart_lists(&self) -> Result<Vec<ReminderSmartList>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, kind, title, colour, emblem, pinned, sorting, filter
             FROM reminder_smart_lists ORDER BY kind != 'custom', title, guid",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ReminderSmartList {
                guid: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                colour: r.get(3)?,
                emblem: r.get(4)?,
                pinned: r.get::<_, i64>(5)? != 0,
                sorting: r.get(6)?,
                filter: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// What a custom smart list holds at `now`, worked out here from its filter.
    pub fn reminders_in_smart_list(&self, guid: &str, now: i64) -> Result<SmartListContents> {
        let filter = self
            .reminder_smart_lists()?
            .into_iter()
            .find(|s| s.guid == guid)
            .and_then(|s| s.filter);
        let Some(filter) = filter else {
            return Ok(SmartListContents {
                reminders: Vec::new(),
                unknown: vec!["(no filter)".into()],
            });
        };
        let filter = SmartFilter::decode(&filter);
        Ok(SmartListContents {
            reminders: self.reminders_matching(&filter, now)?,
            unknown: filter.unknown,
        })
    }

    /// Reminders still to do that a decoded filter holds.
    pub fn reminders_matching(&self, filter: &SmartFilter, now: i64) -> Result<Vec<Reminder>> {
        let tags = self.all_tags_by_reminder()?;
        let none: Vec<String> = Vec::new();
        Ok(self
            .open_reminders()?
            .into_iter()
            .filter(|r| filter.holds(r, tags.get(&r.guid).unwrap_or(&none), now))
            .take(MOST_AT_ONCE)
            .collect())
    }

    /// Each reminder still to do with its tags, named as [`Library::reminder_tags`] names them
    /// and in that order, and how many reminders each custom smart list holds now — what the
    /// Reminders sidebar and rows show beside the lists, read in one go. The window used to ask
    /// for every tag's reminders whole, and every smart list's, only to build this.
    pub fn reminder_extras(&self, now: i64) -> Result<ReminderExtras> {
        // Tags as the sidebar lists them: one name per tag however it is capitalised.
        let tags = self.reminder_tags()?;
        let order: std::collections::HashMap<String, (usize, &str)> = tags
            .iter()
            .enumerate()
            .map(|(i, t)| (tag_key(&t.name), (i, t.name.as_str())))
            .collect();
        let mut stmt = self.conn.prepare(
            "SELECT h.reminder_guid, h.name FROM reminder_hashtags h
             JOIN reminders r ON r.guid = h.reminder_guid
             JOIN reminder_lists l ON l.guid = r.list_guid
             WHERE l.enabled = 1 AND r.completed_at IS NULL",
        )?;
        let mut placed: std::collections::HashMap<String, Vec<(usize, &str)>> =
            std::collections::HashMap::new();
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (reminder, name) = row?;
            if let Some(&at) = order.get(&tag_key(&name)) {
                placed.entry(reminder).or_default().push(at);
            }
        }
        let tags_by_reminder = placed
            .into_iter()
            .map(|(reminder, mut names)| {
                names.sort_unstable();
                names.dedup();
                (
                    reminder,
                    names.into_iter().map(|(_, n)| n.to_string()).collect(),
                )
            })
            .collect();

        // Every custom list's rules over one read of what is still to do, rather than one
        // read of every open reminder per list.
        let custom: Vec<(String, Option<String>)> = self
            .reminder_smart_lists()?
            .into_iter()
            .filter(|s| s.kind == "custom")
            .map(|s| (s.guid, s.filter))
            .collect();
        let mut custom_counts = std::collections::HashMap::new();
        if !custom.is_empty() {
            let tagged = self.all_tags_by_reminder()?;
            let open = self.open_reminders()?;
            let none: Vec<String> = Vec::new();
            for (guid, filter) in custom {
                let count = filter.map_or(0, |filter| {
                    let filter = SmartFilter::decode(&filter);
                    open.iter()
                        .filter(|r| filter.holds(r, tagged.get(&r.guid).unwrap_or(&none), now))
                        .take(MOST_AT_ONCE)
                        .count()
                });
                custom_counts.insert(guid, count);
            }
        }
        Ok(ReminderExtras {
            tags_by_reminder,
            custom_counts,
        })
    }

    /// The account id Apple writes inside early alerts, from any reminder that has one: a new
    /// alert is written with the same.
    pub fn early_alert_account(&self) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row(
                "SELECT account_id FROM reminder_early_alerts WHERE account_id IS NOT NULL LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// A section made or renamed here, shown before the next pass brings Apple's copy.
    pub fn put_reminder_section_local(&mut self, section: &ReminderSection) -> Result<()> {
        self.put_reminder_sections(std::slice::from_ref(section), &[])
    }

    /// One reminder moved into a section of its list, or out of every one, here at once.
    /// A section not in this library, or in another list, places nothing.
    pub fn set_reminder_section(&mut self, reminder: &str, section: Option<&str>) -> Result<()> {
        match section {
            None => {
                self.conn.execute(
                    "UPDATE reminders SET section_guid = NULL WHERE guid = ?1",
                    params![reminder],
                )?;
            }
            Some(section) => {
                self.conn.execute(
                    "UPDATE reminders SET section_guid = ?2 WHERE guid = ?1
                       AND EXISTS (SELECT 1 FROM reminder_sections s
                                   WHERE s.guid = ?2 AND s.list_guid = reminders.list_guid)",
                    params![reminder, section],
                )?;
            }
        }
        Ok(())
    }
}

/// [`REMINDER_COLUMNS`] qualified with `r.`, for queries that join the list.
fn prefixed_columns() -> String {
    "r.guid, r.list_guid, r.title, r.notes, r.due_at, r.completed_at, r.priority, r.flagged,
     r.parent_guid, r.recur_frequency, r.recur_interval, r.etag, r.timezone, r.section_guid,
     r.all_day,
     CASE WHEN json_valid(r.raw) THEN json_extract(r.raw, '$.fields.CreationDate.value') END"
        .to_string()
}

#[cfg(test)]
mod detail_tests {
    use super::*;

    fn reminder(guid: &str, list: &str) -> Reminder {
        Reminder {
            guid: guid.into(),
            list_guid: list.into(),
            title: guid.into(),
            ..Default::default()
        }
    }

    fn library() -> Library {
        let mut lib = Library::in_memory().unwrap();
        let list = |g: &str| ReminderList {
            guid: g.into(),
            title: g.into(),
            enabled: true,
            ..Default::default()
        };
        lib.put_reminder_lists(&[list("List/A"), list("List/B")])
            .unwrap();
        let mut high = reminder("Reminder/1", "List/A");
        high.priority = 1;
        let mut flagged = reminder("Reminder/2", "List/B");
        flagged.flagged = true;
        flagged.priority = 5;
        let mut done = reminder("Reminder/3", "List/A");
        done.completed_at = Some(1);
        lib.put_reminders(&[high, flagged, done, reminder("Reminder/4", "List/B")])
            .unwrap();
        let tag = |g: &str, r: &str, n: &str| ReminderHashtag {
            guid: g.into(),
            reminder_guid: r.into(),
            name: n.into(),
        };
        lib.put_reminder_details(&ReminderDetailsPass {
            whole: true,
            hashtags: vec![
                tag("Hashtag/a", "Reminder/1", "work"),
                tag("Hashtag/b", "Reminder/2", "Work"),
                tag("Hashtag/c", "Reminder/2", "home"),
                tag("Hashtag/d", "Reminder/3", "work"),
            ],
            attachments: vec![
                ReminderAttachment {
                    guid: "Attachment/u".into(),
                    reminder_guid: "Reminder/1".into(),
                    kind: "URL".into(),
                    url: Some("https://example.com/x".into()),
                    uti: Some("public.url".into()),
                },
                ReminderAttachment {
                    guid: "Attachment/f".into(),
                    reminder_guid: "Reminder/1".into(),
                    kind: "Image".into(),
                    url: None,
                    uti: Some("public.jpeg".into()),
                },
            ],
            early_alerts: vec![(
                "Reminder/1".into(),
                Some("ACCT".into()),
                vec![EarlyAlert {
                    id: "E1".into(),
                    count: 30,
                    unit: 0,
                }],
            )],
            smart_lists: vec![ReminderSmartList {
                guid: "SmartList/S".into(),
                kind: "custom".into(),
                title: Some("Medium".into()),
                filter: Some(r#"{"priorities":["medium"]}"#.into()),
                ..Default::default()
            }],
            gone: vec![],
        })
        .unwrap();
        lib
    }

    #[test]
    fn a_reminder_detail_has_its_tags_link_and_early_alert() {
        let lib = library();
        let d = lib.reminder_detail("Reminder/1").unwrap();
        assert_eq!(d.tags, vec!["work".to_string()]);
        assert_eq!(d.url.as_deref(), Some("https://example.com/x"));
        assert_eq!(d.other_attachments, 1);
        assert_eq!(
            d.early_alerts,
            vec![EarlyAlert {
                id: "E1".into(),
                count: 30,
                unit: 0
            }]
        );
        assert_eq!(lib.early_alert_account().unwrap().as_deref(), Some("ACCT"));
        assert_eq!(
            lib.reminder_detail("Reminder/4").unwrap(),
            ReminderDetail {
                guid: "Reminder/4".into(),
                ..Default::default()
            }
        );
    }

    /// One entry per tag however it is capitalised, counting only reminders still to do.
    #[test]
    fn tags_are_listed_once_with_open_counts() {
        let lib = library();
        let tags = lib.reminder_tags().unwrap();
        let names: Vec<(&str, usize)> = tags.iter().map(|t| (t.name.as_str(), t.open)).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&("home", 1)));
        assert!(names
            .iter()
            .any(|(n, c)| n.eq_ignore_ascii_case("work") && *c == 2));
        let tagged: Vec<String> = lib
            .reminders_tagged("#WORK")
            .unwrap()
            .into_iter()
            .map(|r| r.guid)
            .collect();
        assert_eq!(tagged.len(), 2);
        assert!(
            !tagged.contains(&"Reminder/3".to_string()),
            "done ones are not shown"
        );
    }

    /// The sidebar's sweep in one read: the same tags per reminder and the same smart-list
    /// counts as asking for each tag's and each list's reminders one at a time.
    #[test]
    fn extras_match_the_tag_and_smart_list_reads() {
        let lib = library();
        let extras = lib.reminder_extras(0).unwrap();
        let tags = lib.reminder_tags().unwrap();
        let mut one_by_one: std::collections::HashMap<String, Vec<String>> = Default::default();
        for t in &tags {
            for r in lib.reminders_tagged(&t.name).unwrap() {
                one_by_one.entry(r.guid).or_default().push(t.name.clone());
            }
        }
        assert_eq!(extras.tags_by_reminder, one_by_one);
        assert!(
            !extras.tags_by_reminder.contains_key("Reminder/3"),
            "done ones carry no chips"
        );
        assert_eq!(
            extras.custom_counts.get("SmartList/S").copied(),
            Some(
                lib.reminders_in_smart_list("SmartList/S", 0)
                    .unwrap()
                    .reminders
                    .len()
            )
        );
    }

    /// An incremental pass lays itself over what is stored; a gone record goes; a reminder
    /// that comes back without its alert loses it.
    #[test]
    fn an_incremental_pass_merges_and_forgets() {
        let mut lib = library();
        lib.put_reminder_details(&ReminderDetailsPass {
            whole: false,
            early_alerts: vec![("Reminder/1".into(), None, vec![])],
            gone: vec!["Hashtag/a".into(), "Attachment/u".into()],
            ..Default::default()
        })
        .unwrap();
        let d = lib.reminder_detail("Reminder/1").unwrap();
        assert!(d.tags.is_empty());
        assert_eq!(d.url, None);
        assert!(d.early_alerts.is_empty());
        assert_eq!(lib.reminder_detail("Reminder/2").unwrap().tags.len(), 2);
        assert_eq!(lib.reminder_smart_lists().unwrap().len(), 1, "kept");
    }

    /// A reminder deleted takes its details with it.
    #[test]
    fn details_of_a_deleted_reminder_go() {
        let mut lib = library();
        lib.forget_reminders(&["Reminder/2".into()], &[]).unwrap();
        lib.put_reminder_details(&ReminderDetailsPass::default())
            .unwrap();
        assert!(lib
            .reminder_tags()
            .unwrap()
            .iter()
            .all(|t| t.name != "home"));
    }

    /// The one filter measured on a real account.
    #[test]
    fn the_measured_priority_filter_decodes_and_matches() {
        let lib = library();
        let got = lib.reminders_in_smart_list("SmartList/S", 0).unwrap();
        assert!(got.unknown.is_empty());
        assert_eq!(
            got.reminders
                .iter()
                .map(|r| r.guid.as_str())
                .collect::<Vec<_>>(),
            vec!["Reminder/2"]
        );
    }

    #[test]
    fn common_filters_decode() {
        let f = SmartFilter::decode(
            r##"{"hashtags":{"hashtags":["#Work","home"],"operation":"or"},
                "flagged":true,"priorities":["high","low"],
                "lists":["0AB1-X"],"operation":"any",
                "date":{"type":"today"},"minimumSupportedVersion":20230430}"##,
        );
        assert!(!f.match_all);
        let tags = f.tags.clone().unwrap();
        assert_eq!(tags.names, vec!["home".to_string(), "work".to_string()]);
        assert!(!tags.all);
        assert_eq!(f.flagged, Some(true));
        assert_eq!(f.priorities, vec![1, 9]);
        assert_eq!(f.lists, vec!["List/0AB1-X".to_string()]);
        assert_eq!(f.date, Some(DateFilter::Today));
        assert!(f.unknown.is_empty(), "{:?}", f.unknown);

        let range = SmartFilter::decode(r#"{"date":{"start":800000000,"end":800086400}}"#);
        assert_eq!(
            range.date,
            Some(DateFilter::Range {
                from: 800_000_000 + APPLE_EPOCH,
                to: 800_086_400 + APPLE_EPOCH
            })
        );
        let relative =
            SmartFilter::decode(r#"{"date":{"type":"relative","count":3,"unit":"day"}}"#);
        assert!(matches!(
            relative.date,
            Some(DateFilter::Relative {
                count: 3,
                past: false,
                ..
            })
        ));
    }

    /// What is not understood is named, not guessed.
    #[test]
    fn unknown_filter_parts_are_reported() {
        let f = SmartFilter::decode(r#"{"location":{"x":1},"priorities":["urgent"]}"#);
        assert!(f.unknown.iter().any(|u| u == "location"));
        assert!(f.unknown.iter().any(|u| u.contains("urgent")));
        assert!(!SmartFilter::decode("not json").unknown.is_empty());
    }

    #[test]
    fn all_and_any_combine_as_apple_does() {
        let lib = library();
        let ids = |f: &str| -> Vec<String> {
            let mut v: Vec<String> = lib
                .reminders_matching(&SmartFilter::decode(f), 0)
                .unwrap()
                .into_iter()
                .map(|r| r.guid)
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            ids(r#"{"hashtags":["work"],"flagged":true}"#),
            vec!["Reminder/2"]
        );
        assert_eq!(
            ids(r#"{"hashtags":["work"],"flagged":true,"operation":"any"}"#),
            vec!["Reminder/1", "Reminder/2"]
        );
        assert_eq!(
            ids(r#"{"lists":["List/B"]}"#),
            vec!["Reminder/2", "Reminder/4"]
        );
        assert_eq!(
            ids(r#"{"hashtags":{"hashtags":["work","home"],"operation":"and"}}"#),
            vec!["Reminder/2"]
        );
        assert!(ids("{}").is_empty(), "no criteria holds nothing");
    }

    #[test]
    fn date_filters_hold_against_the_clock() {
        let now = 1_800_000_000;
        let mut due = reminder("R", "L");
        due.due_at = Some(now - 60);
        assert!(date_holds(&DateFilter::Overdue, &due, now));
        assert!(date_holds(&DateFilter::Today, &due, now));
        assert!(date_holds(&DateFilter::Any, &due, now));
        let week = DateFilter::Relative {
            count: 7,
            unit: "day".into(),
            past: false,
        };
        due.due_at = Some(now + 3 * 86_400);
        assert!(date_holds(&week, &due, now));
        due.due_at = Some(now + 30 * 86_400);
        assert!(!date_holds(&week, &due, now));
        due.due_at = None;
        assert!(!date_holds(&DateFilter::Any, &due, now));
    }

    #[test]
    fn a_reminder_moves_into_a_section_of_its_own_list_only() {
        let mut lib = library();
        lib.put_reminder_section_local(&ReminderSection {
            guid: "ListSection/S".into(),
            list_guid: "List/A".into(),
            title: "Soon".into(),
            created_at: None,
        })
        .unwrap();
        lib.set_reminder_section("Reminder/1", Some("ListSection/S"))
            .unwrap();
        lib.set_reminder_section("Reminder/2", Some("ListSection/S"))
            .unwrap();
        assert_eq!(
            lib.reminder("Reminder/1")
                .unwrap()
                .unwrap()
                .section_guid
                .as_deref(),
            Some("ListSection/S")
        );
        assert_eq!(
            lib.reminder("Reminder/2").unwrap().unwrap().section_guid,
            None
        );
        lib.set_reminder_section("Reminder/1", None).unwrap();
        assert_eq!(
            lib.reminder("Reminder/1").unwrap().unwrap().section_guid,
            None
        );
    }
}

// ---------------------------------------------------------------------------------------
// The rest of a reminder's schedule: its repeat rule whole, where it rings, who it is about.
// ---------------------------------------------------------------------------------------

/// A record Apple keeps beside a reminder, stored whole as Apple sent it: a `RecurrenceRule`
/// (with the reminder it governs) or a place `AlarmTrigger` (reached through the reminder's
/// alarms). Kept verbatim so anything this app does not read yet is still there to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderPart {
    /// The record name, `RecurrenceRule/<id>` or `AlarmTrigger/<id>`.
    pub name: String,
    pub reminder_guid: Option<String>,
    /// Apple's record type.
    pub kind: String,
    pub raw: String,
}

/// A repeat as the phone's Repeat and End Repeat rows show it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepeatRule {
    /// Apple's `Frequency`: 0 daily, 1 weekly, 2 monthly, 3 yearly, 4 hourly.
    pub frequency: i64,
    pub interval: i64,
    /// End Repeat On Date, as an instant in seconds.
    pub end_at: Option<i64>,
    /// Ends after this many times; 0 is never.
    pub occurrence_count: i64,
    /// The weekdays it falls on, 1 Sunday … 7 Saturday: Weekdays is 2–6, Weekends 1 and 7.
    /// Empty when it falls on its due date's day.
    #[serde(default)]
    pub days_of_week: Vec<i64>,
    /// Fields on Apple's rule this app does not read (chosen weekdays, days of the month…),
    /// by name. A rule with any is shown as Apple's, and its days are kept as they are.
    pub other: Vec<String>,
}

/// A place a reminder rings at: the phone's Location row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PlaceAlarm {
    /// The place as the phone names it ("Alex's Home").
    pub title: Option<String>,
    /// `arriving` or `leaving`, from Apple's `Proximity` (1 and 2); `None` when it says neither.
    pub proximity: Option<String>,
    /// Apple's trigger `Type`: `Location`, or another kind of place it knows.
    pub kind: String,
}

/// The person a reminder comes up for when messaging them: the phone's When Messaging row.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MessagingPerson {
    /// Their name, from this account's contacts, when one of the handles is on a card.
    pub name: Option<String>,
    /// Their email addresses and phone numbers, as Apple keeps them.
    pub handles: Vec<String>,
}

/// What a reminder's row shows beside its date, as the phone's list does: Weekdays or
/// Weekends by the repeat, the Urgent alarm, the place it rings at, the person it is about.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderMarks {
    pub guid: String,
    /// The weekdays its repeat falls on; empty for its due date's day.
    pub days_of_week: Vec<i64>,
    pub urgent: bool,
    pub location: Option<PlaceAlarm>,
    /// The person's name, or a handle where no card has one.
    pub messaging: Option<String>,
    /// Its link, for the chip the phone shows.
    pub url: Option<String>,
}

/// What the phone's Details screen shows beyond tags, a link and early alerts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReminderMore {
    pub guid: String,
    pub repeat: Option<RepeatRule>,
    /// The named zone its time is kept in ("America/New_York").
    pub time_zone: Option<String>,
    /// Urgent: rings as an alarm, even in a Focus or with the sound off. `None` until known.
    pub urgent: Option<bool>,
    pub location: Option<PlaceAlarm>,
    pub messaging: Option<MessagingPerson>,
}

impl Library {
    /// Store the parts a pass brought back. A whole pass replaces what is stored; an
    /// incremental one lays itself over it, less the records named in `gone`.
    pub fn put_reminder_parts(
        &mut self,
        parts: &[ReminderPart],
        gone: &[String],
        whole: bool,
    ) -> Result<()> {
        let tx = self.conn.savepoint()?;
        if whole {
            tx.execute("DELETE FROM reminder_parts", [])?;
        }
        {
            let mut forget = tx.prepare("DELETE FROM reminder_parts WHERE name = ?1")?;
            for name in gone {
                forget.execute(params![name])?;
            }
            let mut put = tx.prepare(
                "INSERT INTO reminder_parts (name, reminder_guid, kind, raw) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(name) DO UPDATE SET reminder_guid = excluded.reminder_guid,
                     kind = excluded.kind, raw = excluded.raw",
            )?;
            for p in parts {
                put.execute(params![p.name, p.reminder_guid, p.kind, p.raw])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Everything kept beside one reminder: its repeat rules, and the place triggers its
    /// alarms name.
    pub fn reminder_parts(&self, guid: &str) -> Result<Vec<ReminderPart>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, reminder_guid, kind, raw FROM reminder_parts WHERE reminder_guid = ?1
             UNION ALL
             SELECT p.name, p.reminder_guid, p.kind, p.raw FROM reminder_parts p
               JOIN reminder_alarms a ON p.name = 'AlarmTrigger/' || a.trigger_id
              WHERE a.reminder_guid = ?1
             ORDER BY 1",
        )?;
        let rows = stmt.query_map(params![guid], |r| {
            Ok(ReminderPart {
                name: r.get(0)?,
                reminder_guid: r.get(1)?,
                kind: r.get(2)?,
                raw: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The weekdays a reminder's repeat falls on (Weekdays, Weekends, chosen days), from the
    /// rule it names now; empty when it falls on its due date's day.
    pub fn reminder_repeat_days(&self, guid: &str) -> Result<Vec<i64>> {
        use rusqlite::OptionalExtension;
        let raw: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT json_extract(p.raw, '$.fields.DaysOfTheWeek.value')
                   FROM reminder_parts p JOIN reminders r ON r.guid = p.reminder_guid
                  WHERE p.reminder_guid = ?1 AND p.kind = 'RecurrenceRule'
                    AND json_valid(p.raw) AND json_valid(r.raw)
                    AND EXISTS (SELECT 1 FROM json_each(r.raw, '$.fields.RecurrenceRuleIDs.value') j
                                 WHERE upper(p.name) = upper('RecurrenceRule/' || j.value))
                  LIMIT 1",
                params![guid],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw
            .flatten()
            .and_then(|b| days_of_week_from(&b))
            .unwrap_or_default())
    }

    /// When a reminder's repeat ends (End Repeat On Date), in seconds: from the rule the
    /// reminder names now, as Apple sent it. `None` when it repeats for ever or not at all.
    pub fn reminder_repeat_end(&self, guid: &str) -> Result<Option<i64>> {
        Ok(self
            .repeat_ends(Some(guid))?
            .into_iter()
            .next()
            .map(|(_, end)| end))
    }

    /// Every open reminder whose repeat ends on a date, with that date in seconds.
    pub fn reminder_repeat_ends(&self) -> Result<Vec<(String, i64)>> {
        self.repeat_ends(None)
    }

    fn repeat_ends(&self, guid: Option<&str>) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.reminder_guid, json_extract(p.raw, '$.fields.EndDate.value')
               FROM reminder_parts p JOIN reminders r ON r.guid = p.reminder_guid
              WHERE (?1 IS NULL OR p.reminder_guid = ?1) AND p.kind = 'RecurrenceRule'
                AND r.completed_at IS NULL AND r.recur_frequency IS NOT NULL
                AND json_valid(p.raw) AND json_valid(r.raw)
                AND json_extract(p.raw, '$.fields.EndDate.value') IS NOT NULL
                AND EXISTS (SELECT 1 FROM json_each(r.raw, '$.fields.RecurrenceRuleIDs.value') j
                             WHERE upper(p.name) = upper('RecurrenceRule/' || j.value))",
        )?;
        let rows = stmt.query_map(params![guid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?.div_euclid(1000),
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The name on the card holding one of these email addresses or phone numbers. Numbers
    /// are matched on their digits, the last ten where there are more, so "+91 98765 43210"
    /// on a card finds "+919876543210".
    pub fn contact_name_for(&self, handles: &[String]) -> Result<Option<String>> {
        let digits = |s: &str| -> String { s.chars().filter(char::is_ascii_digit).collect() };
        let tail = |d: &str| d[d.len().saturating_sub(10)..].to_string();
        let mut stmt = self.conn.prepare(
            "SELECT m.kind, m.value, trim(coalesce(c.first_name, '') || ' ' || coalesce(c.last_name, '')),
                    c.company
               FROM contact_methods m JOIN contacts c ON c.guid = m.contact_guid
              ORDER BY c.guid, m.position",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for handle in handles {
            let email = handle.contains('@');
            let wanted = if email {
                handle.trim().to_lowercase()
            } else {
                tail(&digits(handle))
            };
            if wanted.is_empty() {
                continue;
            }
            let found = rows.iter().find(|(_, value, _, _)| {
                if email {
                    value.trim().to_lowercase() == wanted
                } else {
                    let d = digits(value);
                    d.len() >= 6 && tail(&d) == wanted
                }
            });
            if let Some((_, _, name, company)) = found {
                let name = if name.is_empty() {
                    company.clone().unwrap_or_default()
                } else {
                    name.clone()
                };
                if !name.trim().is_empty() {
                    return Ok(Some(name));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod parts_tests {
    use super::*;

    fn part(name: &str, reminder: Option<&str>, kind: &str) -> ReminderPart {
        ReminderPart {
            name: name.into(),
            reminder_guid: reminder.map(Into::into),
            kind: kind.into(),
            raw: format!("{{\"recordName\":\"{name}\"}}"),
        }
    }

    #[test]
    fn a_reminder_finds_its_rule_and_the_place_its_alarm_names() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_parts(
            &[
                part("RecurrenceRule/R", Some("Reminder/1"), "RecurrenceRule"),
                part("AlarmTrigger/T", None, "AlarmTrigger"),
                part("RecurrenceRule/X", Some("Reminder/2"), "RecurrenceRule"),
            ],
            &[],
            true,
        )
        .unwrap();
        lib.put_alarms(
            &[crate::alarms::Alarm {
                id: "A".into(),
                reminder_guid: "Reminder/1".into(),
                trigger_id: "T".into(),
                acknowledged_at: None,
            }],
            &[],
            &[],
            true,
        )
        .unwrap();
        let names: Vec<String> = lib
            .reminder_parts("Reminder/1")
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["AlarmTrigger/T", "RecurrenceRule/R"]);
    }

    fn repeating(due: i64, frequency: i64) -> Reminder {
        Reminder {
            guid: "Reminder/1".into(),
            list_guid: "List/a".into(),
            title: "Take vitamins".into(),
            due_at: Some(due),
            recur_frequency: Some(frequency),
            recur_interval: Some(1),
            timezone: Some("UTC".into()),
            ..Default::default()
        }
    }

    const DUE: i64 = 1_792_681_200; // 22 Oct 2026, 15:00 UTC

    #[test]
    fn the_last_tick_before_the_end_completes_the_series() {
        let r = repeating(DUE, 0);
        // Ends the evening of the 22nd: tomorrow's is past it.
        assert!(r.tick_ends_series(Some(DUE + 8 * 3600), DUE - 60));
        // Ends on the 23rd at 23:59: tomorrow's 15:00 is still in it.
        assert!(!r.tick_ends_series(Some(DUE + 86_400 + 9 * 3600), DUE - 60));
        // No end: never.
        assert!(!r.tick_ends_series(None, DUE - 60));
        // A one-off has no series to end.
        let mut once = r.clone();
        once.recur_frequency = None;
        assert!(!once.tick_ends_series(Some(DUE), DUE - 60));
    }

    #[test]
    fn weekdays_step_over_the_weekend_and_weekends_over_the_week() {
        // Thu 22 Oct 2026 15:00 UTC.
        let mut r = repeating(DUE, 1);
        let day = 86_400;
        let weekdays = [2, 3, 4, 5, 6];
        assert_eq!(
            r.next_occurrence_on(&weekdays, DUE - 60),
            Some(DUE + day),
            "Thursday to Friday"
        );
        r.due_at = Some(DUE + day);
        assert_eq!(
            r.next_occurrence_on(&weekdays, DUE),
            Some(DUE + 4 * day),
            "Friday to Monday"
        );
        r.due_at = Some(DUE);
        assert_eq!(
            r.next_occurrence_on(&[1, 7], DUE - 60),
            Some(DUE + 2 * day),
            "Thursday to Saturday"
        );
        // Mon/Wed/Fri every other week: Friday this week, then Monday a fortnight on.
        r.recur_interval = Some(2);
        assert_eq!(r.next_occurrence_on(&[2, 4, 6], DUE - 60), Some(DUE + day));
        r.due_at = Some(DUE + day);
        assert_eq!(r.next_occurrence_on(&[2, 4, 6], DUE), Some(DUE + 11 * day));
        // No days: a week on, as before.
        r.recur_interval = Some(1);
        r.due_at = Some(DUE);
        assert_eq!(r.next_occurrence_on(&[], DUE - 60), Some(DUE + 7 * day));
        // Weekdays ending Friday night: ticked Friday, it is done.
        r.due_at = Some(DUE + day);
        assert!(r.tick_ends_series_on(Some(DUE + day + 3600), &weekdays, DUE));
    }

    #[test]
    fn hourly_steps_by_the_hour_and_ends_on_time() {
        let r = repeating(DUE, 4);
        assert_eq!(r.next_occurrence_after(DUE - 60), Some(DUE + 3600));
        assert!(r.tick_ends_series(Some(DUE + 1800), DUE - 60));
        assert!(!r.tick_ends_series(Some(DUE + 3600), DUE - 60));
    }

    #[test]
    fn the_end_is_read_from_the_rule_the_reminder_names() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "List/a".into(),
            title: "A".into(),
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        let mut r = repeating(DUE, 0);
        r.raw = Some(
            r#"{"fields":{"RecurrenceRuleIDs":{"type":"STRING_LIST","value":["abc-1"]}}}"#.into(),
        );
        lib.put_reminders(&[r]).unwrap();
        let rule = |name: &str, end: i64| ReminderPart {
            name: name.into(),
            reminder_guid: Some("Reminder/1".into()),
            kind: "RecurrenceRule".into(),
            raw: format!(r#"{{"fields":{{"EndDate":{{"type":"TIMESTAMP","value":{end}}}}}}}"#),
        };
        // An old rule the reminder no longer names is not its end.
        lib.put_reminder_parts(
            &[
                rule("RecurrenceRule/old", 1_000_000),
                rule("RecurrenceRule/ABC-1", 1_792_900_000_000),
            ],
            &[],
            true,
        )
        .unwrap();
        assert_eq!(
            lib.reminder_repeat_end("Reminder/1").unwrap(),
            Some(1_792_900_000)
        );
        lib.put_reminder_parts(
            &[part(
                "RecurrenceRule/ABC-1",
                Some("Reminder/1"),
                "RecurrenceRule",
            )],
            &[],
            true,
        )
        .unwrap();
        assert_eq!(
            lib.reminder_repeat_end("Reminder/1").unwrap(),
            None,
            "repeats for ever"
        );
    }

    #[test]
    fn an_incremental_pass_lays_over_and_forgets_what_is_gone() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_reminder_parts(
            &[part(
                "RecurrenceRule/R",
                Some("Reminder/1"),
                "RecurrenceRule",
            )],
            &[],
            true,
        )
        .unwrap();
        lib.put_reminder_parts(
            &[part(
                "RecurrenceRule/S",
                Some("Reminder/1"),
                "RecurrenceRule",
            )],
            &["RecurrenceRule/R".into()],
            false,
        )
        .unwrap();
        let names: Vec<String> = lib
            .reminder_parts("Reminder/1")
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["RecurrenceRule/S"]);
    }
}
