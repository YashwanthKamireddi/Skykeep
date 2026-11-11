//! Reminder alerts on the desktop, rung when the phone would ring them.
//!
//! One task sleeps until the next alarm and wakes for it — no polling. It also wakes when the
//! reminders change (a pass, or a write here), when the machine resumes from sleep (logind's
//! `PrepareForSleep`), and at the latest every ten minutes, because a sleep measured on the
//! monotonic clock does not count the time a machine spent suspended and a clock can be set.
//! Every alert is recorded as rung before it is shown, so none is ever shown twice — not by
//! two wakes racing, not after a restart.

use crate::{Auth, Data};
use quince_core::alarms::DueAlarm;
use std::time::Duration;
use tauri::Emitter;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

/// Something that can move the next alarm has happened.
static CHANGED: Notify = Notify::const_new();

/// The longest the task sleeps without looking again.
const LONGEST_SLEEP: Duration = Duration::from_secs(10 * 60);

/// How late an alert may still be shown. An alarm missed by less — the machine was asleep,
/// or Quince was not running — rings once when it can; one missed by more is recorded and
/// left, so opening Quince after a week away is not a wall of old alerts.
const STILL_WORTH_SAYING: i64 = 6 * 60 * 60;

/// The latest after its event's start an alert is looked for: a day, past the 9 AM of an
/// all-day event's "on the day" alert and any after-the-start alert set on a Mac.
const LATEST_AFTER_START: i64 = 86_400;

/// The earliest start of an event whose alerts are looked at, at `at`. As far ahead as the
/// earliest alert Calendar offers — a week before — and a day more is looked at too (two days
/// ahead, a week-before alert never rang). Back, as far as an alert can ring after its event
/// starts and still be shown: an all-day event's "on the day" alert rings at 9 AM, nine hours
/// after its midnight start, and looking six hours back the event had gone before it rang.
fn coming_from(at: i64) -> i64 {
    at - STILL_WORTH_SAYING - LATEST_AFTER_START
}

/// Snooze, as the phone offers it.
const SNOOZE: i64 = 60 * 60;

/// The app's mark on its own sky-blue circle, the letters sized and set inside it —
/// notification servers crop icons to a circle of their own, which cut the full icon's letters
/// off — carried in the binary so an alert shows it on any desktop. A notification
/// names its icon either from the icon theme — where nothing is installed for a copy run
/// straight from a download or a build, so alerts came up blank — or by file path, which
/// always resolves. So it is written once beside the library and named by path.
const ICON: &[u8] = include_bytes!("../icons/notify.png");
static ICON_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// What a press on a reminder's alert does — Linux's notification actions and Windows' toast
/// buttons alike: complete this occurrence, put it off an hour, or open it in Reminders.
fn answer(app: &AppHandle, due: &DueAlarm, due_then: Option<Option<i64>>, action: &str) {
    match action {
        "complete" => {
            let app = app.clone();
            let due = due.clone();
            tauri::async_runtime::block_on(async move {
                let auth = app.state::<Auth>();
                let data = app.state::<Data>();
                let now = data
                    .0
                    .lock()
                    .await
                    .reminder(&due.reminder_guid)
                    .ok()
                    .flatten();
                let same = now
                    .as_ref()
                    .is_some_and(|r| r.completed_at.is_none() && Some(r.due_at) == due_then);
                if !same {
                    quiet!("an alert was answered after its reminder had moved on");
                } else if let Err(e) =
                    crate::set_done(&auth, &data, &due.reminder_guid, &due.list_guid, true).await
                {
                    quiet!("an alert's reminder could not be completed: {e}");
                }
                // Reminders looks again; no other screen has anything new.
                let report = crate::refresh::status_changed(&data.0, &["reminders"]).await;
                app.emit(crate::SYNCED, &report).ok();
            });
        }
        "snooze" => {
            let app = app.clone();
            let guid = due.reminder_guid.clone();
            tauri::async_runtime::block_on(async move {
                let data = app.state::<Data>();
                let mut library = data.0.lock().await;
                library.snooze_reminder(&guid, now() + SNOOZE).ok();
            });
            changed();
        }
        "default" => {
            // Reminders' own window, open on that reminder in its list.
            crate::apps::reveal(
                app,
                "reminders",
                &serde_json::json!({
                    "screen": "reminders",
                    "id": due.reminder_guid,
                    "within": due.list_guid,
                }),
            );
        }
        _ => {}
    }
}

