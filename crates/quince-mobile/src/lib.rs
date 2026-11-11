//! Skykeep's engine for Android, as the Kotlin app sees it.
//!
//! Coarse calls only: each crosses the bridge once and does its work here, in Rust, so the
//! app never drives the engine a step at a time. The engine's own crates stay as they are on
//! desktop; this crate only adapts them to what Android gives (its Keystore, through
//! [`SecretStore`]) and what it can call (UniFFI's generated Kotlin).

use std::sync::OnceLock;

uniffi::setup_scaffolding!();

/// Why a call failed, in words the app can show.
#[derive(Debug, thiserror::Error, uniffi::Error)]
/// (`reason`, not `message`: Kotlin exceptions already have a `message`.)
pub enum EngineError {
    #[error("{reason}")]
    Failed { reason: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for EngineError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        EngineError::Failed { reason: e.reason }
    }
}

/// The app's secret store: Android's Keystore, kept by the Kotlin side, where the session's
/// keys live as they live in the keyring on desktop.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    fn get(&self, service: String, name: String) -> Result<Option<String>, EngineError>;
    fn set(&self, service: String, name: String, value: String) -> Result<(), EngineError>;
    fn delete(&self, service: String, name: String) -> Result<(), EngineError>;
}

/// The Kotlin store, seen by the engine as its platform's secrets.
struct Bridged(std::sync::Arc<dyn SecretStore>);

impl quince_auth::PlatformSecrets for Bridged {
    fn get(&self, service: &str, name: &str) -> Result<Option<String>, String> {
        self.0
            .get(service.into(), name.into())
            .map_err(|e| e.to_string())
    }
    fn set(&self, service: &str, name: &str, value: &str) -> Result<(), String> {
        self.0
            .set(service.into(), name.into(), value.into())
            .map_err(|e| e.to_string())
    }
    fn delete(&self, service: &str, name: &str) -> Result<(), String> {
        self.0
            .delete(service.into(), name.into())
            .map_err(|e| e.to_string())
    }
}

/// One runtime for the life of the process: Android starts and stops activities freely, and
/// the engine's work must not start and stop with them.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("skykeep-engine")
            .enable_all()
            .build()
            .expect("the engine's runtime")
    })
}

/// The engine's version, the same as the desktop app's.
/// Where the app goes when it opens.
#[derive(uniffi::Enum)]
pub enum Reopened {
    Library,
    Code,
    SignIn,
}

#[uniffi::export]
pub fn engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Hands the engine Android's secret store. Once per process, before anything signs in.
#[uniffi::export]
pub fn start(secrets: std::sync::Arc<dyn SecretStore>) -> Result<(), EngineError> {
    quince_auth::use_platform_secrets(Box::new(Bridged(secrets)))
        .map_err(|reason| EngineError::Failed { reason })
}

/// Whether Apple's sign-in service can be reached from this phone, through the engine's own
/// Apple-only client: the network, DNS and TLS checked before a sign-in is offered.
#[uniffi::export]
pub fn apple_reachable() -> Result<bool, EngineError> {
    runtime().block_on(async {
        let http = quince_auth::apple_only()
            .build()
            .map_err(|e| EngineError::Failed {
                reason: e.to_string(),
            })?;
        Ok(http
            .get("https://setup.icloud.com/setup/ws/1/validate")
            .send()
            .await
            .is_ok())
    })
}

/// What signing in decided.
#[derive(uniffi::Enum)]
pub enum SignIn {
    /// Signed in: Apple already trusted this phone.
    Complete,
    /// Apple wants a six-digit code (see [`Account::code_options`]).
    NeedsCode,
}

/// A phone number Apple will send a code to, already masked by Apple.
#[derive(uniffi::Record)]
pub struct TrustedPhone {
    pub id: u64,
    pub masked_number: String,
}

/// Where Apple is willing to send a code.
#[derive(uniffi::Record)]
pub struct CodeOptions {
    /// Devices Apple pushed the code to; zero means nothing arrives on its own.
    pub trusted_devices: u64,
    pub phones: Vec<TrustedPhone>,
    /// Apple refuses more codes for a while.
    pub too_many_codes_sent: bool,
    /// The account signs in with security keys, which Skykeep cannot answer yet.
    pub security_key: bool,
}

/// A way to reach someone: `phone`, `email`, `url` or `address`, with Apple's label.
#[derive(uniffi::Record)]
pub struct Method {
    pub kind: String,
    pub label: Option<String>,
    pub value: String,
}

/// A person in the iCloud address book, as Android's Contacts will hold them.
#[derive(uniffi::Record)]
pub struct Person {
    /// Apple's id for the card: the raw contact's source id, so a sync finds it again.
    pub guid: String,
    /// Changes whenever the card does: an unchanged card is not rewritten.
    pub etag: Option<String>,
    pub first_name: Option<String>,
    pub middle_name: Option<String>,
    pub last_name: Option<String>,
    pub company: Option<String>,
    pub is_company: bool,
    pub birthday: Option<String>,
    pub notes: Option<String>,
    pub methods: Vec<Method>,
    /// The rest of the name and work, as iCloud keeps them (the engine's `ContactDetails`).
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub nickname: Option<String>,
    pub phonetic_first_name: Option<String>,
    pub phonetic_last_name: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    /// The iCloud lists (Android's labels) the card is in, by the lists' ids.
    pub groups: Vec<String>,
    /// The card's picture as Apple holds it, if it has one; fetched with
    /// [`Account::contact_photo`] only when its `key` is not the one the phone has.
    pub photo: Option<PhotoRef>,
}

