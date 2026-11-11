//! Birthdays and public holidays, drawn on the calendar the way a phone draws them.
//!
//! Neither lives in an iCloud calendar — measured: the account holds four calendars, all
//! personal, and neither of these. A phone makes both itself: birthdays from the contacts
//! that have one, holidays from Apple's own published feed for the region. So does this,
//! from the same two sources, and both are read-only.

use crate::{Error, Result};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use quince_core::calendar::{Calendar, Event};
use quince_core::contacts::Contact;

/// The calendar birthdays are drawn in. Not Apple's — there is no such record — so named
/// where no Apple guid can collide with it.
pub const BIRTHDAYS: &str = "quince:birthdays";
/// The calendar holidays are drawn in.
pub const HOLIDAYS: &str = "quince:holidays";

/// The year Apple writes for a birthday given without one (`1604-03-17`).
const NO_YEAR: i16 = 1604;

/// A birthday as Android's Contacts keeps it, from Apple's: one without a year is `--MM-DD`
/// there (vCard's form, which Android shows as a day and month), never Apple's 1604, which
/// Android showed as a year and so an age of four hundred. A full date is left as it is.
#[must_use]
pub fn birthday_for_phone(apple: &str) -> String {
    match apple.trim().strip_prefix("1604-") {
        Some(day) if day.len() == 5 && day.as_bytes()[2] == b'-' => format!("--{day}"),
        _ => apple.to_string(),
    }
}

/// A birthday as Apple keeps it, from Android's: a day and month without a year (`--MM-DD`,
/// or `--MMDD`) is Apple's 1604, so it shows on an iPhone as a birthday with no year. A full
/// date is left as it is.
#[must_use]
pub fn birthday_from_phone(phone: &str) -> String {
    let Some(day) = phone.trim().strip_prefix("--").filter(|d| d.is_ascii()) else {
        return phone.to_string();
    };
    let (month, date) = match (day.len(), day.as_bytes().get(2)) {
        (5, Some(b'-')) => (&day[..2], &day[3..]),
        (4, _) => (&day[..2], &day[2..]),
        _ => return phone.to_string(),
    };
    if !(month.bytes().all(|b| b.is_ascii_digit()) && date.bytes().all(|b| b.is_ascii_digit())) {
        return phone.to_string();
    }
    format!("{NO_YEAR}-{month}-{date}")
}

/// The two calendars, as the sidebar lists them.
#[must_use]
pub fn calendars(holidays_title: Option<&str>, any_birthdays: bool) -> Vec<Calendar> {
    let read_only = |guid: &str, title: &str, colour: &str, order: i64| Calendar {
        guid: guid.into(),
        title: title.into(),
        colour: Some(colour.into()),
        ctag: None,
        sort_order: order,
        enabled: true,
        writable: false,
        is_default: false,
    };
    let mut out = Vec::new();
    if any_birthdays {
        out.push(read_only(BIRTHDAYS, "Birthdays", "#8e8e93", 900));
    }
    if let Some(title) = holidays_title {
        out.push(read_only(HOLIDAYS, title, "#34c759", 901));
    }
    out
}

/// An all-day event for `day`, from this machine's midnight to the next — the zone the
/// interface draws a day in.
fn all_day(guid: String, calendar: &str, title: String, day: Date) -> Option<Event> {
    let zone = TimeZone::system();
    let start = day.to_zoned(zone.clone()).ok()?.timestamp().as_second();
    let end = day
        .tomorrow()
        .ok()?
        .to_zoned(zone)
        .ok()?
        .timestamp()
        .as_second();
    Some(Event {
        guid,
        calendar_guid: calendar.into(),
        title,
        location: None,
        starts_at: start,
        ends_at: end,
        all_day: true,
        timezone: None,
        url: None,
        notes: None,
        etag: None,
    })
}

/// The years `[from, to)` touches, in this machine's zone.
fn years(from: i64, to: i64) -> std::ops::RangeInclusive<i16> {
    let year = |t: i64| {
        jiff::Timestamp::from_second(t)
            .map(|t| t.to_zoned(TimeZone::system()).year())
            .unwrap_or(2000)
    };
    year(from)..=year(to.max(from))
}

fn within(event: &Event, from: i64, to: i64) -> bool {
    event.starts_at < to && event.ends_at > from
}