/// A plain notice from the app itself — not a reminder or an event — sent as Skykeep, and
/// opening the window when clicked where the desktop allows it.
pub fn tell(app: &AppHandle, title: &str, body: &str) {
    let app = app.clone();
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        let mut notice = notify_rust::Notification::new();
        #[cfg(windows)]
        notice.app_id(&crate::taskbar::own_id(&app.config().identifier));
        notice
            .appname("Skykeep")
            .summary(&title)
            .body(&body)
            .icon(icon_path(&app).unwrap_or("quince"));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let _ = notice;
            let notice = crate::notices::Notice {
                summary: &title,
                body: &body,
                icon: icon_path(&app).unwrap_or("quince"),
                actions: &[("default", "Open")],
                category: None,
                desktop_entry: Some(app.config().identifier.clone()),
                sound: crate::notices::Sound::Default,
                resident: false,
            };
            let app = app.clone();
            crate::notices::show(&notice, move |action| {
                if action == "default" {
                    crate::apps::open_skykeep(&app);
                }
            });
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        notice.show().ok();
    });
}

/// An alert's sound: the desktop's own notification sound ("message-new-instant" in its sound
/// theme), as Reminders and Calendar on a Mac use the system's alert sound rather than an alarm.
/// The theme's "alarm-clock-elapsed" rang like an alarm clock, unlike anything else on the desktop.
/// A notification server that plays sounds is asked for it; one that does not — Quickshell,
/// mako, dunst and others say no "sound" capability — leaves it to the app, as the spec says,
/// and it is played here through libcanberra, which keeps to the sound theme and to the
/// desktop's "event sounds" switch, and the server is asked to add none (Quickshell plays one
/// anyway, and the alert sounded twice). Without this every alert on such a desktop was silent.
#[cfg(all(unix, not(target_os = "macos")))]
fn chime() -> crate::notices::Sound {
    const SOUND: &str = "message-new-instant";
    static SERVER_PLAYS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let server_plays = *SERVER_PLAYS.get_or_init(|| {
        notify_rust::get_capabilities().is_ok_and(|c| c.iter().any(|x| x == "sound"))
    });
    if server_plays {
        return crate::notices::Sound::Named(SOUND);
    }
    // The alert never stands behind its sound, and the sound is waited for on a thread of its own:
    // left unwaited, every alert left a finished player behind as a zombie process.
    match std::process::Command::new("canberra-gtk-play")
        .args(["--id", SOUND, "--description", "Skykeep alert"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(mut player) => {
            std::thread::spawn(move || {
                let _ = player.wait();
            });
            crate::notices::Sound::PlayedHere
        }
        // No player here: the server is left to sound it, if it does.
        Err(_) => crate::notices::Sound::Default,
    }
}

fn icon_path(app: &AppHandle) -> Option<&'static str> {
    ICON_PATH
        .get_or_init(|| {
            let path = crate::state_dir(app).join("quince-alert.png");
            let current = std::fs::read(&path).ok();
            if current.as_deref() != Some(ICON) {
                std::fs::create_dir_all(path.parent()?).ok()?;
                std::fs::write(&path, ICON).ok()?;
            }
            Some(path.to_string_lossy().into_owned())
        })
        .as_deref()
}

