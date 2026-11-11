//! Reminders on the phone, as on the desktop: read from the phone's own library (instant, and
//! there offline), every change recorded and shown at once and sent behind through the same
//! queue, and each pass from Apple laid under what is still on its way. All of it is
//! `quince_sync::reminders`, the desktop's own code; this file only opens the library and says
//! it in the phone's terms.

use crate::{failed, runtime, Account, EngineError};
use quince_core::reminders::{Reminder, SmartList};
use quince_sync::library_lock::LibraryLock;
use quince_sync::reminders as shared;
use std::sync::Arc;
use tokio::sync::Mutex;

/// One pass at a time over Reminders, sending or reading: a pass read just before a send and
/// stored just after would put the old state back with nothing left to lay over it.
static PASS: Mutex<()> = Mutex::const_new(());

/// A reminder list, with how many are still open in it.
#[derive(uniffi::Record)]
pub struct RList {
    pub guid: String,
    pub title: String,
    /// `#rrggbb`, or none for Apple's default.
    pub colour: Option<String>,
    pub open: u32,
}

/// One reminder.
#[derive(uniffi::Record)]
pub struct RItem {
    pub guid: String,
    pub list_guid: String,
    pub title: String,
    pub notes: Option<String>,
    /// Unix seconds; an all-day one's is its day's midnight on the phone.
    pub due_at: Option<i64>,
    pub all_day: bool,
    pub flagged: bool,
    /// Apple's: 0 none, 1 high, 5 medium, 9 low.
    pub priority: i64,
    pub completed_at: Option<i64>,
    pub parent_guid: Option<String>,
    pub repeats: bool,
}

/// Which reminders: a list's, or one of Reminders' own lists.
#[derive(uniffi::Enum)]
pub enum RView {
    List { guid: String, completed: bool },
    Today,
    Scheduled,
    Flagged,
    All,
    Completed,
}

/// How many are in each of Reminders' own lists.
#[derive(uniffi::Record)]
pub struct RCounts {
    pub today: u32,
    pub scheduled: u32,
    pub flagged: u32,
    pub all: u32,
    pub completed: u32,
}

/// An edit: each field given is changed, each left out stays. `clear_notes` and `clear_due`
/// take the notes or the date away.
#[derive(uniffi::Record)]
pub struct REdit {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub clear_notes: bool,
    pub due_at: Option<i64>,
    pub all_day: Option<bool>,
    pub clear_due: bool,
    pub flagged: Option<bool>,
    pub priority: Option<i64>,
    pub list_guid: Option<String>,
}

/// What a sync of Reminders came to.
#[derive(uniffi::Record)]
pub struct RSync {
    pub lists: u32,
    /// Open reminders, in all the lists.
    pub reminders: u32,
    /// Changes made here still waiting for Apple (it couldn't be reached).
    pub waiting: u32,
    /// Changes Apple refused, each with its reason, for the person.
    pub refused: Vec<String>,
    /// Advanced Data Protection holds Reminders until the person approves on one of their
    /// devices: nothing was asked of Apple, and nothing is retried before then.
    pub paused: bool,
}