/// How a contact is named on the day, as the phone writes it: "Zoya Ahuja’s Birthday".
fn birthday_title(c: &Contact) -> Option<String> {
    let name = [c.first_name.as_deref(), c.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let name = if name.is_empty() {
        c.company.as_deref()?.trim().to_string()
    } else {
        name
    };
    (!name.is_empty()).then(|| format!("{name}’s Birthday"))
}

/// Every contact's birthday falling in `[from, to)`, as all-day events.
///
/// Apple stores a birthday as `YYYY-MM-DD`, with the year 1604 standing for "no year given".
/// A birthday on 29 February falls on the 28th in other years, as a phone shows it.
#[must_use]
pub fn birthdays(contacts: &[Contact], from: i64, to: i64) -> Vec<Event> {
    let mut out = Vec::new();
    for c in contacts {
        let (Some(raw), Some(title)) = (c.birthday.as_deref(), birthday_title(c)) else {
            continue;
        };
        let Ok(born) = raw.trim().get(..10).unwrap_or(raw).parse::<Date>() else {
            continue;
        };
        for year in years(from, to) {
            if born.year() != NO_YEAR && year < born.year() {
                continue;
            }
            let day = Date::new(year, born.month(), born.day())
                .or_else(|_| Date::new(year, born.month(), born.day() - 1));
            let Ok(day) = day else { continue };
            if let Some(e) = all_day(
                format!("{BIRTHDAYS}:{}:{year}", c.guid),
                BIRTHDAYS,
                title.clone(),
                day,
            ) {
                if within(&e, from, to) {
                    out.push(e);
                }
            }
        }
    }
    out
}

/// One holiday in Apple's feed: its first date, and how many years it repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holiday {
    pub uid: String,
    pub title: String,
    pub first: Date,
    /// `RRULE:FREQ=YEARLY;COUNT=n` — the number of years, the first included. 1 for a date
    /// given once (most of them: a movable feast is listed year by year).
    pub years: u32,
    /// `BYMONTH=m;BYDAY=nDD`: not the same date each year but the nth weekday of a month —
    /// Columbus Day the second Monday of October; negative counts from the month's end.
    pub weekday: Option<(i8, i8, jiff::civil::Weekday)>,
}

/// The day a holiday falls on in `year`.
fn occurrence(h: &Holiday, year: i16) -> Option<Date> {
    let Some((month, nth, weekday)) = h.weekday else {
        return Date::new(year, h.first.month(), h.first.day()).ok();
    };
    let first = Date::new(year, month, 1).ok()?;
    let last = first.last_of_month();
    let weeks = |n: i8| jiff::Span::new().days(7 * i64::from(n.unsigned_abs() - 1));
    let day = if nth > 0 {
        // The first such weekday on or after the 1st, then whole weeks on.
        let ahead = (weekday.to_monday_zero_offset() - first.weekday().to_monday_zero_offset())
            .rem_euclid(7);
        first
            .checked_add(jiff::Span::new().days(i64::from(ahead)))
            .ok()?
            .checked_add(weeks(nth))
            .ok()?
    } else {
        // The last such weekday on or before the month's end, then whole weeks back.
        let behind = (last.weekday().to_monday_zero_offset() - weekday.to_monday_zero_offset())
            .rem_euclid(7);
        last.checked_sub(jiff::Span::new().days(i64::from(behind)))
            .ok()?
            .checked_sub(weeks(nth))
            .ok()?
    };
    (day.month() == month).then_some(day)
}

fn weekday_of(code: &str) -> Option<jiff::civil::Weekday> {
    use jiff::civil::Weekday::*;
    Some(match code {
        "MO" => Monday,
        "TU" => Tuesday,
        "WE" => Wednesday,
        "TH" => Thursday,
        "FR" => Friday,
        "SA" => Saturday,
        "SU" => Sunday,
        _ => return None,
    })
}

/// The feed's name for this machine's region and language, from its locale (`en_IN.UTF-8`
/// is India, in English). Apple publishes one per region at
/// `calendars.icloud.com/holidays/<region>_<language>.ics`.
#[must_use]
pub fn feed_for(locale: &str) -> Option<(String, String)> {
    let tag = locale.split(['.', '@']).next()?.trim();
    let (lang, region) = tag.split_once(['_', '-'])?;
    let ok = |s: &str| s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic());
    (ok(lang) && ok(region)).then(|| (region.to_ascii_lowercase(), lang.to_ascii_lowercase()))
}

/// The region a time zone belongs to, from the system's own table (`zone.tab`: `IN` for
/// Asia/Kolkata).
#[must_use]
pub fn region_of_zone(table: &str, zone: &str) -> Option<String> {
    table.lines().filter(|l| !l.starts_with('#')).find_map(|l| {
        let mut cols = l.split('\t');
        let code = cols.next()?;
        let name = cols.nth(1)?;
        (name == zone).then(|| code.to_ascii_lowercase())
    })
}