/// Tell the task to look again: the reminders or their alarms may have changed.
pub fn changed() {
    CHANGED.notify_one();
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// What is due now, each recorded as rung, and when to look next.
///
/// Recording happens here, under the library's lock, before anything is shown: whatever
/// happens to the notification afterwards, this alert has had its one chance.
pub fn take_due(library: &mut quince_core::Library, now: i64) -> (Vec<DueAlarm>, Option<i64>) {
    let mut ring = Vec::new();
    for due in library.due_alarms(now).unwrap_or_default() {
        if library
            .mark_alarm_fired(&due.key, due.fires_at, now)
            .is_err()
        {
            // Not recorded means it could ring again; better to stay quiet than to repeat.
            continue;
        }
        if now - due.fires_at <= STILL_WORTH_SAYING {
            ring.push(due);
        }
    }
    (ring, library.next_alarm_after(now).unwrap_or(None))
}

/// Start the task. It runs for the life of the process, window open or not.
/// Settings' switches for each kind of alert. An alert due while its switch is off is let
/// go, not saved for later: turned back on, it does not ring for what has passed.
pub const REMINDER_ALERTS: &str = "settings.alerts.reminders";
pub const EVENT_ALERTS: &str = "settings.alerts.events";

/// Whether a kind of alert may ring. On unless Settings turned it off.
pub fn allowed(library: &quince_core::Library, key: &str) -> bool {
    library.state(key).ok().flatten().as_deref() != Some("off")
}

pub fn spawn(app: AppHandle) {
    watch_for_resume();
    tauri::async_runtime::spawn(async move {
        loop {
            let at = now();
            // Only the signed-in account's reminders ring: after a sign-out (its copy kept) or
            // while another account is signing in, the library open is not theirs to show.
            // Nothing is taken as rung meanwhile, so the account's own alerts still come when
            // it is back.
            if !crate::same_account(&app).await {
                tokio::select! {
                    () = tokio::time::sleep(LONGEST_SLEEP) => {}
                    () = CHANGED.notified() => {}
                }
                continue;
            }
            let (ring, next, on) = {
                let data = app.state::<Data>();
                let mut library = data.0.lock().await;
                let (ring, next) = take_due(&mut library, at);
                (ring, next, allowed(&library, REMINDER_ALERTS))
            };
            for due in ring.into_iter().filter(|_| on) {
                show(app.clone(), due);
            }
            let wait = next.map_or(LONGEST_SLEEP, |t| {
                Duration::from_secs(u64::try_from(t - at).unwrap_or(1).max(1)).min(LONGEST_SLEEP)
            });
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = CHANGED.notified() => {}
            }
        }
    });
}

/// Wake when the machine comes back from sleep, where logind says so. Where it cannot be
/// asked — no system bus, no logind, not Linux — the ten-minute ceiling still holds.
#[cfg(not(target_os = "linux"))]
fn watch_for_resume() {}