/// An iCloud contacts list: a label in Android's Contacts.
#[derive(uniffi::Record)]
pub struct CList {
    pub guid: String,
    pub name: String,
    pub etag: Option<String>,
}

/// The address book: the people, and the lists.
#[derive(uniffi::Record)]
pub struct Book {
    pub people: Vec<Person>,
    pub lists: Vec<CList>,
}

/// A change to a list made on the phone. Members are changed as added and removed, laid over
/// the list as iCloud has it when sent, so members added on another device meanwhile stay.
#[derive(uniffi::Enum)]
pub enum ListChange {
    Create {
        name: String,
        members: Vec<String>,
    },
    Rename {
        guid: String,
        name: String,
    },
    Delete {
        guid: String,
    },
    Members {
        guid: String,
        add: Vec<String>,
        remove: Vec<String>,
    },
}

/// How one list change went: in iCloud now (`guid`, the list's), gone from iCloud already
/// (deleted on another device), or not sent (`later`: try again on the next sync).
#[derive(uniffi::Enum)]
pub enum ListOutcome {
    Saved { guid: String },
    Gone,
    Failed { reason: String, later: bool },
}

/// Where a card's picture is and which picture it is.
#[derive(uniffi::Record, Clone)]
pub struct PhotoRef {
    pub url: String,
    /// The square its owner chose, `x,y,width,height`, as Apple keeps it.
    pub crop: Option<String>,
    /// Changes when the picture does (Apple's signature, or the url where Apple gives none).
    pub key: String,
}

/// A picture changed on the phone, sent with the card.
#[derive(uniffi::Enum)]
pub enum PhotoEdit {
    /// The picture the phone's Contacts now holds for the card, as an image file.
    Set {
        image: Vec<u8>,
    },
    Removed,
}

/// What changed on the phone to one contact, to be sent to iCloud.
#[derive(uniffi::Enum)]
pub enum ChangeKind {
    /// Made on the phone: created in iCloud under `guid`, chosen on the phone beforehand so a
    /// send repeated after a lost answer finds it made rather than making a second.
    Create,
    /// Edited on the phone: only what changed from `base` written over iCloud's current card.
    Update,
    /// Deleted on the phone.
    Delete,
}

/// One change, as the phone's Contacts holds it now and as it was last synced (`base`).
#[derive(uniffi::Record)]
pub struct ContactChange {
    pub kind: ChangeKind,
    pub guid: String,
    pub person: Option<Person>,
    pub base: Option<Person>,
    /// The card's picture changed on the phone; `None`: left as Apple has it.
    pub photo: Option<PhotoEdit>,
}

/// How one change went.
#[derive(uniffi::Enum)]
pub enum ChangeOutcome {
    /// In iCloud now (or already as asked: made, or gone).
    Saved,
    /// Edited here but deleted in iCloud meanwhile: kept, as a new card under `guid`, rather
    /// than the edit lost.
    Recreated { guid: String },
    /// Not sent: `later` true when it can go as it is on a later sync (Apple busy or out of
    /// reach, the card changed meanwhile); false when Apple refused it, with why.
    Failed { reason: String, later: bool },
}

/// One change made on the phone to an event (or to one occurrence of a repeating event, whose id
/// is `<series>__<start>`), to be sent to iCloud.
#[derive(uniffi::Record)]
pub struct EventChange {
    pub kind: ChangeKind,
    pub guid: String,
    pub event: Option<Ev>,
    pub base: Option<Ev>,
    /// A new event's repeat, when the phone gave it one iCloud can keep.
    pub repeat: Option<Repeat>,
}

/// How a new event repeats, in iCloud's terms (the engine's `EventRepeat`): `daily`, `weekly`,
/// `monthly` or `yearly`, every `interval`; on `week_days`, the `position` weekday of the month,
/// `month_days` or `months`; `count` times, until the day `until`, or for ever. Made from the
/// phone's own rule by [`read_repeat`], never by hand.
#[derive(uniffi::Record)]
pub struct Repeat {
    pub freq: String,
    pub interval: i64,
    pub count: Option<i64>,
    pub week_days: Vec<String>,
    pub position: Option<String>,
    pub month_days: Vec<i64>,
    pub months: Vec<i64>,
    pub until: Option<String>,
}

/// A phone's repeat rule, read as iCloud will hold it.
#[derive(uniffi::Enum)]
pub enum RepeatRead {
    /// It ends before it would happen a second time: a one-off.
    Once,
    Repeats {
        repeat: Repeat,
    },
    /// iCloud's repeat can't hold it; the sentence says why, for the person.
    Refused {
        reason: String,
    },
}