/// Which holiday feed this machine wants: the region its clock is set to — where its owner
/// is, as a phone's region setting is — in the language of its locale.
///
/// The locale alone is not the region: a machine in India set up in US English said `en_US`
/// and was offered American holidays. The zone is the better witness; the locale's region is
/// the fallback, and English the fallback language.
#[must_use]
pub fn this_machines_feed() -> Option<(String, String)> {
    let locale = std::env::var("LC_TIME")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    let from_locale = feed_for(&locale);
    let lang = from_locale
        .as_ref()
        .map_or_else(|| "en".to_string(), |(_, l)| l.clone());
    let zone = TimeZone::system().iana_name().map(ToString::to_string);
    let region = zone
        .and_then(|z| {
            let table = std::fs::read_to_string("/usr/share/zoneinfo/zone.tab").ok()?;
            region_of_zone(&table, &z)
        })
        .or_else(|| from_locale.map(|(r, _)| r))?;
    Some((region, lang))
}

/// Read Apple's holiday feed (iCalendar), and its name ("India Holidays").
///
/// Only what the feed uses: all-day `DTSTART;VALUE=DATE`, a `SUMMARY` (with parameters),
/// and an optional yearly `RRULE` with a `COUNT`. Folded lines are unfolded first.
#[must_use]
pub fn parse_feed(ics: &str) -> (Option<String>, Vec<Holiday>) {
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.lines() {
        if let Some(rest) = raw.strip_prefix([' ', '\t']) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(raw.trim_end_matches('\r').to_string());
    }
    let value = |line: &str| line.split_once(':').map(|(_, v)| v.trim().to_string());
    let mut name = None;
    let mut out = Vec::new();
    type Partial = (
        Option<String>,
        Option<String>,
        Option<Date>,
        u32,
        Option<(i8, i8, jiff::civil::Weekday)>,
    );
    let mut current: Option<Partial> = None;
    for line in &lines {
        let key = line.split([':', ';']).next().unwrap_or("");
        match key {
            "X-WR-CALNAME" if current.is_none() => name = value(line),
            "BEGIN" if line.ends_with(":VEVENT") => current = Some((None, None, None, 1, None)),
            "UID" => {
                if let Some(c) = current.as_mut() {
                    c.0 = value(line);
                }
            }
            "SUMMARY" => {
                if let Some(c) = current.as_mut() {
                    c.1 = value(line).map(|v| v.replace("\\,", ",").replace("\\;", ";"));
                }
            }
            "DTSTART" => {
                if let Some(c) = current.as_mut() {
                    c.2 = value(line).and_then(|v| {
                        // Digits only: sliced by byte, a feed's stray accented character there
                        // panicked rather than being skipped.
                        let d = v
                            .get(..8)
                            .filter(|d| d.bytes().all(|b| b.is_ascii_digit()))?;
                        format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..8])
                            .parse()
                            .ok()
                    });
                }
            }
            "RRULE" => {
                if let Some(c) = current.as_mut() {
                    let rule = value(line).unwrap_or_default();
                    if rule.contains("FREQ=YEARLY") {
                        let part = |k: &str| rule.split(';').find_map(|p| p.strip_prefix(k));
                        c.3 = part("COUNT=").and_then(|n| n.parse().ok()).unwrap_or(1);
                        c.4 = (|| {
                            let month: i8 = part("BYMONTH=")?.parse().ok()?;
                            let day = part("BYDAY=")?;
                            let (n, code) = day.split_at(day.len().checked_sub(2)?);
                            Some((month, n.parse().ok()?, weekday_of(code)?))
                        })();
                    }
                }
            }
            "END" if line.ends_with(":VEVENT") => {
                if let Some((Some(uid), Some(title), Some(first), years, weekday)) = current.take()
                {
                    out.push(Holiday {
                        uid,
                        title,
                        first,
                        years: years.max(1),
                        weekday,
                    });
                }
            }
            _ => {}
        }
    }
    (name, out)
}

/// The holidays falling in `[from, to)`, as all-day events.
#[must_use]
pub fn holidays(feed: &[Holiday], from: i64, to: i64) -> Vec<Event> {
    let mut out = Vec::new();
    let span = years(from, to);
    for h in feed {
        for n in 0..h.years {
            let Ok(n) = i16::try_from(n) else { break };
            let year = h.first.year() + n;
            if !span.contains(&year) {
                continue;
            }
            let Some(day) = occurrence(h, year) else {
                continue;
            };
            if let Some(e) = all_day(
                format!("{HOLIDAYS}:{}:{year}", h.uid),
                HOLIDAYS,
                h.title.clone(),
                day,
            ) {
                if within(&e, from, to) {
                    out.push(e);
                }
            }
        }
    }
    out
}