#[cfg(target_os = "linux")]
fn watch_for_resume() {
    tauri::async_runtime::spawn(async {
        let Ok(bus) = zbus::Connection::system().await else {
            return;
        };
        let Ok(logind) = zbus::Proxy::new(
            &bus,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .await
        else {
            return;
        };
        let Ok(mut signals) = logind.receive_signal("PrepareForSleep").await else {
            return;
        };
        use futures_util::StreamExt;
        while let Some(signal) = signals.next().await {
            // `true` on the way down, `false` on the way back up.
            if let Ok(false) = signal.body().deserialize::<bool>() {
                changed();
            }
        }
    });
}

/// The alert, with the phone's two choices on it, waiting for an answer on its own thread.
fn show(app: AppHandle, due: DueAlarm) {
    std::thread::spawn(move || {
        let body = due
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(ToString::to_string)
            .or_else(|| due.list_title.clone())
            .unwrap_or_default();
        // The occurrence this alert is for: a repeating reminder ticked since (here or on the
        // phone) has moved on, and a press on this alert must not move it on again.
        let due_then = {
            let app = app.clone();
            let guid = due.reminder_guid.clone();
            tauri::async_runtime::block_on(async move {
                let data = app.state::<Data>();
                let library = data.0.lock().await;
                library.reminder(&guid).ok().flatten().map(|r| r.due_at)
            })
        };
        // On Windows the phone's choices are the toast's own buttons, and the toast is sent as
        // the app the Start menu knows (the installer stamps its shortcut with this ID; without
        // it every alert came from "Windows PowerShell"), with Windows' reminder chime.
        #[cfg(windows)]
        {
            use tauri_winrt_notification::{Duration as Shown, Scenario, Sound, Toast};
            let answering = app.clone();
            let due_answered = due.clone();
            let mut toast = Toast::new(&crate::taskbar::own_id(&app.config().identifier))
                .title(&due.title)
                .text1(&body)
                .sound(Some(Sound::Reminder))
                .duration(Shown::Long)
                .scenario(Scenario::Reminder)
                .add_button("Mark as Completed", "complete")
                .add_button("Remind Me in 1 Hour", "snooze")
                .on_activated(move |action| {
                    answer(
                        &answering,
                        &due_answered,
                        due_then,
                        action.as_deref().unwrap_or("default"),
                    );
                    Ok(())
                });
            if let Some(icon) = icon_path(&app) {
                toast = toast.icon(
                    std::path::Path::new(icon),
                    tauri_winrt_notification::IconCrop::Square,
                    "Skykeep",
                );
            }
            toast.show().ok();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let notice = crate::notices::Notice {
                summary: &due.title,
                body: &body,
                icon: icon_path(&app).unwrap_or("quince"),
                actions: &[
                    ("default", "Open"),
                    ("complete", "Mark as Completed"),
                    ("snooze", "Remind Me in 1 Hour"),
                ],
                category: Some("reminder"),
                desktop_entry: Some(format!("{}.Reminders", app.config().identifier)),
                sound: chime(),
                resident: true,
            };
            let (answering, due_answered) = (app.clone(), due.clone());
            crate::notices::show(&notice, move |action| {
                answer(&answering, &due_answered, due_then, action);
            });
        }
    });
}

/// Alerts read for events no longer coming let go of.
fn forget_gone<T>(
    known: &mut std::collections::HashMap<String, T>,
    coming: &[quince_core::calendar::Event],
) {
    let still: std::collections::HashSet<&str> = coming.iter().map(|e| e.guid.as_str()).collect();
    known.retain(|guid, _| still.contains(guid.as_str()));
}

/// Calendar alerts on the desktop, rung as the phone rings them.
///
/// Apple keeps an event's alerts beside it, read one event at a time, so this looks only at
/// what is coming: the events starting in the next two days, their alerts asked for once and
/// asked again only when the event changes. Each alert is recorded as rung before it shows,
/// under the same rule as reminders' — never twice, and one missed by more than a few hours
/// is let go rather than shown late.
pub fn spawn_events(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Alerts already read, by event: its version, and minutes before its start.
        let mut known: std::collections::HashMap<String, (Option<String>, Vec<i64>)> =
            std::collections::HashMap::new();
        loop {
            let at = now();
            // As reminders: only the signed-in account's events are said (CalDAV accounts are
            // the account's too, kept in its library).
            if !crate::same_account(&app).await {
                tokio::select! {
                    () = tokio::time::sleep(LONGEST_SLEEP) => {}
                    () = CHANGED.notified() => {}
                }
                continue;
            }
            let coming: Vec<quince_core::calendar::Event> = {
                let data = app.state::<Data>();
                let library = data.0.lock().await;
                library
                    .events_starting_between(coming_from(at), at + 8 * 86_400)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(guid, _)| {
                        !guid.starts_with("quince:") || quince_core::dav::is_dav(guid)
                    })
                    .filter_map(|(guid, _)| library.event(&guid).ok().flatten())
                    .collect()
            };
            // What is no longer coming is forgotten: kept, the map grew by every event that
            // ever came near, for as long as the app ran.
            forget_gone(&mut known, &coming);
            // Ask for the alerts of any event not yet read, or changed since.
            let stale: Vec<_> = coming
                .iter()
                .filter(|e| known.get(&e.guid).is_none_or(|(tag, _)| *tag != e.etag))
                .cloned()
                .collect();
            // A calendar account's alerts are in the text kept here; only iCloud's are asked for.
            let (accounts, stale): (Vec<_>, Vec<_>) =
                stale.into_iter().partition(crate::dav::is_account_event);
            for e in &accounts {
                let alerts = crate::dav::alerts_of(&app.state::<Data>(), e).await;
                known.insert(e.guid.clone(), (e.etag.clone(), alerts));
            }
            if !stale.is_empty() {
                let session = {
                    let auth = app.state::<Auth>();
                    let client = auth.client.lock().await;
                    client
                        .session()
                        .service_url("calendar")
                        .map(|root| (client.http().clone(), root, client.service_params()))
                };
                if let Some((http, root, params)) = session {
                    let cal = quince_icloud::calendar::CalendarClient::new(&http, &root, params);
                    for e in &stale {
                        if let Ok(alerts) = cal.alerts(e).await {
                            known.insert(e.guid.clone(), (e.etag.clone(), alerts));
                        }
                    }
                }
            }
            let mut next: Option<i64> = None;
            for e in &coming {
                let Some((_, alerts)) = known.get(&e.guid) else {
                    continue;
                };
                for minutes in alerts {
                    let fires_at = e.starts_at - minutes * 60;
                    if fires_at > at {
                        next = Some(next.map_or(fires_at, |n| n.min(fires_at)));
                        continue;
                    }
                    if at - fires_at > STILL_WORTH_SAYING {
                        continue;
                    }
                    let key = format!("event:{}:{minutes}", e.guid);
                    let ring = {
                        let data = app.state::<Data>();
                        let mut library = data.0.lock().await;
                        !library.alarm_already_fired(&key, fires_at).unwrap_or(true)
                            && library.mark_alarm_fired(&key, fires_at, at).is_ok()
                            && allowed(&library, EVENT_ALERTS)
                    };
                    if ring {
                        show_event(app.clone(), e.clone(), *minutes);
                    }
                }
            }
            // Until the next alert, but look again every two minutes for new and moved events.
            let wait = next.map_or(120, |t| (t - at).clamp(1, 120));
            tokio::time::sleep(Duration::from_secs(u64::try_from(wait).unwrap_or(120))).await;
        }
    });
}