/// The phone's repeat rule (its `RRULE`, such as `FREQ=MONTHLY;BYDAY=2TU`) for an event
/// starting at `starts_at` (Unix seconds; an all-day event's midnight on `phone_zone`), on its
/// own clock `timezone` or the phone's: read by the same engine code that reads an .ics file or
/// a CalDAV calendar, so a rule means one thing everywhere.
#[uniffi::export]
pub fn read_repeat(
    rule: String,
    starts_at: i64,
    all_day: bool,
    timezone: Option<String>,
    phone_zone: String,
) -> RepeatRead {
    let zone = |name: &str| jiff::tz::TimeZone::get(name).ok();
    let here = zone(&phone_zone).unwrap_or(jiff::tz::TimeZone::UTC);
    let own = if all_day {
        None
    } else {
        timezone.as_deref().and_then(zone)
    };
    let clock = own.unwrap_or(here);
    let Ok(at) = jiff::Timestamp::from_second(starts_at) else {
        return RepeatRead::Refused {
            reason: "Its start can’t be read.".into(),
        };
    };
    let first = at.to_zoned(clock.clone()).datetime();
    match quince_icloud::rrule::from_rrule(&rule, first, &clock, all_day) {
        Ok(None) => RepeatRead::Once,
        Ok(Some(r)) => RepeatRead::Repeats {
            repeat: Repeat {
                freq: r.freq,
                interval: r.interval,
                count: r.count,
                week_days: r.week_days,
                position: r.position,
                month_days: r.month_days,
                months: r.months,
                until: r.until,
            },
        },
        Err(reason) => RepeatRead::Refused { reason },
    }
}

/// An iCloud calendar, as Android's Calendar will list it.
#[derive(uniffi::Record)]
pub struct Cal {
    pub guid: String,
    pub title: String,
    /// `#rrggbb`, or none for Apple's default.
    pub colour: Option<String>,
    /// False for a subscription or a calendar shared read-only.
    pub writable: bool,
}

/// One event, or one occurrence of a repeating event (its own id), in a calendar.
#[derive(uniffi::Record)]
pub struct Ev {
    pub guid: String,
    pub calendar_guid: String,
    pub title: String,
    pub location: Option<String>,
    /// Unix seconds, UTC.
    pub starts_at: i64,
    pub ends_at: i64,
    pub all_day: bool,
    pub timezone: Option<String>,
    pub notes: Option<String>,
    pub etag: Option<String>,
}

/// The calendars, and every event in a span of days, read in one call.
#[derive(uniffi::Record)]
pub struct CalendarWindow {
    pub calendars: Vec<Cal>,
    pub events: Vec<Ev>,
}

/// An event from the phone as the engine's, under `guid`, at Apple's `etag` when known.
fn event_of(e: &Ev, guid: &str, etag: Option<String>) -> quince_core::calendar::Event {
    quince_core::calendar::Event {
        guid: guid.to_string(),
        calendar_guid: e.calendar_guid.clone(),
        title: e.title.clone(),
        location: e.location.clone(),
        starts_at: e.starts_at,
        ends_at: e.ends_at,
        all_day: e.all_day,
        timezone: e.timezone.clone(),
        url: None,
        notes: e.notes.clone(),
        etag: etag.or_else(|| e.etag.clone()),
    }
}

/// A person as the engine's contact, under `guid`. The photo is never touched from here: the
/// merge sees it unchanged and keeps Apple's.
fn contact_of(p: &Person, guid: &str) -> quince_core::contacts::Contact {
    quince_core::contacts::Contact {
        guid: guid.to_string(),
        first_name: p.first_name.clone(),
        middle_name: p.middle_name.clone(),
        last_name: p.last_name.clone(),
        company: p.company.clone(),
        is_company: p.is_company,
        birthday: p.birthday.clone(),
        notes: p.notes.clone(),
        etag: p.etag.clone(),
        methods: p
            .methods
            .iter()
            .map(|m| quince_core::contacts::ContactMethod {
                kind: m.kind.clone(),
                label: m.label.clone(),
                value: m.value.clone(),
            })
            .collect(),
        photo_url: None,
        photo_signature: None,
        photo_crop: None,
    }
}

/// A person as the engine's card: the contact and the rest of the name and work. The engine
/// sends only what changed from the base, field by field, so a field the phone never showed is
/// left as iCloud has it.
fn card_of(contact: quince_core::contacts::Contact, p: &Person) -> quince_core::contacts::Card {
    quince_core::contacts::Card {
        contact,
        details: quince_core::contacts::ContactDetails {
            prefix: p.prefix.clone(),
            suffix: p.suffix.clone(),
            nickname: p.nickname.clone(),
            phonetic_first_name: p.phonetic_first_name.clone(),
            phonetic_last_name: p.phonetic_last_name.clone(),
            job_title: p.job_title.clone(),
            department: p.department.clone(),
            ..Default::default()
        },
    }
}

/// What went wrong, for the screen: the engine's sentence. The whole of it — Apple's status,
/// the connection's own error — goes to the phone's log (`adb logcat -s Skykeep`), where it was
/// lost: "the connection dropped" on screen said nothing of which request or why.
/// The card with the phone's picture change in it: a picture chosen goes as icloud.com sends
/// one, the image itself under Apple's size; a picture taken away is none. No change: the card
/// carries no picture, and the base none either, so Apple's is left exactly as it is.
fn with_photo(
    mut card: quince_core::contacts::Contact,
    edit: Option<&PhotoEdit>,
) -> quince_icloud::Result<quince_core::contacts::Contact> {
    match edit {
        None => {}
        Some(PhotoEdit::Removed) => {
            card.photo_url = None;
            card.photo_crop = None;
        }
        Some(PhotoEdit::Set { image }) => {
            let (url, _) = quince_media::photo_for_apple(image, quince_media::APPLE_PHOTO_LIMIT)
                .ok_or_else(|| {
                    quince_icloud::Error::Refused(
                        "This contact's picture couldn't be read, so it wasn't sent.".into(),
                    )
                })?;
            card.photo_url = Some(url);
            card.photo_crop = None;
        }
    }
    Ok(card)
}

