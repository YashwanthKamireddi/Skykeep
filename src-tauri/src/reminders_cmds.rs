//! Reminders commands for what sits beside a reminder: tags, its link, an early alert, custom
//! smart lists, and sections. Reads come from the library; writes go to Apple (each refused
//! there whenever `REMINDER_DETAIL_WRITES` is off) and the library is read again after.

use crate::{Auth, Data, UiResult};
use quince_core::reminders::{
    Reminder, ReminderDetail, ReminderExtras, ReminderMarks, ReminderMore, ReminderSection,
    ReminderSmartList, SmartListContents, TagCount,
};
use quince_icloud::reminders::{details, more};
use tauri::State;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// One reminder's tags, link and early alerts.
#[tauri::command]
pub async fn reminder_detail(guid: String, data: State<'_, Data>) -> UiResult<ReminderDetail> {
    data.0
        .read(|library| library.reminder_detail(&guid))
        .await
        .map_err(|e| e.to_string())
}

/// The rest of one reminder's Details: its repeat whole, time zone, a place, the person it
/// comes up for when messaging — from the library — and Urgent, which lives in a small file
/// read from Apple the first time it is asked for (read only; offline it stays unknown).
#[tauri::command]
pub async fn reminder_more(
    guid: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<ReminderMore> {
    // A read beside any sync pass (`LibraryLock::read`), not the writer's turn: one read
    // queued for the writer behind a long Photos store sent every other window's reads into the
    // same queue (desktop audit H1).
    let (mut more, urgent_unread) = data
        .0
        .read(|library| {
            let reminder = library.reminder(&guid)?;
            // The record as Apple sent it; the reminder as read for the window leaves it out.
            let raw = library.reminder_raw(&guid)?;
            let raw = raw.as_deref();
            let parts = library.reminder_parts(&guid)?;
            let mut more = more::more_from(
                &guid,
                raw,
                reminder.as_ref().and_then(|r| r.timezone.as_deref()),
                &parts,
            );
            if let Some(person) = more.messaging.as_mut() {
                person.name = library.contact_name_for(&person.handles)?;
            }
            more.urgent = more::urgent_known(raw);
            let unread = more.urgent.is_none() && raw.is_some();
            Ok((more, unread))
        })
        .await
        .map_err(|e| e.to_string())?;
    if urgent_unread {
        if let Ok((http, root, params)) = crate::cloudkit_context(&auth).await {
            more.urgent = more::urgent_cloudkit(&http, &root, params, &guid)
                .await
                .ok()
                .flatten();
        }
    }
    Ok(more)
}

/// Reminders whose tick now is the last of their repeat (End Repeat On Date): ticked, each is
/// completed rather than moved on, so the window shows it done as it does a one-off.
#[tauri::command]
pub async fn reminders_ending(data: State<'_, Data>) -> UiResult<Vec<String>> {
    let at = now();
    // Beside a sync pass, as every pure read is (see `reminder_more`).
    data.0
        .read(|library| {
            let mut out = Vec::new();
            for (guid, end) in library.reminder_repeat_ends()? {
                if let Some(r) = library.reminder(&guid)? {
                    let days = library.reminder_repeat_days(&guid)?;
                    if r.tick_ends_series_on(Some(end), &days, at) {
                        out.push(guid);
                    }
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| e.to_string())
}

/// Told to the window as Urgent is read from Apple for rows the library could not say: each
/// reminder's id and whether it is Urgent.
pub const URGENT_READ: &str = "quince://reminder-urgent";

/// One row's Urgent, read from Apple after its marks were answered.
#[derive(Clone, serde::Serialize)]
pub struct UrgentRead {
    guid: String,
    urgent: bool,
}

/// Reminders whose Urgent file is being read now: a second read of the list while the first is
/// still at Apple does not ask for the same files again.
static URGENT_ASKING: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
    std::sync::Mutex::new(None);

/// Of `unread`, those not being read already, now taken as being read.
fn take_unasked(unread: Vec<(String, String)>) -> Vec<(String, String)> {
    let Ok(mut asking) = URGENT_ASKING.lock() else {
        return Vec::new();
    };
    let asking = asking.get_or_insert_with(Default::default);
    unread
        .into_iter()
        .filter(|(guid, _)| asking.insert(guid.clone()))
        .collect()
}

fn done_asking(guids: &[String]) {
    if let Ok(mut asking) = URGENT_ASKING.lock() {
        if let Some(asking) = asking.as_mut() {
            for guid in guids {
                asking.remove(guid);
            }
        }
    }
}

/// What each of these reminders' rows shows beside its date, as the phone's list does:
/// Weekdays or Weekends, the Urgent alarm, a place, a person, a link. Only rows with any of
/// them come back, from the library, at once. Urgent is shown off until it is known: for rows
/// not read yet it is read from Apple behind the answer — a few at a time, read only — and
/// told to the window as [`URGENT_READ`]. Waiting for those files held the whole answer, so a
/// list's place, person and link chips came seconds after its rows, and every row with one
/// grew by a line as they did; offline, each read waited out the timeouts.
#[tauri::command]
pub async fn reminder_marks(
    guids: Vec<String>,
    app: tauri::AppHandle,
    data: State<'_, Data>,
) -> UiResult<Vec<ReminderMarks>> {
    /// Urgent files read for one call at most; the rest are read on the next.
    const MOST_UNREAD: usize = 40;
    // Beside a sync pass, as every pure read is (see `reminder_more`): asked on every change of
    // the rows shown, this one under the writer's turn held every window's reads behind a store.
    let (mut out, unread) = data
        .0
        .read(|library| {
            let mut out = Vec::new();
            let mut unread: Vec<(String, String)> = Vec::new();
            for guid in guids.iter().take(2_000) {
                let raw = library.reminder_raw(guid)?;
                let parts = library.reminder_parts(guid)?;
                let found = more::more_from(guid, raw.as_deref(), None, &parts);
                let messaging = match found.messaging {
                    Some(person) => library
                        .contact_name_for(&person.handles)?
                        .or_else(|| person.handles.first().cloned()),
                    None => None,
                };
                let url = library.reminder_detail(guid)?.url;
                let urgent = more::urgent_known(raw.as_deref());
                out.push(ReminderMarks {
                    guid: guid.clone(),
                    days_of_week: found.repeat.map(|r| r.days_of_week).unwrap_or_default(),
                    urgent: urgent.unwrap_or(false),
                    location: found.location,
                    messaging,
                    url,
                });
                if urgent.is_none() && unread.len() < MOST_UNREAD {
                    if let Some(raw) = raw {
                        // One that failed lately is left until it is worth asking again.
                        if !more::urgent_failed_lately(guid, &raw) {
                            unread.push((guid.clone(), raw));
                        }
                    }
                }
            }
            Ok((out, unread))
        })
        .await
        .map_err(|e| e.to_string())?;
    let unread = take_unasked(unread);
    if !unread.is_empty() {
        tauri::async_runtime::spawn(read_urgent(app, unread));
    }
    out.retain(|m| {
        m.urgent
            || !m.days_of_week.is_empty()
            || m.location.is_some()
            || m.messaging.is_some()
            || m.url.is_some()
    });
    Ok(out)
}

/// Read these rows' Urgent files from Apple, six at a time, and tell the window what came back.
async fn read_urgent(app: tauri::AppHandle, unread: Vec<(String, String)>) {
    use futures_util::StreamExt;
    use tauri::{Emitter, Manager};
    let guids: Vec<String> = unread.iter().map(|(guid, _)| guid.clone()).collect();
    let auth = app.state::<Auth>();
    if let Ok((http, root, params)) = crate::cloudkit_context(&auth).await {
        let read: Vec<UrgentRead> = futures_util::stream::iter(unread)
            .map(|(guid, raw)| {
                let (http, root, params) = (http.clone(), root.clone(), params.clone());
                async move {
                    let urgent = more::urgent_for(&http, &root, params, &guid, &raw).await;
                    urgent.map(|urgent| UrgentRead { guid, urgent })
                }
            })
            .buffer_unordered(6)
            .filter_map(|found| async move { found })
            .collect()
            .await;
        if !read.is_empty() {
            let _ = app.emit(URGENT_READ, read);
        }
    }
    done_asking(&guids);
}

/// Every tag on the account, with how many reminders still to do carry each.
#[tauri::command]
pub async fn reminder_tags(data: State<'_, Data>) -> UiResult<Vec<TagCount>> {
    data.0
        .read(|library| library.reminder_tags())
        .await
        .map_err(|e| e.to_string())
}

/// Reminders still to do carrying a tag.
#[tauri::command]
pub async fn reminders_tagged(name: String, data: State<'_, Data>) -> UiResult<Vec<Reminder>> {
    data.0
        .read(|library| library.reminders_tagged(&name))
        .await
        .map_err(|e| e.to_string())
}

/// Each reminder still to do with its tags, and how many each custom smart list holds: the
/// sidebar's and the rows' tag chips in one read, rather than one per tag and per list.
#[tauri::command]
pub async fn reminder_extras(data: State<'_, Data>) -> UiResult<ReminderExtras> {
    let at = now();
    data.0
        .read(|library| library.reminder_extras(at))
        .await
        .map_err(|e| e.to_string())
}

/// The account's smart lists, built-in and custom.
#[tauri::command]
pub async fn reminder_smart_lists(data: State<'_, Data>) -> UiResult<Vec<ReminderSmartList>> {
    data.0
        .read(|library| library.reminder_smart_lists())
        .await
        .map_err(|e| e.to_string())
}

/// What a custom smart list holds now, and any part of its filter not understood.
#[tauri::command]
pub async fn reminders_in_smart_list(
    guid: String,
    data: State<'_, Data>,
) -> UiResult<SmartListContents> {
    let at = now();
    data.0
        .read(|library| library.reminders_in_smart_list(&guid, at))
        .await
        .map_err(|e| e.to_string())
}

/// A write's result, with the library read again either way: on success it brings Apple's
/// copy, on a refusal it puts back whatever was shown ahead of Apple.
async fn finish<T>(
    result: Result<T, quince_icloud::Error>,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    data: &State<'_, Data>,
) -> UiResult<T> {
    let refreshed = crate::resync_reminders(http, root, params, data).await;
    let value = result.map_err(|e| e.to_string())?;
    crate::settle(refreshed)?;
    Ok(value)
}

/// Tag a reminder.
#[tauri::command]
pub async fn reminder_tag_add(
    guid: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let made = details::add_tag_cloudkit(&http, &root, params.clone(), &guid, &name).await;
    finish(made.map(|_| ()), &http, &root, params, &data).await
}

/// Take a tag off a reminder.
#[tauri::command]
pub async fn reminder_tag_remove(
    guid: String,
    name: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let done = details::remove_tag_cloudkit(&http, &root, params.clone(), &guid, &name).await;
    finish(done, &http, &root, params, &data).await
}

/// Set a reminder's link, or clear it with `null`.
#[tauri::command]
pub async fn reminder_url_set(
    guid: String,
    url: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let url = url.filter(|u| !u.trim().is_empty());
    let done = details::set_url_cloudkit(&http, &root, params.clone(), &guid, url.as_deref()).await;
    finish(done, &http, &root, params, &data).await
}

/// Remind early: `count` units before it is due (unit 0 minutes, 1 hours, 2 days, 3 weeks,
/// 4 months), or `null` to clear.
#[tauri::command]
pub async fn reminder_early_alert_set(
    guid: String,
    count: Option<i64>,
    unit: Option<i64>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let account = {
        let library = data.0.lock().await;
        library.early_alert_account().map_err(|e| e.to_string())?
    };
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let alert = count.map(|c| (c, unit.unwrap_or(0)));
    let done = details::set_early_alert_cloudkit(
        &http,
        &root,
        params.clone(),
        &guid,
        alert,
        account.as_deref(),
    )
    .await;
    finish(done, &http, &root, params, &data).await
}

/// Make a section in a list. Answers it, already stored.
#[tauri::command]
pub async fn reminder_section_create(
    list_guid: String,
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<ReminderSection> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let made =
        details::create_section_cloudkit(&http, &root, params.clone(), &list_guid, &title).await;
    if let Ok(section) = &made {
        let mut library = data.0.lock().await;
        library
            .put_reminder_section_local(section)
            .map_err(|e| e.to_string())?;
    }
    finish(made, &http, &root, params, &data).await
}

/// Rename a section.
#[tauri::command]
pub async fn reminder_section_rename(
    guid: String,
    title: String,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let done = details::rename_section_cloudkit(&http, &root, params.clone(), &guid, &title).await;
    finish(done, &http, &root, params, &data).await
}

/// Move a reminder into a section of its list, or out of every one with `null`. Shown here
/// at once; a refusal puts it back as Apple has it.
#[tauri::command]
pub async fn reminder_section_move(
    list_guid: String,
    guid: String,
    section: Option<String>,
    auth: State<'_, Auth>,
    data: State<'_, Data>,
) -> UiResult<()> {
    {
        let mut library = data.0.lock().await;
        library
            .set_reminder_section(&guid, section.as_deref())
            .map_err(|e| e.to_string())?;
    }
    let (http, root, params) = crate::cloudkit_context(&auth).await?;
    let done = details::move_to_section_cloudkit(
        &http,
        &root,
        params.clone(),
        &list_guid,
        &guid,
        section.as_deref(),
    )
    .await;
    finish(done, &http, &root, params, &data).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row whose Urgent file is on its way is not asked for again until that read is over.
    #[test]
    fn a_row_being_read_is_not_asked_for_twice() {
        let row = |g: &str| (g.to_string(), "{}".to_string());
        let first = take_unasked(vec![row("Reminder/U1"), row("Reminder/U2")]);
        assert_eq!(first.len(), 2);
        let again = take_unasked(vec![row("Reminder/U1"), row("Reminder/U3")]);
        assert_eq!(again, vec![row("Reminder/U3")]);
        done_asking(&["Reminder/U1".to_string()]);
        assert_eq!(
            take_unasked(vec![row("Reminder/U1")]),
            vec![row("Reminder/U1")]
        );
        done_asking(&[
            "Reminder/U1".to_string(),
            "Reminder/U2".to_string(),
            "Reminder/U3".to_string(),
        ]);
    }
}