/// "Dentist" — "In 15 minutes · 4:30 PM", as Calendar words an alert.
fn show_event(app: AppHandle, event: quince_core::calendar::Event, minutes: i64) {
    std::thread::spawn(move || {
        let zone = jiff::tz::TimeZone::system();
        let twelve = crate::appearance::system_time_locale()
            .as_deref()
            .is_none_or(twelve_hour);
        let mut body = event_lead(&event, minutes, &zone, twelve);
        if let Some(place) = event.location.as_deref().filter(|p| !p.trim().is_empty()) {
            body.push_str(&format!("\n{place}"));
        }
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        {
            let mut toast = notify_rust::Notification::new();
            // Windows names a toast's sender by the app ID it claims, matched to the Start
            // menu shortcut the installer stamps with the same ID; without it every alert
            // came from "Windows PowerShell".
            #[cfg(windows)]
            toast.app_id(&crate::taskbar::own_id(&app.config().identifier));
            // Windows' own reminder chime; without a sound named the toast was silent.
            #[cfg(windows)]
            toast.sound_name("Reminder");
            toast
                .appname("Skykeep")
                .summary(&event.title)
                .body(&body)
                .icon(icon_path(&app).unwrap_or("quince"))
                .show()
                .ok();
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            let notice = crate::notices::Notice {
                summary: &event.title,
                body: &body,
                icon: icon_path(&app).unwrap_or("quince"),
                actions: &[("default", "Open")],
                category: Some("calendar"),
                desktop_entry: Some(format!("{}.Calendar", app.config().identifier)),
                sound: chime(),
                resident: false,
            };
            // Calendar's own window, on the event's day, its calendar showing.
            let shown = serde_json::json!({
                "screen": "calendar",
                "id": event.guid,
                "within": event.calendar_guid,
                "at": event.starts_at,
            });
            let app = app.clone();
            crate::notices::show(&notice, move |action| {
                if action == "default" {
                    crate::apps::reveal(&app, "calendar", &shown);
                }
            });
        }
    });
}