/// The base an edit is laid against. With the picture changed on the phone it holds the picture
/// Apple has now, so the merge sees exactly that change, made against what is really there (a
/// picture changed on an iPhone meanwhile is replaced only because one was chosen here).
fn base_with_photo(
    base: Option<quince_core::contacts::Contact>,
    edit: Option<&PhotoEdit>,
    card: &serde_json::Value,
) -> Option<quince_core::contacts::Contact> {
    let mut base = base?;
    if edit.is_some() {
        base.photo_url = card
            .pointer("/photo/url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        base.photo_crop = None;
    }
    Some(base)
}

/// An error as the app is given it: the reason shown in notifications and logged by the app is
/// redacted as the engine's own log line is, so Apple's query (the account number, the session's
/// ids) never reaches a notification or the phone's log through it.
fn failed(e: impl std::fmt::Display + std::fmt::Debug) -> EngineError {
    log::error(&log::redact(&format!("{e:?}")));
    EngineError::Failed {
        reason: log::redact(&e.to_string()),
    }
}

/// The phone's own log, through Android's `liblog`; nothing elsewhere.
mod drive;
mod notes;
mod photos;
mod reminders;
pub use drive::DItem;
pub use notes::{NDoc, NFolder, NItem, NSpan, NSync};
pub use photos::{PAlbum, PBackup, PIndex, PItem, PSize, PSync, PView};
pub use reminders::{RCounts, REdit, RItem, RList, RSync, RView};

mod log {
    /// A line for the phone's log with every web address cut at its `?`: Apple's addresses carry
    /// the account's number and the session's ids in their query, and the log ends up in bug
    /// reports and on any computer connected over USB. The address itself (which call failed)
    /// stays, as the desktop's diagnostics keep it.
    pub fn redact(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find("http") {
            let (before, url) = rest.split_at(at);
            out.push_str(before);
            let end = url
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ','))
                .unwrap_or(url.len());
            let (address, after) = url.split_at(end);
            match address.find('?') {
                Some(q) => {
                    out.push_str(&address[..q]);
                    out.push_str("?…");
                }
                None => out.push_str(address),
            }
            rest = after;
        }
        out.push_str(rest);
        out
    }

    #[cfg(target_os = "android")]
    pub fn error(text: &str) {
        use std::ffi::{c_char, c_int, CString};
        #[link(name = "log")]
        extern "C" {
            fn __android_log_write(
                priority: c_int,
                tag: *const c_char,
                text: *const c_char,
            ) -> c_int;
        }
        const ERROR: c_int = 6;
        let (Ok(tag), Ok(text)) = (
            CString::new("Skykeep"),
            CString::new(text.replace('\0', " ")),
        ) else {
            return;
        };
        // SAFETY: both are valid, NUL-terminated strings that outlive the call.
        unsafe {
            __android_log_write(ERROR, tag.as_ptr(), text.as_ptr());
        }
    }

    #[cfg(not(target_os = "android"))]
    pub fn error(_text: &str) {}
}

/// One Apple Account, signed in or being signed in: the desktop app's own sign-in (SRP,
/// two-factor, the session sealed in the Keystore through [`start`]), unchanged.
#[derive(uniffi::Object)]
pub struct Account {
    client: std::sync::Mutex<quince_auth::Client>,
    /// The app's private files: the session, and each account's library under `libraries/`.
    dir: std::path::PathBuf,
    /// The open library, and whose it is (reminders.rs).
    library: std::sync::Mutex<
        Option<(
            String,
            std::sync::Arc<quince_sync::library_lock::LibraryLock>,
        )>,
    >,
}

#[uniffi::export]
impl Account {
    /// The account whose state lives in `state_dir` (the app's private files).
    #[uniffi::constructor]
    pub fn new(state_dir: String) -> Result<std::sync::Arc<Self>, EngineError> {
        let client = quince_auth::Client::new(std::path::Path::new(&state_dir)).map_err(failed)?;
        Ok(std::sync::Arc::new(Self {
            client: std::sync::Mutex::new(client),
            dir: std::path::PathBuf::from(state_dir),
            library: std::sync::Mutex::new(None),
        }))
    }

    /// Signs in again with the session kept from last time; false when there is none to use.
    /// Whether a session is kept on the phone: answered from the phone alone, so the app can
    /// open the library at once and let [`Account::reopen`] check with Apple behind it.
    pub fn has_session(&self) -> bool {
        self.client.lock().is_ok_and(|c| c.has_stored_session())
    }