/// Apple's holiday feed for a region, as text. The server sends it gzipped whether asked to
/// or not (measured), so a gzip body is opened here.
///
/// # Errors
/// When the feed cannot be reached, or there is no feed for this region.
pub async fn fetch_feed(http: &reqwest::Client, region: &str, lang: &str) -> Result<String> {
    for name in [format!("{region}_{lang}"), format!("{region}_en")] {
        let url = format!("https://calendars.icloud.com/holidays/{name}.ics");
        let response = http.get(&url).send().await?;
        if !response.status().is_success() {
            continue;
        }
        let bytes = response.bytes().await?;
        let text = if bytes.starts_with(&[0x1f, 0x8b]) {
            let out = crate::bounded::gunzip(&bytes, crate::bounded::MOST_INFLATED)
                .ok_or_else(|| Error::Protocol("the holiday calendar would not open".into()))?;
            String::from_utf8(out)
                .map_err(|e| Error::Protocol(format!("the holiday calendar would not open: {e}")))?
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        if text.contains("BEGIN:VCALENDAR") {
            return Ok(text);
        }
    }
    Err(Error::NotFound)
}

#[cfg(test)]
mod tests {

    /// A birthday without a year goes to Android as `--MM-DD` and comes back as Apple's 1604,
    /// both ways; a full date, and anything else, is left as it is.
    #[test]
    fn a_yearless_birthday_is_the_phones_form_there_and_apples_here() {
        assert_eq!(birthday_for_phone("1604-03-17"), "--03-17");
        assert_eq!(birthday_from_phone("--03-17"), "1604-03-17");
        assert_eq!(birthday_from_phone("--0317"), "1604-03-17");
        assert_eq!(
            birthday_from_phone(&birthday_for_phone("1604-12-01")),
            "1604-12-01"
        );
        for kept in [
            "1990-03-17",
            "2000-02-29",
            "",
            "--3-17",
            "--é3-17",
            "1604-3-17",
        ] {
            assert_eq!(birthday_for_phone(kept), kept);
            assert_eq!(birthday_from_phone(kept), kept);
        }
    }

    /// A start date with an accented character where the digits go is skipped, not a panic.
    #[test]
    fn a_damaged_start_date_is_skipped_not_a_panic() {
        let ics = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Odd\nDTSTART;VALUE=DATE:202\u{e9}0101\nEND:VEVENT\nEND:VCALENDAR\n";
        let (_, found) = parse_feed(ics);
        assert!(found.is_empty() || found.iter().all(|h| h.title != "Odd"));
    }

    use super::*;

    fn contact(guid: &str, first: &str, birthday: &str) -> Contact {
        Contact {
            guid: guid.into(),
            first_name: Some(first.into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Ahuja".into()),
            company: None,
            is_company: false,
            birthday: Some(birthday.into()),
            notes: None,
            etag: None,
            methods: Vec::new(),
        }
    }

    fn day_of(t: i64) -> (i16, i8, i8) {
        let z = jiff::Timestamp::from_second(t)
            .unwrap()
            .to_zoned(TimeZone::system());
        (z.year(), z.month(), z.day())
    }

    fn span(a: &str, b: &str) -> (i64, i64) {
        let at = |d: &str| {
            d.parse::<Date>()
                .unwrap()
                .to_zoned(TimeZone::system())
                .unwrap()
                .timestamp()
                .as_second()
        };
        (at(a), at(b))
    }

    /// A birthday with no year (Apple's 1604), and one with a year, each on its own day
    /// every year — named as a phone names it.
    #[test]
    fn birthdays_fall_on_their_day_every_year() {
        let people = [
            contact("A", "Zoya", "1604-03-17"),
            contact("B", "Amma", "2005-11-05"),
        ];
        let (from, to) = span("2026-01-01", "2027-01-01");
        let got = birthdays(&people, from, to);
        let mut seen: Vec<_> = got
            .iter()
            .map(|e| (e.title.clone(), day_of(e.starts_at), e.all_day))
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            [
                ("Amma Ahuja’s Birthday".to_string(), (2026, 11, 5), true),
                ("Zoya Ahuja’s Birthday".to_string(), (2026, 3, 17), true),
            ]
        );
        assert!(got.iter().all(|e| e.calendar_guid == BIRTHDAYS));
    }

    #[test]
    fn a_birthday_is_not_drawn_before_the_person_was_born_and_leap_days_land_on_the_28th() {
        let (from, to) = span("2003-01-01", "2004-01-01");
        assert!(birthdays(&[contact("B", "Amma", "2005-11-05")], from, to).is_empty());
        let (from, to) = span("2027-02-01", "2027-03-01");
        let leap = birthdays(&[contact("L", "Leap", "2000-02-29")], from, to);
        assert_eq!(day_of(leap[0].starts_at), (2027, 2, 28));
    }

    /// Lines as Apple's India feed sends them.
    const FEED: &str = "BEGIN:VCALENDAR\r\nX-WR-CALNAME:India Holidays\r\nBEGIN:VEVENT\r\nUID:new-year\r\nDTSTART;VALUE=DATE:20250101\r\nSUMMARY;LANGUAGE=en:New Year’s Day\r\nRRULE:FREQ=YEARLY;COUNT=5\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:holi\r\nDTSTART;VALUE=DATE:20260304\r\nSUMMARY;LANGUAGE=en:Holi\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:long\r\nDTSTART;VALUE=DATE:20261002\r\nSUMMARY;LANGUAGE=en:Mahatma Gandhi\r\n  Jayanti\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn apples_holiday_feed_is_read_with_its_repeats() {
        let (name, feed) = parse_feed(FEED);
        assert!(feed.iter().all(|h| h.weekday.is_none()));
        assert_eq!(name.as_deref(), Some("India Holidays"));
        assert_eq!(feed.len(), 3);
        assert_eq!(feed[0].years, 5);
        assert_eq!(
            feed[2].title, "Mahatma Gandhi Jayanti",
            "a folded line is unfolded"
        );

        let (from, to) = span("2026-01-01", "2027-01-01");
        let mut got: Vec<_> = holidays(&feed, from, to)
            .iter()
            .map(|e| (e.title.clone(), day_of(e.starts_at)))
            .collect();
        got.sort_by_key(|g| g.1);
        assert_eq!(
            got,
            [
                ("New Year’s Day".to_string(), (2026, 1, 1)),
                ("Holi".to_string(), (2026, 3, 4)),
                ("Mahatma Gandhi Jayanti".to_string(), (2026, 10, 2)),
            ]
        );
        // Past its count, a repeat stops.
        let (from, to) = span("2030-01-01", "2031-01-01");
        assert!(holidays(&feed, from, to).is_empty());
    }

    /// The rules Apple's US and UK feeds use: the nth weekday of a month, and counting back
    /// from its end.
    #[test]
    fn a_holiday_on_the_nth_weekday_moves_with_the_calendar() {
        let rule = |m: i8, n: i8, w| Holiday {
            uid: "x".into(),
            title: "x".into(),
            first: "2022-01-01".parse().unwrap(),
            years: 10,
            weekday: Some((m, n, w)),
        };
        use jiff::civil::Weekday::*;
        let on = |h: &Holiday, y| occurrence(h, y).map(|d| d.to_string());
        assert_eq!(
            on(&rule(10, 2, Monday), 2026).as_deref(),
            Some("2026-10-12"),
            "Columbus Day"
        );
        assert_eq!(
            on(&rule(11, 4, Thursday), 2026).as_deref(),
            Some("2026-11-26"),
            "Thanksgiving"
        );
        assert_eq!(
            on(&rule(5, -1, Monday), 2026).as_deref(),
            Some("2026-05-25"),
            "UK spring bank holiday"
        );
        assert_eq!(
            on(&rule(3, -1, Sunday), 2026).as_deref(),
            Some("2026-03-29"),
            "last Sunday of March"
        );
        assert_eq!(
            on(&rule(6, -3, Saturday), 2026).as_deref(),
            Some("2026-06-13"),
            "third Saturday from the end"
        );
        assert_eq!(
            on(&rule(2, 5, Monday), 2026),
            None,
            "no fifth Monday in February 2026"
        );
    }

    #[test]
    fn a_zone_names_its_region() {
        let table = "# comment\nIN\t+2232+08822\tAsia/Kolkata\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\n";
        assert_eq!(region_of_zone(table, "Asia/Kolkata").as_deref(), Some("in"));
        assert_eq!(
            region_of_zone(table, "America/New_York").as_deref(),
            Some("us")
        );
        assert_eq!(region_of_zone(table, "Mars/Olympus"), None);
    }

    #[test]
    fn the_feed_is_named_from_the_locale() {
        assert_eq!(feed_for("en_IN.UTF-8"), Some(("in".into(), "en".into())));
        assert_eq!(feed_for("de_DE@euro"), Some(("de".into(), "de".into())));
        assert_eq!(feed_for("C"), None);
    }
}
