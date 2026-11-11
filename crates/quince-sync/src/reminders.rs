//! Reminders: a change made here recorded and shown at once (`record_*`), laid back over every
//! pass from Apple until it is sent (`lay`, `replay`), and sent to Apple behind (`send`); and a
//! pass from Apple stored (`refresh`, `apply`). Moved whole from the desktop app so the phone's
//! engine runs the same code: one reminder, one behaviour, wherever it was changed.

use crate::library_lock::LibraryLock;
use crate::queue::{json, known_gone, last_word, unreadable};
use quince_core::outbox::Pending;
use quince_core::reminders::Reminder;
use quince_core::Library;
use quince_icloud::reminders::ReminderEdit;
use quince_icloud::Error;
use serde::{Deserialize, Serialize};

/// The queue Reminders' changes wait in.
pub const REMINDERS: &str = "reminders";

/// What a waiting change does to a reminder: ticked (`at` the moment) or not, over the copy of
/// Apple's record it was made from (`base`: what tells a change made elsewhere since from one
/// sent from here). A tick queued before there was a base has none, and is sent unless Apple
/// already has it as wanted.
#[derive(Serialize, Deserialize)]
pub struct Done {
    pub done: bool,
    pub at: i64,
    #[serde(default)]
    pub base: quince_icloud::reminders::TickBase,
}

/// A reminder made here, under the name it was given here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Create {
    pub list_guid: String,
    pub title: String,
    pub due_at: Option<i64>,
    pub all_day: Option<bool>,
    pub flagged: Option<bool>,
}

/// `Option<Option<T>>` kept whole through JSON: absent is "leave it", null is "clear it".
mod double {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer, T: Serialize>(
        v: &Option<Option<T>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match v {
            Some(inner) => inner.serialize(s),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> Result<Option<Option<T>>, D::Error> {
        Option::<T>::deserialize(d).map(Some)
    }
}

/// A reminder edit as queued: [`ReminderEdit`], with "clear" and "leave" kept apart, and the
/// list the reminder was in before any of it — what a move is made from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double")]
    pub notes: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double")]
    pub due_at: Option<Option<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_day: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flagged: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_guid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "double")]
    pub parent_guid: Option<Option<String>>,
    pub from_list: String,
}

impl Edit {
    fn new(e: &ReminderEdit, from_list: &str) -> Self {
        Self {
            title: e.title.clone(),
            notes: e.notes.clone(),
            due_at: e.due_at,
            all_day: e.all_day,
            flagged: e.flagged,
            completed: e.completed,
            priority: e.priority,
            list_guid: e.list_guid.clone(),
            parent_guid: e.parent_guid.clone(),
            from_list: from_list.to_string(),
        }
    }

    /// A later edit laid over this one: what it says wins, what it leaves stays.
    fn then(mut self, later: Self) -> Self {
        macro_rules! over {
            ($($f:ident),*) => { $( if later.$f.is_some() { self.$f = later.$f; } )* };
        }
        over!(
            title,
            notes,
            due_at,
            all_day,
            flagged,
            completed,
            priority,
            list_guid,
            parent_guid
        );
        self
    }

    fn to_apple(&self) -> ReminderEdit {
        ReminderEdit {
            title: self.title.clone(),
            notes: self.notes.clone(),
            due_at: self.due_at,
            all_day: self.all_day,
            flagged: self.flagged,
            completed: self.completed,
            priority: self.priority,
            list_guid: self.list_guid.clone(),
            parent_guid: self.parent_guid.clone(),
        }
    }

    /// The edit, made to the stored reminder.
    fn lay_on(&self, r: &mut Reminder) {
        if let Some(t) = &self.title {
            r.title = t.trim().to_string();
        }
        if let Some(n) = &self.notes {
            r.notes.clone_from(n);
        }
        if let Some(due) = self.due_at {
            r.due_at = due;
            r.all_day = self.all_day.unwrap_or(false) && due.is_some();
        }
        if let Some(f) = self.flagged {
            r.flagged = f;
        }
        if let Some(c) = self.completed {
            r.completed_at = if c {
                r.completed_at.or(Some(jiff::Timestamp::now().as_second()))
            } else {
                None
            };
        }
        if let Some(p) = self.priority {
            r.priority = match p {
                1..=3 => 1,
                4..=6 => 5,
                7..=9 => 9,
                _ => 0,
            };
        }
        if let Some(l) = &self.list_guid {
            r.list_guid.clone_from(l);
        }
        if let Some(p) = &self.parent_guid {
            r.parent_guid.clone_from(p);
        }
    }
}

