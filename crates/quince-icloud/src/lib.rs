//! Clients for Apple's private iCloud web services, riding an authenticated session.
//!
//! These are reverse-engineered endpoints, not a published API. Every response is treated as
//! untrusted: a missing field or a malformed date returns an error, never a panic.

/// What went wrong, said on the terminal in a development build only, and never with anything
/// of the person's in it — no names, record ids, addresses or Apple's answers. A release build
/// says nothing at all: Quince keeps no diagnostics and sends none anywhere.
macro_rules! quiet {
    ($($t:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($t)*);
        }
    };
}

pub mod bounded;
pub mod caldav;
pub mod calendar;
pub mod calendar_feeds;
pub mod ckdocument;
pub mod cloudkit;
pub mod contacts;
pub mod courier;
pub mod drive;
pub mod drive_changes;
pub mod drive_parts;
pub mod ical_edit;
pub mod markdown;
pub mod mergeable;
pub mod nextcloud;
pub mod note_media;
pub mod notes;
pub mod occasions;
pub mod photos;
pub mod photos_upload;
pub mod push;
pub mod reminders;
pub mod rrule;
pub mod shared_albums;
pub mod shared_collections;
pub mod sync;
pub mod table;
pub mod topotext;
pub mod zones;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Said in words, never with the address: an address carries the account's id and this
    /// install's client id, and was shown on screen whole.
    #[error("{}", network_words(.0))]
    Network(#[from] reqwest::Error),
    #[error("iCloud sent back something unexpected: {0}")]
    Protocol(String),
    #[error("this needs a fresh sign-in")]
    ReauthRequired,
    /// Advanced Data Protection holds this service's keys on the person's devices, and Apple
    /// releases them to a session only once it is approved on one of them (see
    /// `quince_auth::pcs`). Not a fault: the approval is asked for, and the service is read
    /// again once it is given.
    #[error("{APPROVAL_WORDS}")]
    NeedsDeviceApproval,
    #[error("this changed on another device first — refresh and try again")]
    Conflict,
    /// The zone was mid-write for someone else and the change was not applied; it is sent
    /// again (cloudkit's modify), and only reaches a person as Refused if it stays busy.
    #[error("iCloud is busy right now. Nothing was changed — try again in a minute.")]
    Busy,
    /// Apple turned a request down, said in words a person can act on. What Apple actually
    /// sent goes to the log, not the screen: a path and a JSON body told nobody anything.
    #[error("{0}")]
    Refused(String),
    /// Apple has no such record — it was deleted elsewhere, or never kept.
    #[error("that is no longer in iCloud")]
    NotFound,
    /// CloudKit would not read the cursor we held. Handled where the walk happens by starting
    /// over, so this should never reach a person.
    #[error("iCloud no longer recognises where we had got to")]
    StaleCursor,
    #[error(transparent)]
    Storage(#[from] quince_core::Error),
    #[error("couldn't write the file: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// What [`Error::NeedsDeviceApproval`] says to the person.
pub const APPROVAL_WORDS: &str =
    "Approve Skykeep on your iPhone, iPad or Mac to open this. Advanced Data Protection keeps it end-to-end encrypted.";

/// Whether Apple turned a request down only because Advanced Data Protection is keeping the
/// service's keys from this session — the two answers it gives, measured on an account with
/// it on and "Access iCloud Data on the Web" allowed:
///
///   * iCloud Drive: `423 {"errorReason":"Missing PCS cookies from the request","errorCode":423}`
///   * CloudKit (Notes, Reminders, Photos): `403 {"serverErrorCode":"ACCESS_DENIED",
///     "reason":"private db access disabled for this account"}`, the same inside a zone's
///     own result at 200.
///
/// The words are matched, never the status alone: Apple also answers 423 when it is rate
/// limiting, and 403 ACCESS_DENIED for an endpoint it retired (`push`, `courier`).
#[must_use]
pub fn locked_by_data_protection(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("missing pcs cookies")
        || (lower.contains("access_denied") && lower.contains("private db access disabled"))
}

/// Where Apple's refusals are told when the person has turned diagnostics on (see the app's
/// `diagnostics`): what was asked (`context`), the status, and Apple's answer — which the sink
/// boils down to its error codes before anything is written. Unset, nothing is told.
pub type DiagnosticsSink = fn(context: &str, status: u16, body: &str);
static DIAGNOSTICS: std::sync::OnceLock<DiagnosticsSink> = std::sync::OnceLock::new();

/// Turn diagnostics on for this crate's requests. Once per run; later calls change nothing.
pub fn set_diagnostics(sink: DiagnosticsSink) {
    let _ = DIAGNOSTICS.set(sink);
}

pub(crate) fn told(context: &str, status: reqwest::StatusCode, body: &str) {
    if let Some(sink) = DIAGNOSTICS.get() {
        sink(context, status.as_u16(), body);
    }
}

/// [`Error::NeedsDeviceApproval`] for an answer [`locked_by_data_protection`] recognises.
/// Every refusal passes through here on its way to an error, so it is also where one is told
/// to diagnostics.
pub(crate) fn approval_needed(status: reqwest::StatusCode, body: &str) -> Result<()> {
    told("refused", status, body);
    if locked_by_data_protection(body) {
        quiet!("icloud: waiting for Advanced Data Protection approval ({status})");
        return Err(Error::NeedsDeviceApproval);
    }
    Ok(())
}

/// What went wrong reaching Apple, in words a person can act on — the address left out.
fn network_words(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "iCloud took too long to answer. Check your connection and try again.".into()
    } else if e.is_connect() {
        "Couldn't reach iCloud. Check your connection and try again.".into()
    } else if e.is_body() || e.is_decode() {
        "iCloud's answer was cut off. Try again.".into()
    } else {
        "The connection to iCloud dropped. Try again.".into()
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;

    #[test]
    fn the_two_answers_advanced_data_protection_gives_are_recognised() {
        // As issue #4 reported them, word for word.
        let drive = r#"{"errorReason":"Missing PCS cookies from the request","errorCode":423}"#;
        let cloudkit = r#"{ "serverErrorCode" : "ACCESS_DENIED", "reason" : "private db access disabled for this account" }"#;
        assert!(locked_by_data_protection(drive));
        assert!(locked_by_data_protection(cloudkit));
        // Inside a zone's own result, which CloudKit answers at 200.
        assert!(locked_by_data_protection(&format!(
            r#"{{"zones":[{cloudkit}]}}"#
        )));
        assert!(matches!(
            approval_needed(reqwest::StatusCode::LOCKED, drive),
            Err(Error::NeedsDeviceApproval)
        ));
        assert!(matches!(
            approval_needed(reqwest::StatusCode::FORBIDDEN, cloudkit),
            Err(Error::NeedsDeviceApproval)
        ));
        assert_eq!(Error::NeedsDeviceApproval.to_string(), APPROVAL_WORDS);
    }

    #[test]
    fn look_alikes_are_not_read_as_advanced_data_protection() {
        // 423 is also how Apple says "too many requests"; ACCESS_DENIED is also what a retired
        // endpoint answers.
        assert!(!locked_by_data_protection(""));
        assert!(!locked_by_data_protection(r#"{"error":"Locked"}"#));
        assert!(!locked_by_data_protection(
            r#"{"serverErrorCode":"ACCESS_DENIED","reason":"This endpoint is deprecated."}"#
        ));
        assert!(!locked_by_data_protection(
            "<html>Service Unavailable</html>"
        ));
        assert!(approval_needed(reqwest::StatusCode::LOCKED, "{}").is_ok());
    }
}
