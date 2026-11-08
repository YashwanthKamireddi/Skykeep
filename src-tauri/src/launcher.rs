//! What the Skykeep window's launcher says under each app: counted from the library on this
//! computer, one read for all six, never a request to Apple (docs/apps-as-windows.md).

use crate::{Data, UiResult};
use quince_core::photos::View;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    notes: i64,
    reminders_today: usize,
    events_today: usize,
    next_event: Option<NextEvent>,
    contacts: i64,
    photos: i64,
}

/// Today's next event: the first still to begin, or — with none left — none.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NextEvent {
    title: String,
    starts_at: i64,
    all_day: bool,
}

#[tauri::command]
pub async fn launcher_summary(data: State<'_, Data>) -> UiResult<Summary> {
    let now = jiff::Zoned::now();
    let today = now.start_of_day().map_err(|e| e.to_string())?;
    let (start, end) = (
        today.timestamp().as_second(),
        today
            .tomorrow()
            .map_err(|e| e.to_string())?
            .timestamp()
            .as_second(),
    );
    let at = now.timestamp().as_second();
    data.0
        .read(|library| {
            let events = library.events_between(start, end)?;
            let next_event = events
                .iter()
                .filter(|e| !e.all_day && e.starts_at >= at)
                .min_by_key(|e| e.starts_at)
                .map(|e| NextEvent {
                    title: e.title.clone(),
                    starts_at: e.starts_at,
                    all_day: false,
                });
            Ok(Summary {
                notes: library.note_counts()?.0,
                reminders_today: library.reminder_counts(end)?.today,
                events_today: events.len(),
                next_event,
                contacts: library.contact_count()?,
                photos: library
                    .photo_counts()?
                    .into_iter()
                    .find(|(v, _)| *v == View::Library)
                    .map_or(0, |(_, n)| n),
            })
        })
        .await
        .map_err(|e| e.to_string())
}