    /// Where the app goes, as the desktop decides it: offline with a session kept is the
    /// library; only Apple refusing the session, or nothing kept, is sign-in.
    pub fn reopen(&self) -> Result<Reopened, EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        Ok(match runtime().block_on(c.reopen()).map_err(failed)? {
            quince_auth::Reopen::Library => Reopened::Library,
            quince_auth::Reopen::Code => Reopened::Code,
            quince_auth::Reopen::SignIn => Reopened::SignIn,
        })
    }

    pub fn resume(&self) -> Result<bool, EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime().block_on(c.resume()).map_err(failed)
    }

    pub fn sign_in(&self, apple_id: String, password: String) -> Result<SignIn, EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        match runtime()
            .block_on(c.sign_in(&apple_id, &password))
            .map_err(failed)?
        {
            quince_auth::SignInOutcome::Complete => Ok(SignIn::Complete),
            quince_auth::SignInOutcome::NeedsTwoFactor => Ok(SignIn::NeedsCode),
        }
    }

    pub fn code_options(&self) -> Result<CodeOptions, EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        let o = runtime().block_on(c.two_factor_options()).map_err(failed)?;
        Ok(CodeOptions {
            trusted_devices: o.trusted_device_count,
            phones: o
                .phones
                .into_iter()
                .map(|p| TrustedPhone {
                    id: p.id,
                    masked_number: p.masked_number,
                })
                .collect(),
            too_many_codes_sent: o.too_many_codes_sent,
            security_key: o.security_key,
        })
    }

    /// The code shown on a trusted device.
    pub fn submit_code(&self, code: String) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime()
            .block_on(c.submit_two_factor(&code))
            .map_err(failed)
    }

    pub fn resend_to_devices(&self) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime().block_on(c.resend_to_devices()).map_err(failed)
    }

    pub fn send_sms(&self, phone_id: u64) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime().block_on(c.send_sms(phone_id)).map_err(failed)
    }

    pub fn submit_sms_code(&self, phone_id: u64, code: String) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        runtime()
            .block_on(c.submit_sms_code(phone_id, &code))
            .map_err(failed)
    }

    /// The Apple Account signed in, as Android's account list names it.
    pub fn apple_id(&self) -> Option<String> {
        let c = self.client.lock().ok()?;
        c.session().apple_id.clone()
    }

    /// The signed-in account's own tag (from Apple's account number, never the Apple ID typed):
    /// the same for every spelling of the Apple ID, different for every other account. Android's
    /// account for this Apple Account carries it, so a sync never runs for another account's.
    pub fn account_tag(&self) -> Option<String> {
        let c = self.client.lock().ok()?;
        quince_sync::queue::asked_as(&c.service_params())
    }

    /// The whole iCloud address book, read as desktop reads it (Apple's `/co/startup`).
    pub fn contacts(&self) -> Result<Vec<Person>, EngineError> {
        Ok(self.address_book()?.people)
    }

    /// The whole iCloud address book: the people, and the lists (Android's labels) they are in.
    pub fn address_book(&self) -> Result<Book, EngineError> {
        let c = self.client.lock().map_err(failed)?;
        let root = c
            .session()
            .service_url("contacts")
            .ok_or_else(|| failed("Apple hasn't offered a contacts service for this account."))?;
        let (book, _) = runtime()
            .block_on(quince_icloud::contacts::fetch_book_with_tokens(
                c.http(),
                &root,
                c.service_params(),
            ))
            .map_err(failed)?;
        let mut details: std::collections::HashMap<String, quince_core::contacts::ContactDetails> =
            book.details.into_iter().collect();
        let mut groups: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for list in &book.lists {
            for member in &list.members {
                groups
                    .entry(member.clone())
                    .or_default()
                    .push(list.guid.clone());
            }
        }
        let lists = book
            .lists
            .iter()
            .map(|l| CList {
                guid: l.guid.clone(),
                name: l.name.clone(),
                etag: l.etag.clone(),
            })
            .collect();
        let people = book
            .contacts
            .into_iter()
            .map(|p| {
                let d = details.remove(&p.guid).unwrap_or_default();
                let in_lists = groups.remove(&p.guid).unwrap_or_default();
                Person {
                    guid: p.guid,
                    etag: p.etag,
                    first_name: p.first_name,
                    middle_name: p.middle_name,
                    last_name: p.last_name,
                    company: p.company,
                    is_company: p.is_company,
                    birthday: p.birthday,
                    notes: p.notes,
                    methods: p
                        .methods
                        .into_iter()
                        .map(|m| Method {
                            kind: m.kind,
                            label: m.label,
                            value: m.value,
                        })
                        .collect(),
                    photo: p.photo_url.map(|url| PhotoRef {
                        key: p.photo_signature.unwrap_or_else(|| url.clone()),
                        url,
                        crop: p.photo_crop,
                    }),
                    prefix: d.prefix,
                    suffix: d.suffix,
                    nickname: d.nickname,
                    phonetic_first_name: d.phonetic_first_name,
                    phonetic_last_name: d.phonetic_last_name,
                    job_title: d.job_title,
                    department: d.department,
                    groups: in_lists,
                }
            })
            .collect();
        Ok(Book { people, lists })
    }

    /// A card's picture for the phone's Contacts: cut to the square its owner chose and `size`
    /// pixels on a side, as the desktop draws faces (`quince_media::avatar`). Android makes its
    /// own smaller copy for lists from this one.
    pub fn contact_photo(&self, photo: PhotoRef, size: u32) -> Result<Vec<u8>, EngineError> {
        let (http, params) = {
            let c = self.client.lock().map_err(failed)?;
            (c.http().clone(), c.service_params())
        };
        let bytes = runtime()
            .block_on(quince_icloud::contacts::photo_bytes(
                &http, &params, &photo.url,
            ))
            .map_err(failed)?;
        let crop = photo.crop.as_deref().and_then(quince_media::Crop::parse);
        quince_media::avatar(&bytes, crop, size).map_err(failed)
    }

    /// The phone's changes to lists sent to iCloud, each laid over the list as iCloud has it
    /// now: one read of the address book, then each change in order. One outcome per change.
    pub fn save_lists(&self, changes: Vec<ListChange>) -> Result<Vec<ListOutcome>, EngineError> {
        use quince_icloud::contacts as c;
        let (http, root, params) = {
            let cl = self.client.lock().map_err(failed)?;
            let root = cl.session().service_url("contacts").ok_or_else(|| {
                failed("Apple hasn't offered a contacts service for this account.")
            })?;
            (cl.http().clone(), root, cl.service_params())
        };
        let ledger = c::Ledger::new();
        let mut lists = runtime()
            .block_on(c::fetch_book_with_tokens(&http, &root, params.clone()))
            .map_err(failed)?
            .0
            .lists;
        let mut out = Vec::with_capacity(changes.len());
        for change in changes {
            let at = |guid: &str| lists.iter().position(|l| l.guid == guid);
            let sent: quince_icloud::Result<ListOutcome> = runtime().block_on(async {
                match change {
                    ListChange::Create { name, members } => {
                        let guid =
                            c::create_list(&http, &root, params.clone(), &ledger, &name, &members)
                                .await?;
                        Ok(ListOutcome::Saved { guid })
                    }
                    ListChange::Rename { guid, name } => match at(&guid) {
                        None => Ok(ListOutcome::Gone),
                        Some(i) => {
                            let mut list = lists[i].clone();
                            list.name = name;
                            c::update_list(&http, &root, params.clone(), &ledger, &list).await?;
                            Ok(ListOutcome::Saved { guid })
                        }
                    },
                    ListChange::Delete { guid } => {
                        if at(&guid).is_some() {
                            c::delete_list(&http, &root, params.clone(), &ledger, &guid).await?;
                        }
                        Ok(ListOutcome::Saved { guid })
                    }
                    ListChange::Members { guid, add, remove } => match at(&guid) {
                        None => Ok(ListOutcome::Gone),
                        Some(i) => {
                            let mut list = lists[i].clone();
                            list.members.retain(|m| !remove.contains(m));
                            for m in add {
                                if !list.members.contains(&m) {
                                    list.members.push(m);
                                }
                            }
                            c::update_list(&http, &root, params.clone(), &ledger, &list).await?;
                            Ok(ListOutcome::Saved { guid })
                        }
                    },
                }
            });
            // What this change wrote is old in the lists read before it: read again.
            ledger.spend();
            if matches!(sent, Ok(ListOutcome::Saved { .. })) {
                if let Ok((fresh, _)) =
                    runtime().block_on(c::fetch_book_with_tokens(&http, &root, params.clone()))
                {
                    lists = fresh.lists;
                }
            }
            out.push(match sent {
                Ok(done) => done,
                Err(e) => {
                    log::error(&log::redact(&format!("{e:?}")));
                    let later = matches!(
                        e,
                        quince_icloud::Error::Network(_)
                            | quince_icloud::Error::Busy
                            | quince_icloud::Error::Conflict
                            | quince_icloud::Error::ReauthRequired
                    );
                    ListOutcome::Failed {
                        reason: e.to_string(),
                        later,
                    }
                }
            });
        }
        Ok(out)
    }

    /// The phone's changes to contacts sent to iCloud, as the desktop sends its own (outbox.rs):
    /// the book read once and again only when its tokens are spent; a creation under the id the
    /// phone chose, made at most once; an edit as a three-way merge over Apple's current card,
    /// so what changed on another device meanwhile stays; a deletion against the card as it is
    /// now. One outcome per change, in order. Fails as a whole only when the book can't be read.
    pub fn save_contacts(
        &self,
        changes: Vec<ContactChange>,
    ) -> Result<Vec<ChangeOutcome>, EngineError> {
        use quince_icloud::contacts as c;
        let client = self.client.lock().map_err(failed)?;
        let root = client
            .session()
            .service_url("contacts")
            .ok_or_else(|| failed("Apple hasn't offered a contacts service for this account."))?;
        let (http, params) = (client.http().clone(), client.service_params());
        drop(client);
        let ledger = c::Ledger::new();
        let mut cards: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let mut out = Vec::with_capacity(changes.len());
        for change in changes {
            if !ledger.has_tokens() {
                let (read, tokens) = runtime()
                    .block_on(c::cards_with_tokens(&http, &root, params.clone()))
                    .map_err(failed)?;
                ledger.hold(tokens);
                cards = read
                    .into_iter()
                    .filter_map(|card| Some((card.get("contactId")?.as_str()?.to_string(), card)))
                    .collect();
            }
            let current = cards.get(&change.guid).cloned();
            let sent: quince_icloud::Result<ChangeOutcome> = runtime().block_on(async {
                match change.kind {
                    ChangeKind::Delete => match &current {
                        None => Ok(ChangeOutcome::Saved),
                        Some(card) => {
                            c::merge::delete_card(&http, &root, params.clone(), &ledger, card)
                                .await
                                .map(|()| ChangeOutcome::Saved)
                        }
                    },
                    ChangeKind::Create => {
                        if current.is_some() {
                            return Ok(ChangeOutcome::Saved);
                        }
                        let person = change.person.as_ref().ok_or_else(|| {
                            quince_icloud::Error::Protocol(
                                "a new contact with nothing in it".into(),
                            )
                        })?;
                        let card = card_of(
                            with_photo(contact_of(person, &change.guid), change.photo.as_ref())?,
                            person,
                        );
                        c::create_card_named(
                            &http,
                            &root,
                            params.clone(),
                            &ledger,
                            &change.guid,
                            &card,
                        )
                        .await
                        .map(|()| ChangeOutcome::Saved)
                    }
                    ChangeKind::Update => {
                        let person = change.person.as_ref().ok_or_else(|| {
                            quince_icloud::Error::Protocol("an edit with nothing in it".into())
                        })?;
                        match &current {
                            // Deleted in iCloud while edited here: kept as a new card.
                            None => {
                                let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
                                let card = card_of(
                                    with_photo(contact_of(person, &guid), change.photo.as_ref())?,
                                    person,
                                );
                                c::create_card_named(
                                    &http,
                                    &root,
                                    params.clone(),
                                    &ledger,
                                    &guid,
                                    &card,
                                )
                                .await
                                .map(|()| ChangeOutcome::Recreated { guid })
                            }
                            Some(card) => {
                                let base = change.base.as_ref().and_then(|b| {
                                    base_with_photo(
                                        Some(contact_of(b, &change.guid)),
                                        change.photo.as_ref(),
                                        card,
                                    )
                                    .map(|contact| card_of(contact, b))
                                });
                                let mine = card_of(
                                    with_photo(
                                        contact_of(person, &change.guid),
                                        change.photo.as_ref(),
                                    )?,
                                    person,
                                );
                                c::update_card(
                                    &http,
                                    &root,
                                    params.clone(),
                                    &ledger,
                                    &mine,
                                    Some(card),
                                    base.as_ref(),
                                )
                                .await
                                .map(|()| ChangeOutcome::Saved)
                            }
                        }
                    }
                }
            });
            // What this change wrote is old in the book read now: read again before the next.
            ledger.spend();
            out.push(match sent {
                Ok(done) => done,
                Err(e) => {
                    log::error(&log::redact(&format!("{e:?}")));
                    let later = matches!(
                        e,
                        quince_icloud::Error::Network(_)
                            | quince_icloud::Error::Busy
                            | quince_icloud::Error::Conflict
                            | quince_icloud::Error::ReauthRequired
                    );
                    ChangeOutcome::Failed {
                        reason: e.to_string(),
                        later,
                    }
                }
            });
        }
        Ok(out)
    }

    /// The calendars and the events between two days (`YYYY-MM-DD`, both included) in the
    /// time zone `tz`, read exactly as the desktop reads them (`sync::fetch_calendar`): in
    /// windows Apple accepts, each asked of `/ca/startup` and `/ca/events` and merged — one
    /// `/ca/startup` for thirteen months was refused (400), and startup alone sometimes brings
    /// no events. Repeating events come as their occurrences, each with its own id.
    pub fn calendar_window(
        &self,
        from: String,
        to: String,
        tz: String,
    ) -> Result<CalendarWindow, EngineError> {
        let c = self.client.lock().map_err(failed)?;
        let root = c
            .session()
            .service_url("calendar")
            .ok_or_else(|| failed("Apple hasn't offered a calendar service for this account."))?;
        let snap = runtime()
            .block_on(quince_icloud::sync::fetch_calendar(
                c.http(),
                &root,
                c.service_params(),
                &from,
                &to,
                &tz,
            ))
            .map_err(failed)?;
        Ok(CalendarWindow {
            calendars: snap
                .calendars
                .into_iter()
                .map(|k| Cal {
                    guid: k.guid,
                    title: k.title,
                    colour: k.colour,
                    writable: k.writable,
                })
                .collect(),
            events: snap
                .events
                .into_iter()
                .map(|e| Ev {
                    guid: e.guid,
                    calendar_guid: e.calendar_guid,
                    title: e.title,
                    location: e.location,
                    starts_at: e.starts_at,
                    ends_at: e.ends_at,
                    all_day: e.all_day,
                    timezone: e.timezone,
                    notes: e.notes,
                    etag: e.etag,
                })
                .collect(),
        })
    }

    /// The phone's changes to events sent to iCloud. Apple's copies read once for the span
    /// (`from`..=`to` in `tz`, as [`Self::calendar_window`] reads it). A new event under the id
    /// the phone chose, made at most once; an edit laid over Apple's copy as it is now with
    /// `merge_edit` — a field changed only on another device keeps that change — and, for one
    /// occurrence of a repeating event, saved for that occurrence alone, as "This Event" does on
    /// an iPhone; a deletion likewise. An edit to an event deleted elsewhere is kept as a new
    /// one. One outcome per change, in order.
    pub fn save_events(
        &self,
        changes: Vec<EventChange>,
        from: String,
        to: String,
        tz: String,
    ) -> Result<Vec<ChangeOutcome>, EngineError> {
        use quince_icloud::calendar::{self as k, CalendarClient, EventRepeat, Scope};
        let client = self.client.lock().map_err(failed)?;
        let root = client
            .session()
            .service_url("calendar")
            .ok_or_else(|| failed("Apple hasn't offered a calendar service for this account."))?;
        let (http, params) = (client.http().clone(), client.service_params());
        drop(client);
        let theirs: std::collections::HashMap<String, quince_core::calendar::Event> = runtime()
            .block_on(quince_icloud::sync::fetch_calendar(
                &http,
                &root,
                params.clone(),
                &from,
                &to,
                &tz,
            ))
            .map_err(failed)?
            .events
            .into_iter()
            .map(|e| (e.guid.clone(), e))
            .collect();
        let cal = CalendarClient::new(&http, &root, params);
        let scope = |guid: &str| k::is_occurrence(guid).then_some(Scope::This);
        let mut out = Vec::with_capacity(changes.len());
        for change in changes {
            let current = theirs.get(&change.guid);
            let sent: quince_icloud::Result<ChangeOutcome> = runtime().block_on(async {
                match change.kind {
                    ChangeKind::Delete => match current {
                        None => Ok(ChangeOutcome::Saved),
                        Some(event) => cal
                            .delete_event_scoped(event, None, scope(&event.guid))
                            .await
                            .map(|()| ChangeOutcome::Saved),
                    },
                    ChangeKind::Create => {
                        if current.is_some() {
                            return Ok(ChangeOutcome::Saved);
                        }
                        let ev = change.event.as_ref().ok_or_else(|| {
                            quince_icloud::Error::Protocol("a new event with nothing in it".into())
                        })?;
                        let repeat = change.repeat.as_ref().map(|r| EventRepeat {
                            freq: r.freq.clone(),
                            interval: r.interval,
                            count: r.count,
                            week_days: r.week_days.clone(),
                            position: r.position.clone(),
                            month_days: r.month_days.clone(),
                            months: r.months.clone(),
                            until: r.until.clone(),
                        });
                        cal.put_event_full(
                            &event_of(ev, &change.guid, None),
                            None,
                            None,
                            repeat.as_ref().map(Some),
                            None,
                        )
                        .await
                        .map(|_| ChangeOutcome::Saved)
                    }
                    ChangeKind::Update => {
                        let ev = change.event.as_ref().ok_or_else(|| {
                            quince_icloud::Error::Protocol("an edit with nothing in it".into())
                        })?;
                        match current {
                            // Deleted in iCloud while edited here: kept, as an event of its own.
                            None => {
                                let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
                                cal.put_event(&event_of(ev, &guid, None), None)
                                    .await
                                    .map(|_| ChangeOutcome::Recreated { guid })
                            }
                            Some(now) => {
                                let mut mine = event_of(ev, &change.guid, now.etag.clone());
                                // The link is not on the phone: Apple's kept (the merge keeps it
                                // as unchanged here; with no base it is carried over).
                                mine.url.clone_from(&now.url);
                                let mut merged = match &change.base {
                                    Some(b) => {
                                        let mut base = event_of(b, &change.guid, None);
                                        base.url.clone_from(&now.url);
                                        k::merge_edit(&base, &mine, now)
                                    }
                                    None => mine,
                                };
                                if merged.calendar_guid != now.calendar_guid
                                    && !k::is_occurrence(&change.guid)
                                {
                                    // Moved to another calendar here: moved at Apple, as
                                    // icloud.com moves one — never a second copy in the new one.
                                    cal.put_event_moved(
                                        &merged,
                                        None,
                                        None,
                                        None,
                                        None,
                                        Some((now.calendar_guid.as_str(), None)),
                                    )
                                    .await
                                    .map(|_| ChangeOutcome::Saved)
                                } else {
                                    // One occurrence keeps its series' calendar: Apple moves a
                                    // series, never one of its occurrences alone.
                                    merged.calendar_guid.clone_from(&now.calendar_guid);
                                    cal.put_event_full(
                                        &merged,
                                        None,
                                        None,
                                        None,
                                        scope(&change.guid),
                                    )
                                    .await
                                    .map(|_| ChangeOutcome::Saved)
                                }
                            }
                        }
                    }
                }
            });
            out.push(match sent {
                Ok(done) => done,
                Err(e) => {
                    log::error(&log::redact(&format!("{e:?}")));
                    let later = matches!(
                        e,
                        quince_icloud::Error::Network(_)
                            | quince_icloud::Error::Busy
                            | quince_icloud::Error::Conflict
                            | quince_icloud::Error::ReauthRequired
                    );
                    ChangeOutcome::Failed {
                        reason: e.to_string(),
                        later,
                    }
                }
            });
        }
        Ok(out)
    }

    /// Everything this account keeps on the phone deleted: its library (notes, reminders, photo
    /// and Drive lists, changes not yet sent), as the desktop's Remove from this computer does.
    /// Called before [`Account::sign_out`] when the person chooses Remove from Phone; the app
    /// clears its caches beside it.
    pub fn forget_account(&self) -> Result<(), EngineError> {
        let session = {
            let c = self.client.lock().map_err(failed)?;
            quince_sync::queue::asked_as(&c.service_params())
        };
        // Closed first, so nothing writes into the folder as it goes. Whose it is comes from the
        // session, or else from the library open (a session already half gone still names its
        // library); with neither, nothing can be said to be removed, and the person is told.
        let open = self
            .library
            .lock()
            .map_err(failed)?
            .take()
            .map(|(who, _)| who);
        let account = session.or(open).ok_or_else(|| {
            failed("Couldn't tell which account's copy to remove. Sign in again, then remove it.")
        })?;
        let dir = self.dir.join("libraries").join(account);
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(failed(e)),
            _ => Ok(()),
        }
    }

    /// Signs out here: the session forgotten on this phone, its secrets deleted.
    pub fn sign_out(&self) -> Result<(), EngineError> {
        let mut c = self.client.lock().map_err(failed)?;
        c.sign_out().map_err(failed)
    }
}

#[cfg(test)]
mod log_tests {
    /// Apple's query (the account number, the session's ids) never reaches the phone's log;
    /// the address and the rest of the line do.
    #[test]
    fn a_logged_address_loses_its_query() {
        let line = r#"Network(reqwest::Error { url: "https://p42-ckdatabasews.icloud.com/database/1/x?dsid=12345&clientId=ab", source: dns })"#;
        let out = super::log::redact(line);
        assert!(!out.contains("12345") && !out.contains("clientId"), "{out}");
        assert!(
            out.contains("https://p42-ckdatabasews.icloud.com/database/1/x?…"),
            "{out}"
        );
        assert!(out.ends_with("source: dns })"), "{out}");
        assert_eq!(super::log::redact("no address here"), "no address here");
    }
}
