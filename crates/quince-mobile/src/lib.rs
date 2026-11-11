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
#[derive(uniffi::Record, Clone)]
pub struct Method {
    pub kind: String,
    pub label: Option<String>,
    pub value: String,
}

/// A person in the iCloud address book, as Android's Contacts will hold them.
#[derive(uniffi::Record, Clone)]
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
    /// The card's postal addresses, part by part. `None` in a base kept before addresses were:
    /// what changed in them can't be told, so they are left as Apple has them.
    pub addresses: Option<Vec<contacts::Address>>,
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
    /// The card the person has said is their own, if they have: the account's picture.
    pub me: Option<String>,
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

/// What changed on the phone to one contact or event, to be sent to iCloud.
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind {
    /// Made on the phone: created in iCloud under `guid`, chosen on the phone beforehand so a
    /// send repeated after a lost answer finds it made rather than making a second.
    Create,
    /// Edited on the phone: only what changed from `base` written over iCloud's current copy.
    Update,
    /// Deleted on the phone.
    Delete,
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
#[derive(uniffi::Record, Clone)]
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
    /// The event's link. Android's Calendar has no place for it; kept in the row's base so an
    /// edit made on the phone leaves it as Apple has it.
    pub url: Option<String>,
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
        url: e.url.clone(),
        notes: e.notes.clone(),
        etag: etag.or_else(|| e.etag.clone()),
    }
}

/// The kinds of detail the phone's contacts hold and edit. Every other kind Apple keeps on a
/// card (related names, dates, social profiles, instant messaging) the phone never reads.
const PHONE_KINDS: [&str; 4] = ["phone", "email", "url", "address"];

/// An edit as the engine sends it: the phone's own kinds as edited, every other kind as the
/// base had it. Without this, a card edited on the phone lost at Apple every related name,
/// date, profile and messaging handle, since they were absent from the edit and present in
/// the base, which reads as removed.
fn keep_unmodelled(edit: &Person, base: Option<&Person>) -> Person {
    let mut out = edit.clone();
    out.methods
        .retain(|m| PHONE_KINDS.contains(&m.kind.as_str()));
    if let Some(base) = base {
        out.methods.extend(
            base.methods
                .iter()
                .filter(|m| !PHONE_KINDS.contains(&m.kind.as_str()))
                .cloned(),
        );
    }
    out
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
        // A day and month without a year is Apple's 1604 (occasions::birthday_from_phone).
        birthday: p
            .birthday
            .as_deref()
            .map(quince_icloud::occasions::birthday_from_phone),
        notes: p.notes.clone(),
        etag: p.etag.clone(),
        // Addresses go part by part (contacts.rs), never as a line of text in the card.
        methods: p
            .methods
            .iter()
            .filter(|m| m.kind != "address")
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

/// An error as the app is given it: the reason shown in notifications and logged by the app is
/// redacted as the engine's own log line is, so Apple's query (the account number, the session's
/// ids) never reaches a notification or the phone's log through it.
fn failed(e: impl std::fmt::Display + std::fmt::Debug) -> EngineError {
    log::error(&log::redact(&format!("{e:?}")));
    EngineError::Failed {
        reason: log::redact(&e.to_string()),
    }
}

mod calendar;
mod contacts;
mod drive;
mod notes;
mod outbox;
mod pass;
mod photos;
mod reminders;
pub use calendar::{CalendarSent, PhoneEvent};
pub use contacts::{AccountProfile, Address, BookRead, PhoneCard, PhoneList, SendState, Settled};
pub use drive::DItem;
pub use notes::{NDoc, NFolder, NItem, NSpan, NSync};
pub use outbox::NotSent;
pub use pass::{DataAccessNow, DataState, SessionState};
pub use photos::{
    PAlbum, PBackup, PIndex, PItem, PLibrary, PLibraryChoice, PSharedAlbum, PSize, PSync, PView,
};
pub use reminders::{RCounts, REdit, RItem, RList, RSync, RView};

/// The phone's own log, through Android's `liblog`; nothing elsewhere.
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
    /// iCloud Drive files read in parts for the Files app (drive.rs): where the parts are kept
    /// and how much disk they may take, as the app set it.
    drive_parts: std::sync::Mutex<Option<(std::path::PathBuf, u64)>>,
    /// The parts kept, and whose they are: opened per account, as the library is.
    drive_cache: std::sync::Mutex<Option<(String, std::sync::Arc<quince_vfs::Cache>)>>,
    /// Drive's download links in use, each asked of Apple once while it lasts.
    drive_links: std::sync::Arc<quince_icloud::drive_parts::Links>,
    /// Files written by other apps through the Files app and not yet at Apple (drive.rs):
    /// the desktop folder's own journal, opened per account.
    drive_writes: std::sync::Mutex<Option<(String, std::sync::Arc<quince_vfs::writes::Writes>)>>,
}