/// A reminder deleted here, with the subtasks that go with it, as they were.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delete {
    pub reminder: Reminder,
    pub children: Vec<Reminder>,
}

/// A repeating reminder ticked here: Apple moves it on to its next date. `from_due` is the
/// date it was due as it was ticked, so the tick moves it on once — never twice — however
/// many times it is sent or laid over a pass.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Advance {
    pub from_due: Option<i64>,
    pub at: i64,
}

// --- Recording: each change recorded and laid over the library as one transaction, so a crash
// leaves both or neither.

/// A reminder made here: named now, shown now, sent behind.
pub fn record_reminder_create(
    library: &mut Library,
    create: &Create,
) -> quince_core::Result<String> {
    let guid = quince_icloud::reminders::new_reminder_name();
    library.atomically(|l| {
        l.outbox_append(REMINDERS, &guid, "create", &json(create)?)?;
        lay_create(l, &guid, create)?;
        Ok(guid.clone())
    })
}

fn lay_create(l: &mut Library, guid: &str, c: &Create) -> quince_core::Result<()> {
    if l.reminder(guid)?.is_some() {
        return Ok(());
    }
    // A list deleted since has nowhere to show it.
    if !l.reminder_lists()?.iter().any(|x| x.guid == c.list_guid) {
        return Ok(());
    }
    l.put_reminder_local(&Reminder {
        guid: guid.to_string(),
        list_guid: c.list_guid.clone(),
        title: c.title.trim().to_string(),
        due_at: c.due_at,
        all_day: c.all_day.unwrap_or(false) && c.due_at.is_some(),
        flagged: c.flagged.unwrap_or(false),
        ..Default::default()
    })
}

/// An edit to a reminder. Edits not yet sent and still the last word on it are one edit.
pub fn record_reminder_edit(
    library: &mut Library,
    reminder: &Reminder,
    edit: &ReminderEdit,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let fresh = Edit::new(edit, &reminder.list_guid);
        let waiting = last_word(l, REMINDERS, &reminder.guid, "edit")?
            .filter(|p| p.tries == 0)
            .and_then(|p| {
                serde_json::from_str::<Edit>(&p.payload)
                    .ok()
                    .map(|e| (p, e))
            });
        let amended = match waiting {
            Some((p, before)) => l.outbox_amend(p.id, &json(&before.then(fresh.clone()))?)?,
            None => false,
        };
        if !amended {
            l.outbox_append(REMINDERS, &reminder.guid, "edit", &json(&fresh)?)?;
        }
        lay_edit(l, &reminder.guid, &fresh)
    })
}

fn lay_edit(l: &mut Library, guid: &str, e: &Edit) -> quince_core::Result<()> {
    let Some(mut r) = l.reminder(guid)? else {
        return Ok(());
    };
    e.lay_on(&mut r);
    l.put_reminder_local(&r)?;
    // Moved, its subtasks go with it, as on the phone.
    if let Some(to) = &e.list_guid {
        for mut child in l.subtasks_of(guid)? {
            child.list_guid.clone_from(to);
            l.put_reminder_local(&child)?;
        }
    }
    Ok(())
}

/// A reminder deleted here, with its subtasks.
pub fn record_reminder_delete(
    library: &mut Library,
    reminder: &Reminder,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let children = l.subtasks_of(&reminder.guid)?;
        let delete = Delete {
            reminder: reminder.clone(),
            children,
        };
        l.outbox_append(REMINDERS, &reminder.guid, "delete", &json(&delete)?)?;
        lay_delete(l, &delete)
    })
}

fn lay_delete(l: &mut Library, d: &Delete) -> quince_core::Result<()> {
    let mut gone: Vec<String> = d.children.iter().map(|c| c.guid.clone()).collect();
    gone.push(d.reminder.guid.clone());
    l.forget_reminders(&gone, &[])
}

/// A repeating reminder ticked: moved on to its next date here at once, and queued for Apple,
/// which keeps the history and moves its own copy on. Each tick is its own change — two ticks
/// are two days — and each moves it on from the date it had when it was made, once.
pub fn record_reminder_advance(
    library: &mut Library,
    reminder: &Reminder,
    at: i64,
) -> quince_core::Result<()> {
    library.atomically(|l| {
        let advance = Advance {
            from_due: reminder.due_at,
            at,
        };
        l.outbox_append(REMINDERS, &reminder.guid, "advance", &json(&advance)?)?;
        lay_advance(l, &reminder.guid, &advance)
    })
}