/// What an event's alert says under its title. A timed event: how long until it starts and
/// when, on this computer's clock — "In 15 minutes · 4:30 PM", or "16:30" where the clock is a
/// 24-hour one. An all-day event has no time: the day, as Calendar says it — "Today",
/// "Tomorrow", "In 2 days" — counted from the day the alert rings. Its 9 AM alerts had read
/// "In 15 hours" and "Started 540 minutes ago".
fn event_lead(
    event: &quince_core::calendar::Event,
    minutes: i64,
    zone: &jiff::tz::TimeZone,
    twelve: bool,
) -> String {
    let local = |s: i64| {
        jiff::Timestamp::from_second(s)
            .ok()
            .map(|t| t.to_zoned(zone.clone()))
    };
    if event.all_day {
        let days = local(event.starts_at)
            .zip(local(event.starts_at - minutes * 60))
            .and_then(|(day, rung)| rung.date().until(day.date()).ok())
            .map_or(0, |span| i64::from(span.get_days()));
        return match days {
            ..=0 => "Today".to_string(),
            1 => "Tomorrow".to_string(),
            n => format!("In {n} days"),
        };
    }
    let when = local(event.starts_at)
        .map(|t| {
            t.strftime(if twelve { "%-I:%M %p" } else { "%H:%M" })
                .to_string()
        })
        .unwrap_or_default();
    let lead = match minutes {
        0 => "Now".to_string(),
        m if m % 1440 == 0 => {
            format!("In {} day{}", m / 1440, if m == 1440 { "" } else { "s" })
        }
        m if m % 60 == 0 => format!("In {} hour{}", m / 60, if m == 60 { "" } else { "s" }),
        m if m > 0 => format!("In {m} minutes"),
        m => format!("Started {} minutes ago", -m),
    };
    format!("{lead} · {when}")
}