/// The HTTP client, a service's address and the session's query: one call to Apple's needs.
pub(crate) type Service = (reqwest::Client, String, Vec<(String, String)>);

impl Account {
    /// The session, even after a call that held it panicked: the panic failed that one call
    /// (UniFFI turns it into an exception), and every later one would otherwise fail too,
    /// `has_session` answering "signed out" until the app was killed.
    pub(crate) fn client(&self) -> std::sync::MutexGuard<'_, quince_auth::Client> {
        self.client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// What a call to Apple needs, taken from the session and the lock let go of at once: a
    /// read that takes a minute never holds sign-in, a sync of another service, or the app's
    /// screens waiting behind it. `None` when Apple hasn't offered `service`.
    pub(crate) fn service(&self, service: &str) -> Option<Service> {
        let c = self.client();
        let root = c.session().service_url(service)?;
        Some((c.http().clone(), root, c.service_params()))
    }
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
            drive_parts: std::sync::Mutex::new(None),
            drive_cache: std::sync::Mutex::new(None),
            drive_links: std::sync::Arc::new(quince_icloud::drive_parts::Links::new()),
            drive_writes: std::sync::Mutex::new(None),
        }))
    }

    /// Signs in again with the session kept from last time; false when there is none to use.
    /// Whether a session is kept on the phone: answered from the phone alone, so the app can
    /// open the library at once and let [`Account::reopen`] check with Apple behind it.
    pub fn has_session(&self) -> bool {
        self.client().has_stored_session()
    }

    /// Where the app goes, as the desktop decides it: offline with a session kept is the
    /// library; only Apple refusing the session, or nothing kept, is sign-in.
    pub fn reopen(&self) -> Result<Reopened, EngineError> {
        let mut c = self.client();
        Ok(match runtime().block_on(c.reopen()).map_err(failed)? {
            quince_auth::Reopen::Library => Reopened::Library,
            quince_auth::Reopen::Code => Reopened::Code,
            quince_auth::Reopen::SignIn => Reopened::SignIn,
        })
    }

    pub fn resume(&self) -> Result<bool, EngineError> {
        let mut c = self.client();
        runtime().block_on(c.resume()).map_err(failed)
    }

    pub fn sign_in(&self, apple_id: String, password: String) -> Result<SignIn, EngineError> {
        let mut c = self.client();
        match runtime()
            .block_on(c.sign_in(&apple_id, &password))
            .map_err(failed)?
        {
            quince_auth::SignInOutcome::Complete => Ok(SignIn::Complete),
            quince_auth::SignInOutcome::NeedsTwoFactor => Ok(SignIn::NeedsCode),
        }
    }

    pub fn code_options(&self) -> Result<CodeOptions, EngineError> {
        let mut c = self.client();
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
        let mut c = self.client();
        runtime()
            .block_on(c.submit_two_factor(&code))
            .map_err(failed)
    }

    pub fn resend_to_devices(&self) -> Result<(), EngineError> {
        let mut c = self.client();
        runtime().block_on(c.resend_to_devices()).map_err(failed)
    }

    pub fn send_sms(&self, phone_id: u64) -> Result<(), EngineError> {
        let mut c = self.client();
        runtime().block_on(c.send_sms(phone_id)).map_err(failed)
    }

    pub fn submit_sms_code(&self, phone_id: u64, code: String) -> Result<(), EngineError> {
        let mut c = self.client();
        runtime()
            .block_on(c.submit_sms_code(phone_id, &code))
            .map_err(failed)
    }

    /// The Apple Account signed in, as Android's account list names it.
    pub fn apple_id(&self) -> Option<String> {
        let c = self.client();
        c.session().apple_id.clone()
    }

    /// The signed-in account's own tag (from Apple's account number, never the Apple ID typed):
    /// the same for every spelling of the Apple ID, different for every other account. Android's
    /// account for this Apple Account carries it, so a sync never runs for another account's.
    pub fn account_tag(&self) -> Option<String> {
        let c = self.client();
        quince_sync::queue::asked_as(&c.service_params())
    }

    /// A card's picture for the phone's Contacts: cut to the square its owner chose and `size`
    /// pixels on a side, as the desktop draws faces (`quince_media::avatar`). Android makes its
    /// own smaller copy for lists from this one.
    pub fn contact_photo(&self, photo: PhotoRef, size: u32) -> Result<Vec<u8>, EngineError> {
        let (http, params) = {
            let c = self.client();
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
        let (http, root, params) = self
            .service("calendar")
            .ok_or_else(|| failed("Apple hasn't offered a calendar service for this account."))?;
        let snap = runtime()
            .block_on(quince_icloud::sync::fetch_calendar(
                &http, &root, params, &from, &to, &tz,
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
            events: snap.events.into_iter().map(calendar::ev_of).collect(),
        })
    }

    /// Everything this account keeps on the phone deleted: its library (notes, reminders, photo
    /// and Drive lists, changes not yet sent), as the desktop's Remove from this computer does.
    /// Called before [`Account::sign_out`] when the person chooses Remove from Phone; the app
    /// clears its caches beside it.
    pub fn forget_account(&self) -> Result<(), EngineError> {
        let session = {
            let c = self.client();
            quince_sync::queue::asked_as(&c.service_params())
        };
        self.forget_drive_parts();
        // Closed first, so nothing writes into the folder as it goes. Whose it is comes from the
        // session, or else from the library open (a session already half gone still names its
        // library); with neither, nothing can be said to be removed, and the person is told.
        let open = self
            .library
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        // Drive's parts read here are the session's: never served to whoever signs in next.
        self.forget_drive_parts();
        let mut c = self.client();
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

#[cfg(test)]
mod contact_edit_tests {
    use super::{keep_unmodelled, Method, Person};

    fn person(methods: &[(&str, &str)]) -> Person {
        Person {
            guid: "G".into(),
            etag: None,
            first_name: Some("Ada".into()),
            middle_name: None,
            last_name: None,
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            methods: methods
                .iter()
                .map(|(kind, value)| Method {
                    kind: (*kind).into(),
                    label: None,
                    value: (*value).into(),
                })
                .collect(),
            prefix: None,
            suffix: None,
            nickname: None,
            phonetic_first_name: None,
            phonetic_last_name: None,
            job_title: None,
            department: None,
            groups: vec![],
            photo: None,
            addresses: None,
        }
    }

    /// The phone edits phones, emails, links and addresses; a card's related names, dates,
    /// profiles and messaging handles go to Apple as the base had them, never as removed.
    #[test]
    fn an_edit_keeps_what_the_phone_never_shows() {
        let base = person(&[
            ("phone", "1"),
            ("related", "Mum"),
            ("date", "2020-01-01"),
            ("im", "ada"),
        ]);
        let edit = person(&[("phone", "2"), ("email", "a@example.com")]);
        let sent = keep_unmodelled(&edit, Some(&base));
        let kinds: Vec<(&str, &str)> = sent
            .methods
            .iter()
            .map(|m| (m.kind.as_str(), m.value.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("phone", "2"),
                ("email", "a@example.com"),
                ("related", "Mum"),
                ("date", "2020-01-01"),
                ("im", "ada")
            ]
        );
        // Without a base, the edit is sent as it is.
        assert_eq!(keep_unmodelled(&edit, None).methods.len(), 2);
    }
}