fn item(r: Reminder) -> RItem {
    let repeats = r.repeats();
    RItem {
        guid: r.guid,
        list_guid: r.list_guid,
        title: r.title,
        notes: r.notes,
        due_at: r.due_at,
        all_day: r.all_day,
        flagged: r.flagged,
        priority: r.priority,
        completed_at: r.completed_at,
        parent_guid: r.parent_guid,
        repeats,
    }
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// The end of today on the phone's clock: where "Today" stops, as Reminders counts it.
fn end_of_today() -> i64 {
    let zone = jiff::tz::TimeZone::system();
    jiff::Timestamp::now()
        .to_zoned(zone)
        .date()
        .tomorrow()
        .ok()
        .and_then(|d| d.to_zoned(jiff::tz::TimeZone::system()).ok())
        .map_or_else(now, |z| z.timestamp().as_second())
}

impl Account {
    /// The signed-in account's library: its own folder, so two accounts' reminders never mix.
    /// Opened once, and again only for another account.
    pub(crate) fn library(&self) -> Result<(Arc<LibraryLock>, String), EngineError> {
        let account = {
            let c = self.client();
            quince_sync::queue::asked_as(&c.service_params())
                .ok_or_else(|| failed("Sign in to iCloud first."))?
        };
        Ok((self.library_for(&account)?, account))
    }

    /// `account`'s library (as the library names accounts), opened once and again only for
    /// another account. A sync passes the account its session asks as, read in the same look
    /// as the rest of what the pass needs (`pass::Apple`), never in a second one.
    pub(crate) fn library_for(&self, account: &str) -> Result<Arc<LibraryLock>, EngineError> {
        let account = account.to_string();
        let mut open = self
            .library
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((who, lock)) = open.as_ref() {
            if *who == account {
                return Ok(Arc::clone(lock));
            }
        }
        let dir = self.dir.join("libraries").join(&account);
        std::fs::create_dir_all(&dir).map_err(failed)?;
        let library = quince_core::Library::open(&dir).map_err(failed)?;
        let lock = Arc::new(LibraryLock::new(library, Some(dir)));
        runtime().block_on(async {
            let mut l = lock.lock().await;
            if l.state(quince_sync::queue::ACCOUNT_KEY)
                .map_err(failed)?
                .is_none()
            {
                l.set_state(quince_sync::queue::ACCOUNT_KEY, &account)
                    .map_err(failed)?;
            }
            Ok::<_, EngineError>(())
        })?;
        *open = Some((account, Arc::clone(&lock)));
        Ok(lock)
    }

    fn with_library<T>(
        &self,
        f: impl FnOnce(&mut quince_core::Library) -> quince_core::Result<T>,
    ) -> Result<T, EngineError> {
        let (lock, _) = self.library()?;
        runtime().block_on(async {
            let mut l = lock.lock().await;
            f(&mut l).map_err(failed)
        })
    }

    fn stored(l: &quince_core::Library, guid: &str) -> quince_core::Result<Reminder> {
        l.reminder(guid)?.ok_or_else(|| {
            quince_core::Error::Io(std::io::Error::other(
                "This reminder isn't here any more: it was deleted, here or on another device.",
            ))
        })
    }
}

#[uniffi::export]
impl Account {
    /// Changes made on the phone not yet at Apple, said before signing out so nothing unsent goes
    /// without the person knowing: every one waiting or refused in the engine's queue, except
    /// Contacts' and Calendar's still waiting — their rows are still marked changed in Android's
    /// providers, and the app counts those itself (counted here too, each would be said twice).
    pub fn unsent_changes(&self) -> Result<u32, EngineError> {
        use quince_sync::{calendar::CALENDAR, contacts::CONTACTS};
        self.with_library(|l| {
            let in_rows = l.outbox_pending(CONTACTS)?.len() + l.outbox_pending(CALENDAR)?.len();
            Ok(l.outbox_count()?.saturating_sub(in_rows))
        })
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// The reminder lists, in Apple's order, with how many are open in each.
    pub fn reminder_lists(&self) -> Result<Vec<RList>, EngineError> {
        self.with_library(|l| {
            let open = l.reminder_counts(end_of_today())?.lists;
            Ok(l.reminder_lists()?
                .into_iter()
                .filter(|list| list.enabled)
                .map(|list| RList {
                    open: count(
                        open.iter()
                            .find(|(g, _)| *g == list.guid)
                            .map_or(0, |(_, n)| *n),
                    ),
                    guid: list.guid,
                    title: list.title,
                    colour: list.colour,
                })
                .collect())
        })
    }

    /// How many are in Today, Scheduled, Flagged, All and Completed.
    pub fn reminder_counts(&self) -> Result<RCounts, EngineError> {
        self.with_library(|l| {
            let c = l.reminder_counts(end_of_today())?;
            Ok(RCounts {
                today: count(c.today),
                scheduled: count(c.scheduled),
                flagged: count(c.flagged),
                all: count(c.all),
                completed: count(c.completed),
            })
        })
    }

    /// The reminders a list or one of Reminders' own lists shows.
    pub fn reminders(&self, view: RView) -> Result<Vec<RItem>, EngineError> {
        self.with_library(|l| {
            let found = match view {
                RView::List { guid, completed } => l.reminders_in(&guid, completed)?,
                RView::Today => l.reminders_smart(SmartList::Today, end_of_today())?,
                RView::Scheduled => l.reminders_smart(SmartList::Scheduled, end_of_today())?,
                RView::Flagged => l.reminders_smart(SmartList::Flagged, end_of_today())?,
                RView::All => l.reminders_smart(SmartList::All, end_of_today())?,
                RView::Completed => l.reminders_smart(SmartList::Completed, end_of_today())?,
            };
            Ok(found.into_iter().map(item).collect())
        })
    }

    /// Open reminders due from `from` to `to` (Unix seconds): what the phone sets alerts for.
    pub fn reminders_due_between(&self, from: i64, to: i64) -> Result<Vec<RItem>, EngineError> {
        self.with_library(|l| {
            Ok(l.reminders_due_between(from, to)?
                .into_iter()
                .map(item)
                .collect())
        })
    }

    /// A reminder made here: in the list at once, sent behind ([`Self::reminders_sync`]).
    pub fn reminder_create(
        &self,
        list_guid: String,
        title: String,
        due_at: Option<i64>,
        all_day: bool,
    ) -> Result<String, EngineError> {
        self.with_library(|l| {
            shared::record_reminder_create(
                l,
                &shared::Create {
                    list_guid,
                    title,
                    due_at,
                    all_day: due_at.map(|_| all_day),
                    flagged: None,
                },
            )
        })
    }

    /// A reminder changed here: shown at once, sent behind.
    pub fn reminder_edit(&self, guid: String, edit: REdit) -> Result<(), EngineError> {
        let apple = quince_icloud::reminders::ReminderEdit {
            title: edit.title,
            notes: if edit.clear_notes {
                Some(None)
            } else {
                edit.notes.map(Some)
            },
            due_at: if edit.clear_due {
                Some(None)
            } else {
                edit.due_at.map(Some)
            },
            all_day: edit.all_day,
            flagged: edit.flagged,
            priority: edit.priority,
            list_guid: edit.list_guid,
            ..Default::default()
        };
        self.with_library(|l| {
            let r = Self::stored(l, &guid)?;
            shared::record_reminder_edit(l, &r, &apple)
        })
    }

    /// Ticked or unticked here. A repeating one ticked moves on to its next date.
    pub fn reminder_tick(&self, guid: String, done: bool) -> Result<(), EngineError> {
        self.with_library(|l| {
            let r = Self::stored(l, &guid)?;
            shared::record_reminder_tick(l, &r, done, now())
        })
    }

    /// Deleted here, with its subtasks.
    pub fn reminder_delete(&self, guid: String) -> Result<(), EngineError> {
        self.with_library(|l| {
            let r = Self::stored(l, &guid)?;
            shared::record_reminder_delete(l, &r)
        })
    }

    /// A new list, called `title`, made at Apple and read back, as the desktop makes one.
    pub fn reminder_list_create(&self, title: String) -> Result<(), EngineError> {
        let (http, root, params) = self.reminders_context()?;
        runtime()
            .block_on(quince_icloud::reminders::create_list_cloudkit(
                &http, &root, params, &title,
            ))
            .map_err(failed)?;
        self.reminders_reread()
    }

    /// A list renamed at Apple and read back.
    pub fn reminder_list_rename(&self, guid: String, title: String) -> Result<(), EngineError> {
        let (http, root, params) = self.reminders_context()?;
        runtime()
            .block_on(quince_icloud::reminders::rename_list_cloudkit(
                &http, &root, params, &guid, &title,
            ))
            .map_err(failed)?;
        self.reminders_reread()
    }

    /// A list's colour, by Apple's name for it (red, orange, yellow, green, lightBlue, blue,
    /// indigo, pink, purple, brown, gray, rose), set at Apple and read back.
    pub fn reminder_list_colour(&self, guid: String, colour: String) -> Result<(), EngineError> {
        let (http, root, params) = self.reminders_context()?;
        runtime()
            .block_on(quince_icloud::reminders::set_list_colour_cloudkit(
                &http, &root, params, &guid, &colour,
            ))
            .map_err(failed)?;
        self.reminders_reread()
    }

    /// A list deleted at Apple, with its reminders, and read back.
    pub fn reminder_list_delete(&self, guid: String) -> Result<(), EngineError> {
        let (http, root, params) = self.reminders_context()?;
        runtime()
            .block_on(quince_icloud::reminders::delete_list_cloudkit(
                &http, &root, params, &guid,
            ))
            .map_err(failed)?;
        // Gone here at once: Apple's change feed can lag the delete it just took, and a list
        // left on screen until the next sync looked like the delete hadn't worked.
        self.with_library(|l| l.forget_reminder_list(&guid))?;
        self.reminders_reread()
    }

    /// The changes made here sent to Apple, oldest first, then Apple's Reminders read and laid
    /// under whatever is still on its way, as the desktop does it.
    pub fn reminders_sync(&self) -> Result<RSync, EngineError> {
        // The session and whose library it is, read in one look: an account switched between
        // two looks once stored one account's reminders in the other's library.
        let apple = self.apple("reminders")?;
        let lock = self.library_of(&apple)?;
        runtime().block_on(async {
            let waits = apple.waiting.contains(&shared::REMINDERS);
            // Held by Advanced Data Protection: nothing is sent (Apple would only refuse it, and
            // the queue would try again and again); it waits, kept, for the approval.
            let report = match apple.cloudkit.as_deref() {
                Some(root) if !waits => {
                    quince_sync::queue::drain_as(
                        &lock,
                        shared::REMINDERS,
                        &PASS,
                        &apple.account,
                        |p| shared::send(&lock, &apple.http, root, apple.params.clone(), p),
                    )
                    .await
                }
                _ => quince_sync::queue::Report {
                    outcome: quince_sync::queue::Outcome::Later,
                    sent: false,
                    refused: Vec::new(),
                },
            };
            // Whose library, a whole walk once a day, a panic kept to it: the desktop's pass.
            let pass = self
                .pass(quince_sync::pass::REMINDERS, &apple, &lock, &PASS)
                .await;
            let paused =
                crate::pass::settled(&pass, "Apple hasn't offered Reminders for this account.")?;
            // What the library holds now, not what this pass changed: a pass from a token
            // brings only what changed ("1 reminders in 1 lists", it once said).
            let l = lock.lock().await;
            let lists = l
                .reminder_lists()
                .map_err(failed)?
                .into_iter()
                .filter(|x| x.enabled)
                .count();
            let open = l.reminder_counts(end_of_today()).map_err(failed)?.all;
            let waiting = l.outbox_pending(shared::REMINDERS).map_err(failed)?.len();
            Ok(RSync {
                lists: count(lists),
                reminders: count(open),
                waiting: count(waiting),
                refused: report.refused.into_iter().map(|(_, why)| why).collect(),
                paused,
            })
        })
    }
}

/// What a call to Apple's Reminders needs: the client, CloudKit's address, the session's parameters.
type Apple = (reqwest::Client, String, Vec<(String, String)>);

impl Account {
    fn reminders_context(&self) -> Result<Apple, EngineError> {
        let c = self.client();
        let root = c
            .session()
            .service_url("ckdatabasews")
            .ok_or_else(|| failed("Apple hasn't offered Reminders for this account."))?;
        Ok((c.http().clone(), root, c.service_params()))
    }

    /// Apple's Reminders read again after a list change that reached Apple. Failing, it is not
    /// the change failing (it is made): the next sync brings the screen up to date.
    fn reminders_reread(&self) -> Result<(), EngineError> {
        let apple = self.apple("reminders")?;
        let lock = self.library_of(&apple)?;
        let _ = runtime().block_on(self.pass(quince_sync::pass::REMINDERS, &apple, &lock, &PASS));
        Ok(())
    }
}