/// Whether a locale's clock is a twelve-hour one: CLDR's preferred hour cycle, by region. A
/// language alone (`fr`) or a region not listed reads a 24-hour clock.
fn twelve_hour(tag: &str) -> bool {
    const TWELVE: &[&str] = &[
        "AE", "AG", "AL", "AS", "AU", "BB", "BD", "BH", "BM", "BN", "BS", "BT", "CA", "CO", "DM",
        "DZ", "EG", "EH", "ER", "ET", "FJ", "FM", "GH", "GM", "GU", "GY", "HK", "IN", "IQ", "JM",
        "JO", "KH", "KI", "KN", "KP", "KR", "KW", "KY", "LB", "LC", "LR", "LS", "LY", "MH", "MO",
        "MP", "MR", "MW", "MY", "NA", "NZ", "OM", "PA", "PG", "PH", "PK", "PR", "PS", "PW", "QA",
        "SA", "SB", "SD", "SL", "SO", "SS", "SY", "SZ", "TC", "TD", "TN", "TO", "TT", "TW", "UM",
        "US", "VC", "VG", "VI", "VU", "WS", "YE", "ZM",
    ];
    // French Canada keeps a 24-hour clock where English Canada does not.
    if tag.starts_with("fr-CA") {
        return false;
    }
    tag.split('-')
        .skip(1)
        .find(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_uppercase()))
        .is_some_and(|region| TWELVE.contains(&region))
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_all_day_alert_names_the_day_not_the_hours() {
        let zone = jiff::tz::TimeZone::get("Asia/Kolkata").unwrap();
        // Midnight on 2 November 2026 in Kolkata.
        let midnight = jiff::civil::date(2026, 11, 2)
            .to_zoned(zone.clone())
            .unwrap()
            .timestamp()
            .as_second();
        let event = quince_core::calendar::Event {
            title: "Trip".into(),
            starts_at: midnight,
            ends_at: midnight + 86_400,
            all_day: true,
            ..Default::default()
        };
        assert_eq!(super::event_lead(&event, -540, &zone, true), "Today");
        assert_eq!(super::event_lead(&event, 900, &zone, true), "Tomorrow");
        assert_eq!(super::event_lead(&event, 2340, &zone, true), "In 2 days");
        assert_eq!(super::event_lead(&event, 9540, &zone, true), "In 7 days");
    }

    #[test]
    fn a_timed_alert_says_when_on_this_computers_clock() {
        let zone = jiff::tz::TimeZone::get("Asia/Kolkata").unwrap();
        let start = jiff::civil::date(2026, 11, 2)
            .at(16, 30, 0, 0)
            .to_zoned(zone.clone())
            .unwrap()
            .timestamp()
            .as_second();
        let event = quince_core::calendar::Event {
            starts_at: start,
            ends_at: start + 3_600,
            ..Default::default()
        };
        assert_eq!(
            super::event_lead(&event, 15, &zone, true),
            "In 15 minutes · 4:30 PM"
        );
        assert_eq!(
            super::event_lead(&event, 60, &zone, false),
            "In 1 hour · 16:30"
        );
        assert_eq!(super::event_lead(&event, 0, &zone, false), "Now · 16:30");
    }

    #[test]
    fn an_on_the_day_alert_is_looked_for_after_its_event_starts() {
        let midnight = 1_793_000_000;
        // 9 AM, when the alert rings, and 2 PM, five hours late after a sleep: the event that
        // started at midnight is still among those looked at.
        for at in [midnight + 9 * 3_600, midnight + 14 * 3_600] {
            assert!(super::coming_from(at) <= midnight);
        }
    }

    #[test]
    fn the_clock_follows_the_locale_region() {
        assert!(super::twelve_hour("en-US"));
        assert!(super::twelve_hour("en-IN"));
        assert!(!super::twelve_hour("en-GB"));
        assert!(!super::twelve_hour("de-DE"));
        assert!(!super::twelve_hour("fr-CA"));
        assert!(!super::twelve_hour("fr"));
    }

    #[test]
    fn alerts_of_events_no_longer_coming_are_let_go_of() {
        let mut known: std::collections::HashMap<String, ()> =
            [("past".to_string(), ()), ("soon".to_string(), ())].into();
        let soon = quince_core::calendar::Event {
            guid: "soon".into(),
            ..Default::default()
        };
        super::forget_gone(&mut known, &[soon]);
        assert_eq!(known.keys().collect::<Vec<_>>(), ["soon"]);
    }

    #[test]
    fn an_alert_rings_unless_settings_turned_it_off() {
        let mut library = quince_core::Library::in_memory().unwrap();
        assert!(allowed(&library, EVENT_ALERTS), "on until turned off");
        library.set_state(EVENT_ALERTS, "off").unwrap();
        assert!(!allowed(&library, EVENT_ALERTS));
        assert!(
            allowed(&library, REMINDER_ALERTS),
            "one switch does not turn off the other"
        );
        library.set_state(EVENT_ALERTS, "on").unwrap();
        assert!(allowed(&library, EVENT_ALERTS));
    }

    use super::*;
    use quince_core::alarms::{Alarm, AlarmTrigger};
    use quince_core::reminders::{Reminder, ReminderList};

    fn library_with(fires: &[(&str, i64)]) -> quince_core::Library {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = quince_core::Library::open(&dir.keep()).unwrap();
        lib.put_reminder_lists(&[ReminderList {
            guid: "L".into(),
            title: "Home".into(),
            enabled: true,
            ..Default::default()
        }])
        .unwrap();
        let mut reminders = Vec::new();
        let mut alarms = Vec::new();
        let mut triggers = Vec::new();
        for (id, at) in fires {
            reminders.push(Reminder {
                guid: format!("R{id}"),
                list_guid: "L".into(),
                title: format!("Do {id}"),
                ..Default::default()
            });
            alarms.push(Alarm {
                id: format!("A{id}"),
                reminder_guid: format!("R{id}"),
                trigger_id: format!("T{id}"),
                acknowledged_at: None,
            });
            triggers.push(AlarmTrigger {
                id: format!("T{id}"),
                fires_at: *at,
            });
        }
        lib.put_reminders(&reminders).unwrap();
        lib.put_alarms(&alarms, &triggers, &[], true).unwrap();
        lib
    }

    #[test]
    fn a_due_alert_is_taken_once_and_the_next_is_named() {
        let mut lib = library_with(&[("now", 1_000_000), ("later", 1_003_600)]);
        let (ring, next) = take_due(&mut lib, 1_000_000);
        assert_eq!(
            ring.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            ["Do now"]
        );
        assert_eq!(next, Some(1_003_600));
        // A second wake at the same moment — resume and a pass together — rings nothing.
        let (again, _) = take_due(&mut lib, 1_000_000);
        assert!(again.is_empty());
    }

    #[test]
    fn missed_while_away_rings_once_but_long_gone_stays_quiet() {
        let now = 10_000_000;
        let mut lib = library_with(&[
            ("an hour ago", now - 3_600),
            ("last week", now - 7 * 86_400),
        ]);
        let (ring, _) = take_due(&mut lib, now);
        assert_eq!(
            ring.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            ["Do an hour ago"]
        );
        // And the old one is recorded, not left to surface later.
        assert!(lib.due_alarms(now).unwrap().is_empty());
    }
}