fn lay_advance(l: &mut Library, guid: &str, a: &Advance) -> quince_core::Result<()> {
    let Some(mut r) = l.reminder(guid)? else {
        return Ok(());
    };
    // Moved on already — by this same tick laid before, or by Apple's own answer — stays.
    if r.due_at != a.from_due {
        return Ok(());
    }
    // Past its End Repeat date there is no next one: the series is done, as on the phone.
    // Weekdays, Weekends or chosen days: on to the next of those, not a week on.
    let days = l.reminder_repeat_days(guid)?;
    if r.tick_ends_series_on(l.reminder_repeat_end(guid)?, &days, a.at) {
        r.completed_at = Some(a.at);
        return l.put_reminder_local(&r);
    }
    if let Some(next) = r.next_occurrence_on(&days, a.at) {
        r.due_at = Some(next);
    }
    r.completed_at = None;
    l.put_reminder_local(&r)
}

/// A reminder ticked or unticked here at `at`. A repeating one ticked moves on to its next date
/// ([`record_reminder_advance`]), unless this is the last of its series, which completes it, as
/// the phone completes it rather than moving it past its End Repeat date.
pub fn record_reminder_tick(
    library: &mut Library,
    reminder: &Reminder,
    done: bool,
    at: i64,
) -> quince_core::Result<()> {
    let end = library.reminder_repeat_end(&reminder.guid)?;
    let days = library.reminder_repeat_days(&reminder.guid)?;
    let last = done && reminder.tick_ends_series_on(end, &days, at);
    if !last && done && reminder.repeats() {
        return record_reminder_advance(library, reminder, at);
    }
    let payload = json(&Done {
        done,
        at,
        base: quince_icloud::reminders::TickBase::of(reminder),
    })?;
    // Shown and queued together, or neither: a tick shown with nothing queued would be put
    // back by the next pass.
    library.atomically(|l| {
        l.set_reminder_completed(&reminder.guid, done.then_some(at))?;
        l.outbox_push(REMINDERS, &reminder.guid, "done", &payload)?;
        Ok(())
    })
}

/// One waiting Reminders change laid over the library.
pub fn lay(l: &mut Library, p: &Pending) -> quince_core::Result<()> {
    match p.kind.as_str() {
        "done" => {
            if let Ok(d) = serde_json::from_str::<Done>(&p.payload) {
                l.set_reminder_completed(&p.record, d.done.then_some(d.at))?;
            }
        }
        "create" => {
            if let Ok(c) = serde_json::from_str::<Create>(&p.payload) {
                lay_create(l, &p.record, &c)?;
            }
        }
        "edit" => {
            if let Ok(e) = serde_json::from_str::<Edit>(&p.payload) {
                lay_edit(l, &p.record, &e)?;
            }
        }
        "delete" => {
            if let Ok(d) = serde_json::from_str::<Delete>(&p.payload) {
                lay_delete(l, &d)?;
            }
        }
        "advance" => {
            if let Ok(a) = serde_json::from_str::<Advance>(&p.payload) {
                lay_advance(l, &p.record, &a)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The reminder changes still waiting, laid over what a pass stored, in the order made.
pub fn replay(library: &mut Library) -> quince_core::Result<()> {
    for p in library.outbox_pending(REMINDERS)? {
        lay(library, &p)?;
    }
    Ok(())
}

// --- From Apple.

/// A pass from Apple stored: its lists and reminders, then every change still on its way laid
/// back over them — without this a reminder ticked offline showed open again after every pass,
/// and its alert could ring for something already done. `full` (a pass from the start, with no
/// token) marks the library as having had one. Answers the lists and reminders it held.
pub fn apply(
    library: &mut Library,
    fetched: quince_icloud::reminders::RemindersFetch,
    full: bool,
) -> Result<(usize, usize), Error> {
    // One transaction, Apple's pass and the edits laid back over it, as the desktop stores it:
    // killed between the two (Android stops apps), unsent edits showed reverted until the next.
    library.atomically(|library| {
        let (lists, count, _) = quince_icloud::reminders::apply_cloudkit(library, fetched)?;
        replay(library)?;
        if full {
            library.mark_full_sync(REMINDERS)?;
        }
        Ok((lists, count))
    })
}

// --- Sending: the queue's half.

pub async fn send(
    library: &LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    p: Pending,
) -> Result<(), Error> {
    use quince_icloud::reminders as r;
    let stored = library.lock().await.reminder(&p.record)?;
    match p.kind.as_str() {
        "done" => {
            let d: Done = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            // Gone from the library (deleted here or elsewhere): nothing left to tick.
            let Some(reminder) = stored else {
                return known_gone(library, REMINDERS).await;
            };
            // A change made on another device after this one was queued wins; the pass after
            // sending reads Apple's version back.
            if r::tick_overtaken(http, root, params.clone(), &reminder, d.done, &d.base).await? {
                return Ok(());
            }
            r::set_completed_cloudkit(http, root, params, &reminder, d.done).await
        }
        "create" => {
            let c: Create = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            r::create_cloudkit_named(
                http,
                root,
                params.clone(),
                &p.record,
                &c.list_guid,
                &c.title,
            )
            .await?;
            if c.due_at.is_some() || c.flagged.is_some() {
                let made = Reminder {
                    guid: p.record.clone(),
                    list_guid: c.list_guid.clone(),
                    title: c.title.clone(),
                    ..Default::default()
                };
                let edit = ReminderEdit {
                    due_at: c.due_at.map(Some),
                    all_day: c.all_day,
                    flagged: c.flagged,
                    ..Default::default()
                };
                // The reminder exists at Apple now: its date or flag failing is noted, never
                // a reason to make it again.
                if let Err(e) = r::edit_cloudkit(http, root, params, &made, &edit).await {
                    crate::quiet!("a new reminder's date or flag was not set: {e}");
                }
            }
            Ok(())
        }
        "advance" => {
            let a: Advance = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            // Gone from the library (deleted here or elsewhere): nothing left to tick.
            let Some(reminder) = stored else {
                return known_gone(library, REMINDERS).await;
            };
            // Moved on at Apple already — by an earlier send of this tick, or on another
            // device since — it is not moved on again.
            r::advance_cloudkit_once(http, root, params, &reminder, a.from_due).await
        }
        "edit" => {
            let e: Edit = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            let Some(mut reminder) = stored else {
                known_gone(library, REMINDERS).await?;
                return Err(Error::Refused(
                    "This reminder was deleted on another device, so the change wasn’t saved."
                        .into(),
                ));
            };
            // Sent as a move from where it was before any of this edit.
            reminder.list_guid.clone_from(&e.from_list);
            let edit = e.to_apple();
            r::edit_cloudkit(http, root, params.clone(), &reminder, &edit).await?;
            if let Some(to) = e.list_guid.as_deref().filter(|to| *to != e.from_list) {
                let children = library.lock().await.subtasks_of(&p.record)?;
                let follow = ReminderEdit {
                    list_guid: Some(to.to_string()),
                    ..Default::default()
                };
                for mut child in children {
                    child.list_guid.clone_from(&e.from_list);
                    if let Err(e) =
                        r::edit_cloudkit(http, root, params.clone(), &child, &follow).await
                    {
                        crate::quiet!("a subtask did not follow its reminder: {e}");
                    }
                }
            }
            // A date carries its alert, as on the phone; made all-day, it rings none.
            let alarm_change = if e.all_day == Some(true) {
                Some(None)
            } else {
                e.due_at
            };
            if let Some(due) = alarm_change {
                let zone = jiff::tz::TimeZone::system();
                r::set_alarm_cloudkit(
                    http,
                    root,
                    params,
                    &p.record,
                    due,
                    zone.iana_name().unwrap_or("UTC"),
                )
                .await?;
            }
            Ok(())
        }
        "delete" => {
            let d: Delete = serde_json::from_str(&p.payload).map_err(|e| unreadable(&e))?;
            match r::delete_cloudkit(http, root, params.clone(), &d.reminder).await {
                Ok(()) | Err(Error::NotFound) => {}
                Err(e) => return Err(e),
            }
            for child in &d.children {
                if let Err(e) = r::delete_cloudkit(http, root, params.clone(), child).await {
                    crate::quiet!("a subtask did not go with its reminder: {e}");
                }
            }
            Ok(())
        }
        other => Err(Error::Refused(format!(
            "Skykeep does not know how to send “{other}”."
        ))),
    }
}
