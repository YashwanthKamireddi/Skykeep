//! Apple ID sign-in, and the session that survives a restart.
//!
//! Two things make this different from every Electron iCloud wrapper:
//!
//! 1. **Auth is native.** Chromium splits WebAuthn across its render engine and its browser
//!    layer, and Electron ships only the former — so "Sign in with iPhone" hangs forever on
//!    Linux (electron/electron#24573, open since 2020). Doing SRP ourselves sidesteps that
//!    entirely; no webview is involved in authentication.
//! 2. **We keep the durable credential.** Apple hands back a trust token from `/2sv/trust`.
//!    A webview never asks for one, so it has nothing to persist and signs you out. We store
//!    it in the OS keyring.

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

pub mod diag;
pub mod keep;
pub mod pcs;
pub mod releases;
mod sealed;
pub mod session;
pub mod srp;
pub mod state;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
pub use keep::Kept;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client as HttpClient, Response, StatusCode};
use reqwest_cookie_store::CookieStoreMutex;
use serde_json::{json, Value};
use session::{backoff_seconds, unix_now, Session, CAPTURED_HEADERS};
pub use session::{use_platform_secrets, PlatformSecrets, Store};
use std::path::Path;
use std::sync::Arc;

/// Base URLs for Apple's auth and setup services.
///
/// Injectable so the sign-in flow can be exercised against a mock server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// Origin for the SRP sign-in calls.
    pub auth_root: String,
    /// `{auth_root}/appleauth/auth`
    pub auth: String,
    /// `.../setup/ws/1`
    pub setup: String,
    /// Origin for everything that is not SRP sign-in.
    pub home: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            auth_root: "https://idmsa.apple.com".into(),
            auth: "https://idmsa.apple.com/appleauth/auth".into(),
            setup: "https://setup.icloud.com/setup/ws/1".into(),
            home: "https://www.icloud.com".into(),
        }
    }
}

/// The domain Apple names for the accounts it keeps on its own servers apart from iCloud.com
/// (`domainToUse` in its answer to `accountLogin`), as it writes it.
pub const REGIONAL_DOMAIN: &str = "icloud.com.cn";

/// iCloud.com's own domain, as Apple names it in `domainToUse`.
const HOME_DOMAIN: &str = "icloud.com";

/// Where Apple's answer says this account is kept, when it says (`domainToUse`).
fn domain_to_use(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("domainToUse")?
        .as_str()
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty())
}

impl Endpoints {
    /// Apple's servers for an account Apple keeps at `domain` (its `domainToUse`, kept in the
    /// session): iCloud.com's for every account, unless Apple named its regional domain. The
    /// same services and protocols, at Apple's own hosts there (icloudpd's `--domain cn`).
    #[must_use]
    pub fn for_domain(domain: Option<&str>) -> Self {
        if domain.is_some_and(|d| d.eq_ignore_ascii_case(REGIONAL_DOMAIN)) {
            Self {
                auth_root: "https://idmsa.apple.com.cn".into(),
                auth: "https://idmsa.apple.com.cn/appleauth/auth".into(),
                setup: "https://setup.icloud.com.cn/setup/ws/1".into(),
                home: "https://www.icloud.com.cn".into(),
            }
        } else {
            Self::default()
        }
    }

    /// Point every service at one base URL. Test seam.
    pub fn at(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            auth_root: base.into(),
            auth: format!("{base}/appleauth/auth"),
            setup: format!("{base}/setup/ws/1"),
            home: base.into(),
        }
    }
}

/// iCloud web's first-party OAuth widget key. Not a secret — it is public in the web client.
const WIDGET_KEY: &str = "d39ba9916b7251055b22c7f910e2ea796ee65e98b2ddecea8f5dde8d9d1a815d";
/// Bounds on Apple's PBKDF2 iteration count. Apple sends each account's own: most see about
/// 20,000, but older accounts far fewer (806 for one, issue #8), and every other client takes
/// whatever comes. So any count is taken but none, which is no derivation at all; only the
/// ceiling guards something, a response that would have us burn the CPU. Apple chose the count
/// over TLS, so a low one is not something an attacker in the middle can ask for.
const MIN_PBKDF2_ITERATIONS: u32 = 1;
const MAX_PBKDF2_ITERATIONS: u32 = 1_000_000;

/// Version strings the iCloud web client sends. Cosmetic to Apple in practice, but sending
/// something plausible keeps our requests looking like the client they are imitating.
const CLIENT_BUILD: &str = "2523Project28";
const CLIENT_MASTERING: &str = "2523B28";

const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Apple didn't accept that Apple Account or password. Use your Apple Account password itself: an app-specific password can't sign in here.")]
    BadCredentials,
    #[error("Apple has locked this Apple Account for security. Unlock it at iforgot.apple.com, then sign in here again.")]
    AccountLocked,
    #[error("That code didn't match. Check your iPhone and try again.")]
    BadCode,
    #[error("Apple didn't return a trust token, so this session wouldn't survive a restart")]
    NoTrustToken,
    #[error("this account signs in a way Skykeep can't handle yet ({0})")]
    UnsupportedAccount(String),
    /// Apple wants something finished on the account before it signs in anywhere new.
    #[error("Apple needs one more step on your Apple Account first. Sign in at account.apple.com, finish what it asks, then sign in here again.")]
    RepairNeeded,
    /// Said without the address, which carries account and client ids.
    #[error("couldn't reach Apple: {}", if .0.is_timeout() { "it took too long to answer" } else if .0.is_connect() { "check your connection" } else { "the connection dropped" })]
    Network(#[from] reqwest::Error),
    #[error("Apple sent back something unexpected: {0}")]
    Protocol(String),
    /// Apple's servers answered but could not serve now — an outage, maintenance, too many
    /// requests (5xx, 429). Like being offline: try again later, never "signed out".
    #[error("Apple’s servers are busy right now. Try again in a few minutes.")]
    Unavailable(u16),
    #[error("Too many attempts. Try again in {seconds} seconds.")]
    CoolingDown { seconds: u64 },
    #[error("this needs a fresh sign-in — Apple won't accept a resumed session for it")]
    ReauthRequired,
    /// Apple keeps this account on its regional servers and this session has moved there, but
    /// Apple wants a sign-in made there: whoever holds the password signs in again (issue #10).
    #[error("Apple keeps this Apple Account on your region's iCloud servers. Sign in again to continue there.")]
    Moved,
    /// [`Error::Moved`] met after a code: signed in again there, and Apple sent a new code.
    #[error("Apple keeps this Apple Account on your region's iCloud servers, and has sent a new code for them. Enter the new code.")]
    CodeAgain,
    // Apple's 429 and its 503 on sign-in both mean "not now", whether from too many tries or
    // its own load: say what to do, not which of the two it was.
    #[error(
        "Apple asked to wait before signing in again. Try again in a few minutes: trying \
             again right away can make Apple lock the account for a while."
    )]
    RateLimited,
    /// Said as what to do: the keyring's own words (a D-Bus error name, a Win32 code) are for
    /// the log, and stay in the value.
    #[error("{}", keyring_words())]
    Keyring(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

type Result<T> = std::result::Result<T, Error>;

/// What a keyring that would not answer means to the person in front of it.
#[cfg(windows)]
pub const KEYRING_WORDS: &str = "Windows Credential Manager didn’t answer. Try again in a moment.";
#[cfg(not(windows))]
pub const KEYRING_WORDS: &str =
    "Your system keyring is locked or not running. Unlock or start GNOME Keyring or KWallet, then try again.";

/// In the Snap, the keyring is reached through the password-manager-service interface, which
/// snapd never connects by itself (its base declaration denies auto-connection). Unconnected,
/// every keyring call is refused whatever state the keyring is in, so "locked" would send the
/// person to the wrong fix.
#[cfg(not(windows))]
pub const SNAP_KEYRING_WORDS: &str = "Skykeep needs your permission to use the system keyring. In a terminal, run: sudo snap connect skykeep:password-manager-service — then try again.";

/// What a keyring that would not answer means here: [`KEYRING_WORDS`], or inside the Snap
/// [`SNAP_KEYRING_WORDS`].
pub fn keyring_words() -> &'static str {
    #[cfg(not(windows))]
    if std::env::var_os("SNAP_NAME").is_some_and(|n| n == "skykeep") {
        return SNAP_KEYRING_WORDS;
    }
    KEYRING_WORDS
}

/// A reply that says "not now" rather than "no": Apple's servers failing or overloaded.
fn transient(status: reqwest::StatusCode) -> Option<Error> {
    (status.is_server_error() || status.as_u16() == 429)
        .then(|| Error::Unavailable(status.as_u16()))
}

/// Apple returns `{"serviceErrors":[{"code":"-21669","message":"..."}]}` on failures.
/// Surface its own words; a bare status code explains nothing.
fn apple_error(context: &str, status: StatusCode, body: &str) -> Error {
    if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::SERVICE_UNAVAILABLE {
        return Error::RateLimited;
    }
    // 450 is Apple's "re-authenticate for this service": a resumed session is not enough.
    if status.as_u16() == 450 {
        return Error::ReauthRequired;
    }
    // An outage or maintenance: its body is a web page, never words for the person signing in.
    if status.is_server_error() {
        return Error::Unavailable(status.as_u16());
    }
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    if let Some(v) = &parsed {
        if let Some(errs) = v["serviceErrors"].as_array().filter(|a| !a.is_empty()) {
            let code = errs[0]["code"].as_str().unwrap_or("");
            let message = errs[0]["message"].as_str().unwrap_or("").trim();
            // -21669 is Apple's "incorrect verification code".
            if code == "-21669" {
                return Error::BadCode;
            }
            // -20209 is Apple's "locked for security reasons", most often after many tries.
            if code == "-20209" {
                return Error::AccountLocked;
            }
            if message.to_ascii_lowercase().contains("too many") {
                return Error::RateLimited;
            }
            if !message.is_empty() {
                return Error::Protocol(format!(
                    "{context}: {message} (Apple code {code}, HTTP {status})"
                ));
            }
        }
    }
    let snippet: String = body
        .chars()
        .take(220)
        .collect::<String>()
        .replace('\n', " ");
    Error::Protocol(format!("{context}: HTTP {status} {snippet}"))
}

/// Apple turning the password down (a 401, or a 403 before the password was even tried) —
/// unless its own code in the reply says why it really refused.
///
/// A locked account is a 403 or 401 with code -20209 (timlaing/pyicloud#368,
/// icloud_photos_downloader#1010). Read as a wrong password, a lapsed session signing itself
/// back in threw away the password it keeps, so the account was locked *and* signed out, and
/// the person was told to check a password that was right.
fn refused_password(context: &str, status: StatusCode, body: &str) -> Error {
    match apple_error(context, status, body) {
        e @ (Error::AccountLocked | Error::RateLimited) => e,
        _ => Error::BadCredentials,
    }
}

/// A phone Apple will send a code to, when no trusted device responds.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedPhone {
    pub id: u64,
    /// Already masked by Apple, e.g. "+91 ••••• ••123".
    pub masked_number: String,
    /// How Apple reaches this number: `sms`, or `voice` for a number it calls (a landline).
    /// Sent back as it came (timlaing/pyicloud#210): Apple takes a code for a number the way
    /// it sends one there.
    pub mode: String,
    /// Apple's `nonFTEU` flag on the number, sent back with it when Apple gave one.
    #[serde(skip)]
    pub non_fteu: Option<bool>,
}

impl TrustedPhone {
    /// The `phoneNumber` Apple's phone endpoints take for this number.
    fn as_payload(&self) -> Value {
        let mut phone = json!({ "id": self.id });
        if let Some(flag) = self.non_fteu {
            phone["nonFTEU"] = Value::Bool(flag);
        }
        phone
    }
}

/// What Apple's `GET /appleauth/auth` answered, as JSON: the reply itself, or — when Apple
/// sends its sign-in page instead — the JSON that page carries in its `boot_args` script
/// (timlaing/pyicloud#322).
fn auth_options_of(text: &str) -> Value {
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return v;
    }
    let Some(at) = text.find("boot_args") else {
        return Value::Null;
    };
    let rest = &text[at..];
    rest.find('>')
        .map(|open| &rest[open + 1..])
        .and_then(|body| body.find("</script").map(|end| &body[..end]))
        .and_then(|json| serde_json::from_str(json.trim()).ok())
        .unwrap_or(Value::Null)
}

/// Every place Apple has kept the two-factor details in that reply: at the top, and — since
/// its 2026 sign-in — under `phoneNumberVerification`, `direct.twoSV`, and
/// `direct.twoSV.bridgeInitiateData.phoneNumberVerification` (icloud_photos_downloader#1335,
/// timlaing/pyicloud#322).
fn two_factor_places(v: &Value) -> Vec<&Value> {
    let two_sv = v.pointer("/direct/twoSV");
    [
        Some(v),
        v.get("phoneNumberVerification"),
        v.get("direct"),
        // The security-key answer's second layout (`fsaVerification.fsaChallenge`, seen by
        // FindMy.py; docs/security-keys.md): read as well as the top-level one.
        v.get("fsaVerification"),
        two_sv,
        two_sv.and_then(|t| t.get("phoneNumberVerification")),
        two_sv.and_then(|t| t.pointer("/bridgeInitiateData/phoneNumberVerification")),
    ]
    .into_iter()
    .flatten()
    .filter(|p| p.is_object())
    .collect()
}

/// The trusted phones in Apple's two-factor reply, wherever it kept them, each once.
///
/// Read only at the top, an account whose numbers Apple had moved inward showed "Apple has
/// nowhere to send a code" — for someone with no other device, no way to sign in at all.
fn trusted_phones(v: &Value) -> Vec<TrustedPhone> {
    let mut phones: Vec<TrustedPhone> = Vec::new();
    for place in two_factor_places(v) {
        let listed = place
            .get("trustedPhoneNumbers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .chain(place.get("trustedPhoneNumber").filter(|p| p.is_object()));
        for p in listed {
            let id = p["id"]
                .as_u64()
                .or_else(|| p["id"].as_str().and_then(|s| s.trim().parse().ok()));
            let Some(id) = id else { continue };
            if phones.iter().any(|known| known.id == id) {
                continue;
            }
            phones.push(TrustedPhone {
                id,
                masked_number: p["numberWithDialCode"]
                    .as_str()
                    .or_else(|| p["obfuscatedNumber"].as_str())
                    .unwrap_or("(hidden)")
                    .to_string(),
                mode: p["pushMode"]
                    .as_str()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .unwrap_or("sms")
                    .to_string(),
                non_fteu: p["nonFTEU"].as_bool(),
            });
        }
    }
    phones
}

/// Where Apple is willing to send a code. Read after a `NeedsTwoFactor` outcome.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorOptions {
    /// Devices Apple pushed the code to. Zero means nothing will arrive on its own.
    pub trusted_device_count: u64,
    pub phones: Vec<TrustedPhone>,
    /// Apple refuses further sends for a while.
    pub too_many_codes_sent: bool,
    /// The account signs in with security keys (FIDO): Apple offers a WebAuthn challenge
    /// (`fsaChallenge`) in place of a code (icloudpd#583, #1423). Skykeep cannot answer one yet,
    /// so the code screen says so rather than waiting for a code that will not come.
    pub security_key: bool,
}

/// Whether Apple's two-factor answer offers a security-key challenge: `fsaChallenge` with its
/// `challenge`, `keyHandles` and `rpId`, as icloudpd reads it.
fn offers_security_key(v: &Value) -> bool {
    let c = &v["fsaChallenge"];
    c.is_object()
        && ["challenge", "keyHandles", "rpId"].iter().all(|k| {
            let x = &c[*k];
            !x.is_null() && x.as_str() != Some("") && x.as_array().is_none_or(|a| !a.is_empty())
        })
}

/// Where an app goes when it opens with what it kept ([`Client::reopen`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reopen {
    /// Signed in (or offline with a session kept): the library.
    Library,
    /// Apple wants a code before it trusts this device again.
    Code,
    /// Nothing kept, or Apple refused what was: sign in.
    SignIn,
}

/// What `sign_in` decided.
#[derive(Debug, PartialEq, Eq)]
pub enum SignInOutcome {
    /// Signed in. Apple already trusted this install.
    Complete,
    /// Apple wants a six-digit code. Call [`Client::submit_two_factor`] next.
    NeedsTwoFactor,
}

/// Apple's SRP challenge, from `/signin/init` or `/escrow/init`.
struct Challenge {
    salt: Vec<u8>,
    b_pub: Vec<u8>,
    c: String,
    iterations: u32,
    protocol: srp::Protocol,
}

impl Challenge {
    fn of(init: &Value) -> Result<Self> {
        let decode = |key: &str| -> Result<Vec<u8>> {
            let raw = init[key]
                .as_str()
                .ok_or_else(|| Error::Protocol(format!("sign-in response had no `{key}`")))?;
            B64.decode(raw)
                .map_err(|_| Error::Protocol(format!("`{key}` wasn't valid base64")))
        };
        let salt = decode("salt")?;
        let b_pub = decode("b")?;
        let c = init["c"]
            .as_str()
            .ok_or_else(|| Error::Protocol("sign-in response had no `c`".into()))?
            .to_string();
        let iterations = {
            let raw = init["iteration"]
                .as_u64()
                .ok_or_else(|| Error::Protocol("sign-in response had no `iteration`".into()))?;
            u32::try_from(raw)
                .ok()
                .filter(|n| (MIN_PBKDF2_ITERATIONS..=MAX_PBKDF2_ITERATIONS).contains(n))
                .ok_or_else(|| {
                    Error::Protocol(format!(
                        "Apple asked for an implausible {raw} key-derivation rounds"
                    ))
                })?
        };
        let protocol_name = init["protocol"].as_str().unwrap_or("s2k");
        let protocol = srp::Protocol::parse(protocol_name)
            .ok_or_else(|| Error::UnsupportedAccount(format!("SRP protocol {protocol_name}")))?;
        Ok(Self {
            salt,
            b_pub,
            c,
            iterations,
            protocol,
        })
    }
}

/// Whether Apple wants its escrow step: `X-Apple-EDP` or `X-Apple-PDP` on a 409 (icloud-photos-
/// sync#1124, rclone#10048). Since iOS 26.4 Apple asks for it after an accepted code, and when
/// a sign-in presents a trust token it accepts — and until it is done it does not honour the
/// trust token, so every later sign-in looks as if it needs a code again.
fn wants_escrow(response: &Response) -> bool {
    let h = response.headers();
    h.contains_key("x-apple-edp") || h.contains_key("x-apple-pdp")
}

/// Whether a sign-in that Apple wants a code for asks Apple to send it now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Code {
    /// The person just typed their password and the code screen is next: ask.
    Send,
    /// Nobody is watching (a lapse mended in the background): leave it to the code screen.
    Wait,
}

/// OAuth headers every idmsa call carries. Deliberately sets no Origin: the HTTP client
/// defaults to the icloud.com origin, which is what the verify, trust and phone endpoints
/// expect. Only SRP overrides it — see [`with_srp_origin`].
/// Two secrets compared without the time taken saying how much of one matched.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn base_oauth_headers(
    client_id: &str,
    home: &str,
    scnt: Option<&str>,
    session_id: Option<&str>,
) -> HeaderMap {
    let mut h = HeaderMap::new();
    let mut put = |k: &'static str, v: &str| {
        if let Ok(val) = HeaderValue::from_str(v) {
            h.insert(HeaderName::from_static(k), val);
        }
    };
    put("accept", "application/json, text/javascript, */*; q=0.01");
    put("content-type", "application/json");
    put("x-apple-oauth-client-id", WIDGET_KEY);
    put("x-apple-oauth-client-type", "firstPartyAuth");
    put("x-apple-oauth-redirect-uri", home);
    put("x-apple-oauth-require-grant-code", "true");
    put("x-apple-oauth-response-mode", "web_message");
    put("x-apple-oauth-response-type", "code");
    put("x-apple-oauth-state", client_id);
    put("x-apple-widget-key", WIDGET_KEY);
    if let Some(v) = scnt {
        put("scnt", v);
    }
    if let Some(v) = session_id {
        put("x-apple-id-session-id", v);
    }
    h
}

/// `/signin/init` and `/signin/complete` are served from idmsa and reject the icloud.com
/// origin. Applying this anywhere else earns a 409.
fn with_srp_origin(mut h: HeaderMap, auth_root: &str) -> HeaderMap {
    if let Ok(v) = HeaderValue::from_str(auth_root) {
        h.insert("origin", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{auth_root}/")) {
        h.insert("referer", v);
    }
    h
}

pub struct Client {
    http: HttpClient,
    jar: Arc<CookieStoreMutex>,
    store: Store,
    session: Session,
    endpoints: Endpoints,
    /// iCloud.com's servers, and Apple's regional ones: where [`Client::switch_servers`] goes.
    home_endpoints: Endpoints,
    regional_endpoints: Endpoints,
    /// A sign-in in flight: its password, kept only once the sign-in completes, and the code
    /// Apple wants for it (see [`state::SignIn`]).
    sign: state::SignIn,
    /// The keyring would not answer when this started — locked, or not up yet. Not the same
    /// as nothing stored: it is asked again before anything else happens.
    keyring_error: Option<String>,
    /// The session signed in, or taken up, but the keyring would not keep it (none running:
    /// a window manager without one, a session over SSH): held in memory for as long as
    /// Skykeep runs, and written again at every heartbeat until a keyring takes it.
    unsaved: Option<String>,
    /// What was last written, as a hash of the session and one of the cookies: a heartbeat that
    /// changed neither writes neither (see [`Client::persist`]).
    persisted: std::sync::Mutex<Persisted>,
    /// The cookies on disk, as this start found them (see [`read_jar`]).
    cookies: Cookies,
    /// The trusted phones Apple last listed, so a code is sent to and checked for each number
    /// the way that number takes one.
    phones: Vec<TrustedPhone>,
    /// What this run knows of Advanced Data Protection approvals (see [`pcs`]).
    pcs: pcs::Pcs,
    /// What the passes' session checks remember between them (see [`keep`]).
    keeping: keep::Keeping,
}

/// What became of the cookies kept on disk when the client started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cookies {
    /// Read, or there were none.
    Read,
    /// Sealed, and the keyring would not answer for their key — locked, or not up yet. The file
    /// is left as it is and read again once the keyring answers.
    Unread,
    /// Sealed, and could not be opened: the keyring no longer holds their key (it was reset),
    /// or the file was changed. Set aside; the session carries on from its tokens.
    SetAside,
}

/// The cookie jar kept on disk, opened, and what became of the file.
///
/// A sealed file is opened with its key from the keyring. One that cannot be — no key, a key
/// that does not fit, a version this build does not know, a changed byte — is deleted, never
/// a reason to fail: the cookies only save a round trip, and the tokens the keyring holds open
/// a new set (see [`Client::resume`]).
///
/// A jar from before sealing (plain JSON) is read once and sealed in its place, or, with no
/// keyring to keep a key, deleted — its cookies held in memory for as long as Skykeep runs and
/// sealed by the first write a keyring takes. Either way no plaintext is left behind: the sealed
/// file is renamed over it, so the plain one is never there beside it.
fn read_jar(store: &Store) -> (cookie_store::CookieStore, Cookies) {
    let path = store.cookie_path();
    let parse = |json: &[u8]| cookie_store::serde::json::load(json).ok();
    let Ok(bytes) = std::fs::read(path) else {
        return (cookie_store::CookieStore::default(), Cookies::Read);
    };
    let set_aside = |why: &str| {
        quiet!("quince: the saved cookies were set aside: {why}");
        let _ = std::fs::remove_file(path);
        (cookie_store::CookieStore::default(), Cookies::SetAside)
    };
    if sealed::is_sealed(&bytes) {
        return match store.cookie_key() {
            Ok(Some(key)) => match sealed::open(&key, &bytes) {
                Ok(json) => match parse(&json) {
                    Some(jar) => (jar, Cookies::Read),
                    None => set_aside("they were not a cookie jar"),
                },
                Err(sealed::Unopened::Version(v)) => set_aside(&format!("unknown version {v}")),
                Err(sealed::Unopened::Damaged) => set_aside("they did not open with their key"),
            },
            Ok(None) => set_aside("the keyring no longer holds their key"),
            Err(e) => {
                quiet!("quince: the saved cookies wait for the keyring: {e}");
                (cookie_store::CookieStore::default(), Cookies::Unread)
            }
        };
    }
    // From before sealing: read once, then sealed in place or gone.
    let Some(jar) = parse(&bytes) else {
        let _ = std::fs::remove_file(path);
        return (cookie_store::CookieStore::default(), Cookies::Read);
    };
    let sealed_in_place = (|| -> Result<()> {
        let key = store.cookie_key_or_new()?;
        let json = jar_bytes(&jar)?;
        Ok(session::write_whole(
            path,
            &sealed::seal(&key, &json)?,
            true,
        )?)
    })();
    if let Err(e) = sealed_in_place {
        quiet!("quince: the cookies are kept in memory only: {e}");
        let _ = std::fs::remove_file(path);
    }
    (jar, Cookies::Read)
}

/// The jar as it is written to disk: every cookie that has not run out, Apple's session
/// cookies among them.
///
/// `cookie_store`'s own `save` keeps only cookies with an expiry, and Apple sets several of the
/// ones its sign-in leans on without one — idmsa's own, and some of icloud.com's. Each start
/// came back without them, so the next sign-in was a stranger's and Apple asked for a code
/// again, about every day. pyicloud and icloudpd keep them the same way (`LWPCookieJar.save(
/// ignore_discard=True)`), as a browser that restores its session does. They are a credential
/// like the rest of the jar, and sealed with it (see [`sealed`]).
fn jar_bytes(jar: &cookie_store::CookieStore) -> Result<Vec<u8>> {
    let cookies: Vec<&cookie_store::Cookie<'static>> = jar.iter_unexpired().collect();
    let mut out = serde_json::to_vec_pretty(&cookies)
        .map_err(|e| Error::Protocol(format!("couldn't save cookies: {e}")))?;
    out.push(b'\n');
    Ok(out)
}

/// Hashes of the session and the cookie jar as last written; `None` for not written yet.
#[derive(Default)]
struct Persisted {
    session: Option<u64>,
    cookies: Option<u64>,
}

fn hash_of(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Apple's client for an account at `endpoints`: their web origin on every request (what
/// setup.icloud.com and the service endpoints expect; the idmsa sign-in calls name their own),
/// the cookie jar the session keeps, and the rest every connection to Apple gets.
fn apple_client(endpoints: &Endpoints, jar: &Arc<CookieStoreMutex>) -> Result<HttpClient> {
    // The account's web origin — what setup and the service endpoints expect. The idmsa SRP
    // endpoints want their own origin and override it per request.
    let mut defaults = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(&endpoints.home) {
        defaults.insert("origin", v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("{}/", endpoints.home)) {
        defaults.insert("referer", v);
    }
    defaults.insert(
        "accept-language",
        HeaderValue::from_static("en-US,en;q=0.9"),
    );

    // Never waits for ever. After a sleep or a change of network a pooled connection can
    // be dead without saying so, and a request on it hung until the kernel gave up — a
    // quarter of an hour or more — holding the sign-in lock, so every screen waited too.
    // A read that brings nothing for a minute is given up (a long download keeps going as
    // long as bytes arrive); idle connections are let go of before they can go stale; and
    // keep-alives notice a dead one.
    apple_only()
        // The jar itself, telling diagnostics (when the person turned them on) which
        // cookies go out and come back, by name.
        .cookie_provider(Arc::new(diag::Told(Arc::clone(jar))))
        .user_agent(UA)
        .default_headers(defaults)
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(60))
        .pool_idle_timeout(std::time::Duration::from_secs(50))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .build()
        .map_err(Error::from)
}

impl Client {
    pub fn new(state_dir: &Path) -> Result<Self> {
        Self::with(state_dir, Endpoints::default(), Store::keyring(state_dir)?)
    }

    /// Build a client against specific endpoints and a specific secret store.
    pub fn with(state_dir: &Path, endpoints: Endpoints, store: Store) -> Result<Self> {
        std::fs::create_dir_all(state_dir)?;
        let (session, keyring_error) = match store.try_load() {
            Ok(session) => (session, None),
            Err(e) => (session::Session::default(), Some(e.to_string())),
        };

        let (jar, cookies) = read_jar(&store);
        let jar = Arc::new(CookieStoreMutex::new(jar));

        // The account's own servers: iCloud.com's, or Apple's regional ones for an account
        // Apple keeps there (issue #10), unless a test pointed the client somewhere itself.
        let endpoints = if endpoints == Endpoints::default() {
            Endpoints::for_domain(session.domain.as_deref())
        } else {
            endpoints
        };
        open_regional(endpoints == Endpoints::for_domain(Some(REGIONAL_DOMAIN)));
        let http = apple_client(&endpoints, &jar)?;

        // Where switch_servers goes: a test's mock for both, unless it names another for the
        // regional servers; Apple's own otherwise.
        let home_endpoints = if endpoints == Endpoints::for_domain(Some(REGIONAL_DOMAIN)) {
            Endpoints::default()
        } else {
            endpoints.clone()
        };
        let regional_endpoints = if home_endpoints == Endpoints::default() {
            Endpoints::for_domain(Some(REGIONAL_DOMAIN))
        } else {
            home_endpoints.clone()
        };
        Ok(Self {
            http,
            jar,
            store,
            session,
            endpoints,
            home_endpoints,
            regional_endpoints,
            sign: state::SignIn::Idle,
            keyring_error,
            unsaved: None,
            persisted: std::sync::Mutex::new(Persisted::default()),
            cookies,
            phones: Vec::new(),
            pcs: pcs::Pcs::default(),
            keeping: keep::Keeping::default(),
        })
    }

    /// Said when the cookies kept on disk could not be opened and were set aside — the keyring
    /// was reset, or the file changed. Nothing else is lost: Skykeep carries on from the
    /// session the keyring holds, and asks for a sign-in only if that is gone too.
    #[must_use]
    pub fn cookies_set_aside(&self) -> Option<&'static str> {
        (self.cookies == Cookies::SetAside).then_some(
            "Skykeep couldn't open its saved connection to iCloud, so it set it aside. \
             If you're asked to sign in again, that's all it takes — your library is untouched.",
        )
    }

    /// Cookies left unread at the start because the keyring would not answer, read now that it
    /// may — into a jar nothing has filled since, never over cookies newer than the file.
    fn read_cookies_again(&mut self) {
        if self.cookies != Cookies::Unread {
            return;
        }
        let (jar, cookies) = read_jar(&self.store);
        if cookies == Cookies::Unread {
            return;
        }
        self.cookies = cookies;
        let mut held = self
            .jar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.iter_any().next().is_none() {
            *held = jar;
        }
    }

    /// A lapsed session tried to sign itself back in, and Apple wants a code before it will
    /// trust this computer again. The code screen is the next thing to show.
    #[must_use]
    pub fn awaiting_code(&self) -> bool {
        self.sign.awaiting_code()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// True when a stored session exists and is worth trying.
    ///
    /// A keyring that would not answer at the start (locked, or not up yet at login) may well
    /// hold one: sealed cookies on disk say a keyring kept a session here before. Then it is
    /// worth a resume, which asks the keyring again — and where it is locked, that is what
    /// brings up its unlock prompt — rather than a sign-in screen that would replace it.
    pub fn has_stored_session(&self) -> bool {
        self.session.is_resumable()
            || (self.keyring_error.is_some() && self.store.cookie_path().exists())
    }

    // ---- persistence ----------------------------------------------------------------

    fn capture(&mut self, response: &Response) {
        diag::answer(response.url(), response.status());
        for (header, field) in CAPTURED_HEADERS {
            if let Some(v) = response
                .headers()
                .get(*header)
                .and_then(|v| v.to_str().ok())
            {
                self.session.set(*field, v.to_string());
            }
        }
    }

    /// Keep the session and its cookies where the next start finds them — each only when it
    /// differs from what was last written.
    ///
    /// Every heartbeat, so every sync and every return to the window, wrote the session to the
    /// keyring: several D-Bus writes, deletes of old names and a read-back, all while holding
    /// the sign-in lock, for a session that had not changed. Only a write that went through is
    /// remembered, so one that failed is made again next time.
    fn persist(&self) -> Result<()> {
        let session = serde_json::to_vec(&self.session)
            .map_err(|e| Error::Protocol(format!("couldn't save the session: {e}")))?;
        let cookies = {
            let jar = self
                .jar
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            jar_bytes(&jar)?
        };
        let (session, cookies) = (hash_of(&session), (hash_of(&cookies), cookies));
        let mut last = self
            .persisted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if last.session != Some(session) {
            self.store.save(&self.session)?;
            last.session = Some(session);
        }
        if last.cookies != Some(cookies.0) {
            self.write_cookies(&cookies.1)?;
            last.cookies = Some(cookies.0);
        }
        Ok(())
    }

    /// [`Self::persist`] once Apple has accepted the session: a keyring that will not take it
    /// never undoes that.
    ///
    /// With no Secret Service at all, the password went through, then the write failed and
    /// the sign-in failed with it — before the code was even asked for — so a machine without
    /// a keyring could never sign in, not just never stay signed in. The session is kept in
    /// memory instead, said so ([`Self::unsaved`]), and written again on the next heartbeat.
    /// A live session in memory is newer than anything the keyring holds, so a keyring that
    /// would not answer at the start is not asked to replace it either.
    fn keep(&mut self) {
        self.keyring_error = None;
        // Never an empty jar written over cookies the keyring could not open at the start.
        self.read_cookies_again();
        match self.persist() {
            Ok(()) => self.unsaved = None,
            Err(e) => {
                quiet!("quince: the session is kept in memory only: {e:?}");
                self.unsaved = Some(e.to_string());
            }
        }
    }

    /// Why the session is held in memory only, when a keyring would not keep it: Skykeep
    /// stays signed in while it runs, but not after it closes.
    #[must_use]
    pub fn unsaved(&self) -> Option<&str> {
        self.unsaved.as_deref()
    }

    /// The cookie jar, as `cookie_store` wrote it, sealed to its file (see [`sealed`]).
    ///
    /// With no keyring to keep the key nothing is written: the cookies stay in memory, as the
    /// session does, and are sealed by the first write a keyring takes. Never in the clear.
    fn write_cookies(&self, cookies: &[u8]) -> Result<()> {
        let key = self.store.cookie_key_or_new()?;
        // The session's cookies are a credential: readable by this user alone as well, whatever
        // folder they end up in (a backup, a copy), not only by the private data folder.
        Ok(session::write_whole(
            self.store.cookie_path(),
            &sealed::seal(&key, cookies)?,
            true,
        )?)
    }

    /// Somebody else signing in (Apple ended the last session, or it was never signed out):
    /// the last account's trust token, cookies and client id go first, as at sign-out, so
    /// nothing of it is sent with this sign-in and Apple never sees the two accounts tied
    /// together. The same account typed again (any case, any spacing) keeps them, and with them
    /// skips the code.
    fn forget_other_account(&mut self, apple_id: &str) -> Result<()> {
        let other = self.session.apple_id.as_deref().is_some_and(|before| {
            session::account_fingerprint(before) != session::account_fingerprint(apple_id)
        });
        if other {
            self.sign_out()?;
        }
        Ok(())
    }

    /// Forget this account entirely.
    pub fn sign_out(&mut self) -> Result<()> {
        self.session = Session::default();
        // The next account starts at iCloud.com, and the regional domains close again. Its
        // trouble is told after the store is cleared, never instead of clearing it.
        let switched = self.switch_servers(None);
        self.data_access_forget();
        if let Ok(mut jar) = self.jar.lock() {
            jar.clear();
        }
        // Nothing is stored any more: the next session is written whatever it is.
        *self
            .persisted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Persisted::default();
        self.store.clear().and(switched)
    }

    // ---- headers --------------------------------------------------------------------

    fn auth_headers(&mut self) -> HeaderMap {
        let client_id = self.session.client_id();
        base_oauth_headers(
            &client_id,
            &self.endpoints.home,
            self.session.scnt.as_deref(),
            self.session.session_id.as_deref(),
        )
    }

    /// Headers for `/signin/init` and `/signin/complete` only.
    ///
    /// Those two are served from idmsa and reject the icloud.com origin. Every other auth
    /// endpoint — verify, trust, phone — wants the icloud.com origin the client defaults to.
    fn srp_headers(&mut self) -> HeaderMap {
        let h = self.auth_headers();
        with_srp_origin(h, &self.endpoints.auth_root)
    }

    /// Headers for `/signin/init` specifically.
    ///
    /// Deliberately omits `scnt` and the session id: the flow has not issued them yet, and
    /// sending leftovers from an abandoned challenge is a conflict.
    fn srp_init_headers(&mut self) -> HeaderMap {
        let client_id = self.session.client_id();
        let h = base_oauth_headers(&client_id, &self.endpoints.home, None, None);
        with_srp_origin(h, &self.endpoints.auth_root)
    }

    /// Drop transient challenge state, keeping what identifies this install.
    ///
    /// `trust_token` and `client_id` survive — they are what lets Apple skip 2FA next time.
    /// Everything else belongs to a single sign-in attempt and must not leak into the next.
    fn clear_challenge_state(&mut self) {
        self.session.scnt = None;
        self.session.session_id = None;
        self.session.session_token = None;
    }

    // ---- the restart test -----------------------------------------------------------

    /// Where an app opening with what it kept should go: the library, Apple's code screen, or
    /// sign-in. Offline (or Apple busy, or asking to wait) with a session kept is the library,
    /// as a local-first app opens: the next refresh signs back in once Apple answers. Only
    /// Apple refusing the session, or nothing kept, is sign-in. The desktop and the phone both
    /// open through this.
    pub async fn reopen(&mut self) -> Result<Reopen> {
        let resumed = match self.resume().await {
            Ok(resumed) => resumed,
            Err(
                Error::Network(_)
                | Error::Unavailable(_)
                | Error::RateLimited
                | Error::CoolingDown { .. },
            ) if self.has_stored_session() => true,
            Err(e) => return Err(e),
        };
        Ok(if resumed {
            Reopen::Library
        } else if self.awaiting_code() {
            Reopen::Code
        } else {
            Reopen::SignIn
        })
    }

    /// Try to resume a stored session without a password.
    ///
    /// Returns `Ok(true)` when the stored session still works — the behaviour the whole
    /// product promises. `Ok(false)` means Apple rejected it and a full sign-in is needed;
    /// that is an expected outcome (roughly every 60 days), not a failure.
    pub async fn resume(&mut self) -> Result<bool> {
        // A sign-out the keyring was locked for is never undone by resuming: its entries are
        // erased now if it answers, and until they are, `try_load` (below, and at startup)
        // reads nothing stored. A session signed in since lives in memory and stays.
        if self.store.sign_out_pending() {
            if let Err(e) = self.store.finish_sign_out() {
                quiet!("quince: keyring: sign-out still waiting on it: {e:?}");
            }
        }
        // A keyring that would not answer at startup is asked again; still locked, that is
        // what is said — never "signed out", which led to a sign-in replacing what it holds.
        if self.keyring_error.is_some() {
            match self.store.try_load() {
                Ok(session) => {
                    self.session = session;
                    self.keyring_error = None;
                }
                Err(e) => {
                    quiet!("quince: keyring: still not answering: {e:?}");
                    return Err(e);
                }
            }
        }
        self.read_cookies_again();
        if !self.session.is_resumable() {
            return Ok(false);
        }
        // Cheap path: the cookies are still good, so nothing needs re-establishing.
        let validated = match self.validate().await {
            Ok(data) => {
                self.absorb(&data);
                self.keep();
                return Ok(true);
            }
            Err(e) => e,
        };

        // The cookies are missing or stale but the tokens are not. Apple's service endpoints
        // authenticate by cookie — without X-APPLE-WEBAUTH-USER they answer 421 — so trade
        // the stored tokens for a fresh set. No password, no code.
        //
        // This is what makes staying signed in survive a lost cookie file, a copied install,
        // or cookies expiring before the trust token does.
        let logged_in = match self.account_login().await {
            Ok(data) => {
                self.absorb(&data);
                self.keep();
                return Ok(true);
            }
            // New iCloud terms waiting are not a lapse: `absorb` keeps Apple's flag and the
            // session stays, so nothing signs in again with the password into the same terms.
            Err(e) => e,
        };
        // Unreachable is not rejected. Offline — no network yet after login, or a resume
        // from sleep mid-reconnect — the session is left exactly as it is and the caller
        // told so; signing in again here cleared the session token, failed on the network,
        // and saved the cleared state, so every launch after showed the sign-in screen.
        // Offline, or Apple down for a while: the session is not dead, only unreachable — it
        // is kept and tried again, rather than a sign-in that ends in "Apple signed you out".
        let away = |e: &Error| matches!(e, Error::Network(_) | Error::Unavailable(_));
        if away(&validated) && away(&logged_in) {
            return Err(logged_in);
        }

        // The session token has lapsed too — Apple keeps one for days, not months. With the
        // password kept, sign in again: this computer is still trusted, so Apple asks for no
        // code, and nobody sees a sign-in screen. Only when the trust itself has run out
        // (about two months) does Apple want a code, and then the code screen is what shows.
        if let (Some(apple_id), Some(password)) =
            (self.session.apple_id.clone(), self.session.password.clone())
        {
            let before = self.session.clone();
            // Quietly: a code is asked for only once the person is at the code screen. Asked
            // for here, every lapse — the heartbeat's, a sync's, a launch at login — put an
            // "Allow sign-in?" and a code on their phone while they were nowhere near Skykeep.
            match self.sign_in_as(&apple_id, &password, Code::Wait).await {
                Ok(SignInOutcome::Complete) => return Ok(true),
                // Lost the network part way, or Apple busy, down or asking to wait: not a
                // refusal. What was kept is put back as it was and the next try is left to
                // later — read as signed out, an outage told the person "Apple signed you
                // out" and dropped the session token.
                Err(
                    e @ (Error::Network(_)
                    | Error::Unavailable(_)
                    | Error::RateLimited
                    | Error::CoolingDown { .. }),
                ) => {
                    self.session = before;
                    let _ = self.persist();
                    return Err(e);
                }
                Ok(SignInOutcome::NeedsTwoFactor) => {
                    self.sign.wants_code(true);
                    return Ok(false);
                }
                // The password was changed elsewhere: the kept one is worthless now.
                Err(Error::BadCredentials) => {
                    self.session.password = None;
                    self.keep();
                }
                Err(_) => {}
            }
        }

        // Genuinely dead. Drop the challenge state so it cannot contaminate the sign-in that
        // follows.
        self.clear_challenge_state();
        Ok(false)
    }

    /// `POST /validate` — cheap liveness check. Doubles as the session heartbeat.
    async fn validate(&mut self) -> Result<Value> {
        let r = self
            .http
            .post(format!("{}/validate", self.endpoints.setup))
            .header("content-type", "application/json")
            .body("null")
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(transient(status)
                .unwrap_or_else(|| Error::Protocol(format!("validate returned {status}"))));
        }
        let data = serde_json::from_str(&body).unwrap_or(Value::Null);
        diag::account("validate", &data);
        Ok(data)
    }

    /// Keep the session warm.
    ///
    /// Apple resets auth cookies on long idle gaps; icloudpd's field experience is that
    /// roughly hourly contact keeps them alive. Cheap enough to run on a timer.
    pub async fn heartbeat(&mut self) -> Result<bool> {
        match self.validate().await {
            Ok(_) => {
                self.keep();
                Ok(true)
            }
            // Cookies lapse after an idle spell long before the session does. Giving up here
            // left a running window signed out for good — every refresh failing quietly —
            // when the stored tokens, or the kept password, would have carried on.
            Err(_) => self.resume().await,
        }
    }

    // ---- sign-in --------------------------------------------------------------------

    /// Sign in with an Apple ID and password, over SRP.
    ///
    /// The password is never sent to Apple — SRP proves knowledge of it without transmitting
    /// it. Apple retired the old cleartext endpoint; it now answers 503.
    pub async fn sign_in(&mut self, apple_id: &str, password: &str) -> Result<SignInOutcome> {
        self.sign_in_as(apple_id, password, Code::Send).await
    }

    /// [`Self::sign_in`], saying whether a code Apple wants is asked for now.
    async fn sign_in_as(
        &mut self,
        apple_id: &str,
        password: &str,
        code: Code,
    ) -> Result<SignInOutcome> {
        self.forget_other_account(apple_id)?;
        // What this account already owes, from beside the cookies rather than from the
        // keyring — so a machine with no keyring still remembers, which is where forgetting
        // would matter most. Per account, so signing in as somebody else inherits nothing.
        let owed = self.store.cooldown_for(apple_id);
        self.session.consecutive_failures = owed.consecutive_failures;
        self.session.retry_after = owed.retry_after;

        // Kept only once Apple has accepted it all the way: see `absorb`.
        self.sign.start(apple_id, password);
        let mut outcome = self.sign_in_inner(apple_id, password, code).await;
        // Apple keeps the account on its regional servers and wants the sign-in made there:
        // made again there, once — this session now starts at Apple's servers for it.
        if matches!(outcome, Err(Error::Moved)) {
            self.sign.start(apple_id, password);
            outcome = self.sign_in_inner(apple_id, password, code).await;
        }
        if outcome.is_err() {
            self.sign.fail();
        }
        match &outcome {
            // Reaching the two-factor step means the password was accepted: not a failure.
            Ok(_) => {
                self.session.consecutive_failures = 0;
                self.session.retry_after = None;
            }
            // Only Apple turning the attempt down counts. Not reaching Apple, or this computer
            // failing afterwards (a keyring that would not take the session), is no wrong
            // password: counted, it held a person back from trying again once it was fixed.
            Err(
                Error::CoolingDown { .. }
                | Error::Network(_)
                | Error::Keyring(_)
                | Error::Io(_)
                | Error::Json(_)
                | Error::RepairNeeded
                | Error::Moved
                | Error::CodeAgain,
            ) => {}
            Err(e) => {
                self.session.consecutive_failures =
                    self.session.consecutive_failures.saturating_add(1);
                // Apple saying "slow down" outranks our own schedule.
                let wait = if matches!(e, Error::RateLimited) {
                    backoff_seconds(u32::MAX)
                } else {
                    backoff_seconds(self.session.consecutive_failures)
                };
                self.session.retry_after = (wait > 0).then(|| unix_now() + wait);
            }
        }
        // Written on its own and first. `persist` needs the keyring and is allowed to fail;
        // the wait must be remembered either way, or a machine without a keyring forgets it
        // every time the window closes and somebody can go on hammering Apple until the
        // account they are trying to reach is locked.
        let _ = self.store.save_cooldown(&crate::session::Cooldown {
            account: Some(crate::session::account_fingerprint(apple_id)),
            consecutive_failures: self.session.consecutive_failures,
            retry_after: self.session.retry_after,
        });
        let _ = self.persist();
        outcome
    }

    async fn sign_in_inner(
        &mut self,
        apple_id: &str,
        password: &str,
        code: Code,
    ) -> Result<SignInOutcome> {
        // Refuse before touching Apple. Hammering sign-in is what gets an account
        // temporarily locked, and an impatient user clicking twice should not risk that.
        if let Some(seconds) = session::wait_left(self.session.retry_after, unix_now()) {
            return Err(Error::CoolingDown { seconds });
        }

        // Start clean. An interrupted previous attempt would otherwise announce a
        // half-finished session and Apple would reject this one as a conflict.
        self.clear_challenge_state();
        // A new web session: its encrypted services' keys are its own to ask for.
        self.data_access_forget();

        let client = srp::Client::new(apple_id);

        // --- /signin/init: send A, receive salt, B, protocol, iteration count ---
        let headers = self.srp_init_headers();
        let r = self
            .http
            .post(format!("{}/signin/init", self.endpoints.auth))
            .headers(headers)
            .body(
                json!({
                    "a": B64.encode(client.a_public_bytes()),
                    "accountName": apple_id,
                    "protocols": ["s2k", "s2k_fo"],
                })
                .to_string(),
            )
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(refused_password("starting sign-in", status, &text));
        }
        if !status.is_success() {
            return Err(apple_error("starting sign-in", status, &text));
        }
        let init: Value = serde_json::from_str(&text)?;
        let Challenge {
            salt,
            b_pub,
            c,
            iterations,
            protocol,
        } = Challenge::of(&init)?;

        // --- /signin/complete: submit the proof ---
        let proof = client
            .process_challenge(password, protocol, &salt, &b_pub, iterations)
            .ok_or_else(|| {
                Error::Protocol("Apple's sign-in challenge failed its safety check".into())
            })?;

        let mut body = json!({
            "accountName": apple_id,
            "c": c,
            "m1": B64.encode(proof.m1),
            "m2": B64.encode(proof.m2),
            "rememberMe": true,
            "trustTokens": [],
        });
        if let Some(t) = &self.session.trust_token {
            body["trustTokens"] = json!([t]);
        }

        let headers = self.srp_headers();
        let r = self
            .http
            .post(format!("{}/signin/complete", self.endpoints.auth))
            .query(&[("isRememberMeEnabled", "true")])
            .headers(headers)
            .body(body.to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let escrow = wants_escrow(&r);
        // This step's own reply: its serviceErrors say why Apple refused (a locked account,
        // a wrong password). The challenge from /signin/init is not the reason and is never shown.
        let text = if status.is_success() || status == StatusCode::CONFLICT {
            String::new()
        } else {
            r.text().await.unwrap_or_default()
        };

        match status {
            // Apple accepted this computer's trust token and wants its escrow step, not a code:
            // read as "a code", every lapse asked for one — about daily.
            StatusCode::CONFLICT if escrow && self.session.trust_token.is_some() => {
                self.complete_escrow(password).await?;
                let data = self.account_login().await?;
                self.absorb(&data);
                self.keep();
                Ok(SignInOutcome::Complete)
            }
            StatusCode::CONFLICT => {
                self.keep();
                // Ask for the code — when the person has just typed their password. Apple used
                // to push it to trusted devices on this 409 by itself; since iOS 26.4 it waits
                // to be asked (PUT verify/trusteddevice/securitycode), and until asked no
                // "Allow sign-in?" ever reaches a phone — which is why the only way a code ever
                // arrived here was Airplane Mode and Settings, or pressing "Send again". Not
                // fatal if it fails: the code screen still offers sending it again, and a text.
                //
                // A sign-in nobody is watching (a lapse mended in the background) asks for
                // nothing: the code screen asks once the person opens it ([`Self::request_code`]).
                self.sign.wants_code(false);
                if code == Code::Send {
                    if let Err(e) = self.resend_to_devices().await {
                        quiet!("asking Apple to send the code: {e}");
                    }
                }
                Ok(SignInOutcome::NeedsTwoFactor)
            }
            s if s.is_success() => {
                let data = self.account_login().await?;
                self.absorb(&data);
                self.keep();
                Ok(SignInOutcome::Complete)
            }
            StatusCode::UNAUTHORIZED => Err(refused_password("finishing sign-in", status, &text)),
            // "Precondition failed": Apple wants a step on the account finished first (seen right
            // after new iCloud terms, and on accounts without two-factor). icloudpd answers it
            // with an empty `POST repair/complete` and carries on; so does this.
            StatusCode::PRECONDITION_FAILED => {
                self.repair_complete().await?;
                let data = self.account_login().await?;
                self.absorb(&data);
                self.keep();
                Ok(SignInOutcome::Complete)
            }
            s => Err(apple_error("finishing sign-in", s, &text)),
        }
    }

    /// Where Apple can send a code, and whether it already pushed one.
    ///
    /// Worth reading whenever a code doesn't arrive: a `trusted_device_count` of zero means
    /// no push was ever sent and SMS is the only route.
    pub async fn two_factor_options(&mut self) -> Result<TwoFactorOptions> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .get(&self.endpoints.auth)
            .headers(headers)
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        // A reply Apple refused, or one that names no devices and no numbers at all, says
        // nothing about where a code can go. Read as "0 devices, no numbers" it told someone
        // with an iPhone and a number that their account had nowhere to send a code (reported
        // on X, 5 Nov 2026, after airplane-mode codes had failed): it is an error, and the code
        // screen falls back to its plain wording.
        if !status.is_success() {
            return Err(apple_error(
                "reading where Apple can send a code",
                status,
                &text,
            ));
        }
        let v = auth_options_of(&text);
        let places = two_factor_places(&v);
        let says_where = places.iter().any(|p| {
            [
                "trustedDeviceCount",
                "hasTrustedDevices",
                "trustedPhoneNumbers",
                "trustedPhoneNumber",
            ]
            .iter()
            .any(|k| p.get(*k).is_some())
        });
        // A security-key account names no devices and no numbers: its answer is the key
        // challenge. Said as such, so the screen can tell the person, rather than refused as an
        // answer that says nothing.
        let security_key = places.iter().any(|p| offers_security_key(p));
        if !says_where && !security_key {
            return Err(Error::Protocol(
                "Apple didn't say where it can send a code".into(),
            ));
        }

        let phones = trusted_phones(&v);
        self.phones.clone_from(&phones);
        // A count where Apple gives one; where it says only that there are trusted devices
        // (`direct.hasTrustedDevices`), at least one.
        let counted = places
            .iter()
            .find_map(|p| p.get("trustedDeviceCount").and_then(Value::as_u64));
        let has_devices = places
            .iter()
            .any(|p| p.get("hasTrustedDevices").and_then(Value::as_bool) == Some(true));
        let too_many = places.iter().any(|p| {
            p.pointer("/securityCode/tooManyCodesSent")
                .and_then(Value::as_bool)
                == Some(true)
        });

        Ok(TwoFactorOptions {
            trusted_device_count: counted.unwrap_or(u64::from(has_devices)),
            phones,
            too_many_codes_sent: too_many,
            security_key,
        })
    }

    /// The `phoneNumber` and `mode` for `phone_id`, as Apple last described the number.
    fn phone_payload(&self, phone_id: u64) -> (Value, String) {
        self.phones.iter().find(|p| p.id == phone_id).map_or_else(
            || (json!({ "id": phone_id }), "sms".to_string()),
            |p| (p.as_payload(), p.mode.clone()),
        )
    }

    /// The code screen is open: ask Apple to send the code, unless it was already asked for
    /// this sign-in (the person typed their password a moment ago, or had it texted). A lapse
    /// mended in the background left it unasked on purpose — see [`Self::resume`].
    pub async fn request_code(&mut self) -> Result<()> {
        if self.sign.code_sent() {
            return Ok(());
        }
        self.resend_to_devices().await
    }

    /// Ask Apple to push the code to trusted devices again.
    pub async fn resend_to_devices(&mut self) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .put(format!(
                "{}/verify/trusteddevice/securitycode",
                self.endpoints.auth
            ))
            .headers(headers)
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            let body = r.text().await.unwrap_or_default();
            return Err(apple_error("sending the code again", status, &body));
        }
        self.sign.sent();
        Ok(())
    }

    /// Text the code to one of the trusted phone numbers.
    pub async fn send_sms(&mut self, phone_id: u64) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .put(format!("{}/verify/phone", self.endpoints.auth))
            .headers(headers)
            .body({
                let (phone, mode) = self.phone_payload(phone_id);
                json!({ "phoneNumber": phone, "mode": mode }).to_string()
            })
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            let body = r.text().await.unwrap_or_default();
            return Err(apple_error("texting the code", status, &body));
        }
        self.sign.sent();
        Ok(())
    }

    /// Submit a code that arrived by SMS. Trusted-device codes use `submit_two_factor`.
    pub async fn submit_sms_code(&mut self, phone_id: u64, code: &str) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let (phone, mode) = self.phone_payload(phone_id);
        let r = self
            .http
            .post(format!("{}/verify/phone/securitycode", self.endpoints.auth))
            .headers(headers)
            .body(
                json!({
                    "phoneNumber": phone,
                    "securityCode": { "code": code },
                    "mode": mode,
                })
                .to_string(),
            )
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let escrow = wants_escrow(&r);
        // A correct code on an account with steps waiting (right after new iCloud terms, say)
        // is answered 412 with a repair session token, not 200 (pyicloud#466): accepted.
        let repair = status == StatusCode::PRECONDITION_FAILED
            && r.headers().contains_key("x-apple-repair-session-token");
        let body = r.text().await.unwrap_or_default();
        if !status.is_success() && status != StatusCode::NO_CONTENT && !repair {
            let err = apple_error("checking that texted code", status, &body);
            if matches!(err, Error::BadCode) {
                return Err(err);
            }
            if self.trust().await.is_ok() {
                return self.open_after_code().await;
            }
            return Err(err);
        }
        self.escrow_if_asked(escrow).await?;
        self.trust().await?;
        self.open_after_code().await
    }

    /// Submit the six-digit code, then ask Apple to trust this install.
    pub async fn submit_two_factor(&mut self, code: &str) -> Result<()> {
        let mut headers = self.auth_headers();
        headers.insert("accept", HeaderValue::from_static("application/json"));
        let r = self
            .http
            .post(format!(
                "{}/verify/trusteddevice/securitycode",
                self.endpoints.auth
            ))
            .headers(headers)
            .body(json!({ "securityCode": { "code": code } }).to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let escrow = wants_escrow(&r);
        let body = r.text().await.unwrap_or_default();
        // Since iOS 26.4 a correct code is answered 409 with `securityCode.valid: true`.
        let accepted = status == StatusCode::CONFLICT
            && serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.pointer("/securityCode/valid").and_then(Value::as_bool))
                == Some(true);
        if !status.is_success() && status != StatusCode::NO_CONTENT && !accepted {
            let err = apple_error("checking that code", status, &body);
            // A genuinely wrong code is final; don't burn an attempt probing further.
            if matches!(err, Error::BadCode) {
                return Err(err);
            }
            // Apple answers 409 on this endpoint in some flows even though the code WAS
            // accepted. The status alone is not evidence, so ask whether the session became
            // usable before reporting a failure.
            if self.trust().await.is_ok() {
                return self.open_after_code().await;
            }
            return Err(err);
        }

        self.escrow_if_asked(escrow).await?;
        self.trust().await?;
        self.open_after_code().await
    }

    /// Whether `password` is this account's Apple ID password: what the Hidden album asks for,
    /// as icloud.com asks for it before showing Hidden. The password this computer already
    /// keeps (in the keyring, for signing back in) answers it here when it matches — offline,
    /// with no sign-in for Apple to note. Otherwise (none kept, or the kept one out of date) it
    /// is proved to Apple as sign-in proves it (SRP: the password itself is never sent) on a client
    /// of its own with no cookies: the live session is never touched, nothing Apple answers is
    /// kept, and no code is ever asked for — Apple waits to be asked before it sends one.
    pub async fn check_password(&mut self, password: &str) -> Result<bool> {
        // The kept copy is right nearly every time. One that doesn't match may be the old one —
        // the password changed on the phone since — so Apple is asked before "wrong" is said.
        if self
            .session
            .password
            .as_deref()
            .is_some_and(|kept| same_secret(kept.as_bytes(), password.as_bytes()))
        {
            return Ok(true);
        }
        let apple_id = self.session.apple_id.clone().ok_or(Error::Protocol(
            "no Apple ID to check the password against".into(),
        ))?;
        let http = apple_only()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(Error::Network)?;
        let headers = with_srp_origin(
            base_oauth_headers(&self.session.client_id(), &self.endpoints.home, None, None),
            &self.endpoints.auth_root,
        );
        let client = srp::Client::new(&apple_id);
        let r = http
            .post(format!("{}/signin/init", self.endpoints.auth))
            .headers(headers.clone())
            .body(
                json!({
                    "a": B64.encode(client.a_public_bytes()),
                    "accountName": apple_id,
                    "protocols": ["s2k", "s2k_fo"],
                })
                .to_string(),
            )
            .send()
            .await?;
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Ok(false);
        }
        if !status.is_success() {
            return Err(apple_error("checking your password", status, &text));
        }
        let challenge = Challenge::of(&serde_json::from_str(&text)?)?;
        let proof = client
            .process_challenge(
                password,
                challenge.protocol,
                &challenge.salt,
                &challenge.b_pub,
                challenge.iterations,
            )
            .ok_or_else(|| {
                Error::Protocol("Apple's sign-in challenge failed its safety check".into())
            })?;
        let mut body = json!({
            "accountName": apple_id,
            "c": challenge.c,
            "m1": B64.encode(proof.m1),
            "m2": B64.encode(proof.m2),
            "rememberMe": false,
            "trustTokens": [],
        });
        if let Some(t) = &self.session.trust_token {
            body["trustTokens"] = json!([t]);
        }
        let r = http
            .post(format!("{}/signin/complete", self.endpoints.auth))
            .headers(headers)
            .body(body.to_string())
            .send()
            .await?;
        match r.status() {
            // Accepted — whether Apple would want a code next, its escrow step, or nothing: the
            // password was right, and nothing further is asked.
            s if s.is_success()
                || s == StatusCode::CONFLICT
                || s == StatusCode::PRECONDITION_FAILED =>
            {
                Ok(true)
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Ok(false),
            s => {
                let text = r.text().await.unwrap_or_default();
                Err(apple_error("checking your password", s, &text))
            }
        }
    }

    /// Apple's escrow step: the SRP password proof again, with an empty account name, sending
    /// the session key with it — `POST escrow/init` `{a, accountName: "", protocols}`, then
    /// `POST escrow/complete` `{m1, m2, c, k}` (icloud-photos-sync#1124, rclone#10048). Apple
    /// issues a new session token for it, taken with the other headers.
    async fn complete_escrow(&mut self, password: &str) -> Result<()> {
        let client = srp::Client::new("");
        let r = self
            .http
            .post(format!("{}/escrow/init", self.endpoints.auth))
            .headers(self.srp_headers())
            .body(
                json!({
                    "a": B64.encode(client.a_public_bytes()),
                    "accountName": "",
                    "protocols": ["s2k", "s2k_fo"],
                })
                .to_string(),
            )
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(apple_error(
                "confirming your password with Apple",
                status,
                &text,
            ));
        }
        let challenge = Challenge::of(&serde_json::from_str(&text)?)?;
        let proof = client
            .process_challenge(
                password,
                challenge.protocol,
                &challenge.salt,
                &challenge.b_pub,
                challenge.iterations,
            )
            .ok_or_else(|| {
                Error::Protocol("Apple's sign-in challenge failed its safety check".into())
            })?;
        let r = self
            .http
            .post(format!("{}/escrow/complete", self.endpoints.auth))
            .headers(self.srp_headers())
            .body(
                json!({
                    "m1": B64.encode(proof.m1),
                    "m2": B64.encode(proof.m2),
                    "c": challenge.c,
                    "k": B64.encode(proof.key),
                })
                .to_string(),
            )
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            let text = r.text().await.unwrap_or_default();
            return Err(apple_error(
                "confirming your password with Apple",
                status,
                &text,
            ));
        }
        quiet!("quince: Apple's escrow step is done");
        Ok(())
    }

    /// `POST repair/complete` with `{}`, as icloudpd sends it when `/signin/complete` answers
    /// 412. Refused, the account needs the person at account.apple.com.
    async fn repair_complete(&mut self) -> Result<()> {
        let r = self
            .http
            .post(format!("{}/repair/complete", self.endpoints.auth))
            .headers(self.auth_headers())
            .body("{}")
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        if status.is_success() || status == StatusCode::NO_CONTENT {
            return Ok(());
        }
        if let Some(e) = transient(status) {
            return Err(e);
        }
        Err(Error::RepairNeeded)
    }

    /// The password for the escrow step after a code: the one just typed, or the one kept.
    fn password_for_escrow(&self) -> Option<String> {
        self.sign
            .password()
            .map(str::to_string)
            .or_else(|| self.session.password.clone())
    }

    /// After an accepted code: Apple's escrow step when the answer asks for it, before the
    /// trust token is asked for — without it Apple does not honour that token next time.
    async fn escrow_if_asked(&mut self, asked: bool) -> Result<()> {
        if !asked {
            return Ok(());
        }
        let password = self.password_for_escrow().ok_or(Error::ReauthRequired)?;
        self.complete_escrow(&password).await
    }

    /// `GET /2sv/trust` — yields the trust token. Without it, sign-in wouldn't outlive the process.
    async fn trust(&mut self) -> Result<()> {
        let headers = self.auth_headers();
        let r = self
            .http
            .get(format!("{}/2sv/trust", self.endpoints.auth))
            .headers(headers)
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let body = r.text().await.unwrap_or_default();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            return Err(apple_error("trusting this device", status, &body));
        }
        if self.session.trust_token.is_none() {
            return Err(Error::NoTrustToken);
        }
        Ok(())
    }

    /// Exchange the session token for Apple's per-account service URL map.
    async fn account_login(&mut self) -> Result<Value> {
        let body = json!({
            "accountCountryCode": self.session.account_country.clone().unwrap_or_default(),
            "dsWebAuthToken": self.session.session_token.clone().unwrap_or_default(),
            "extended_login": true,
            "trustToken": self.session.trust_token.clone().unwrap_or_default(),
        });
        let r = self
            .http
            .post(format!("{}/accountLogin", self.endpoints.setup))
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        self.capture(&r);
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        // An account Apple keeps on its regional servers: answered `302` with only
        // `{"domainToUse": "iCloud.com.cn"}` (issue #10; rclone#8257). Asked again there, with
        // the sign-in this session has; an answer naming where it already is goes on below.
        if let Some(to) = domain_to_use(&text) {
            let here = self.session.domain.as_deref().unwrap_or(HOME_DOMAIN);
            if !to.eq_ignore_ascii_case(here) {
                if !to.eq_ignore_ascii_case(REGIONAL_DOMAIN)
                    && !to.eq_ignore_ascii_case(HOME_DOMAIN)
                {
                    return Err(Error::Protocol(format!(
                        "Apple keeps this Apple Account at {to}, which Skykeep can't open yet"
                    )));
                }
                quiet!("quince: Apple keeps this account at {to}; asking there");
                self.switch_servers(Some(&to))?;
                return match Box::pin(self.account_login()).await {
                    Ok(data) => Ok(data),
                    // Not there to ask: said as it is, and asked again later.
                    Err(e @ (Error::Network(_) | Error::Unavailable(_) | Error::RateLimited)) => {
                        Err(e)
                    }
                    // A sign-in made at iCloud.com that Apple won't take there: made again.
                    Err(_) => Err(Error::Moved),
                };
            }
        }
        if !status.is_success() {
            return Err(transient(status).unwrap_or_else(|| {
                Error::Protocol(format!("couldn't open the account ({status})"))
            }));
        }
        let data: Value = serde_json::from_str(&text)?;
        diag::account("accountLogin", &data);
        // New iCloud terms not yet accepted: Apple answers 200 but opens the session only
        // part-way, without the web cookies every service needs (rclone/rclone#9919). Not an
        // error: a sign-in that met them is complete — the password proved itself, and is no
        // failed attempt — and `absorb` keeps `termsUpdateNeeded`, which the window says and
        // every pass asks again until the person has agreed at icloud.com.
        Ok(data)
    }

    /// This account's servers, from now on: Apple's at `domain` (its `domainToUse`) — iCloud.com's
    /// for `None` — with a client of their own (their web origin on every request), the lock
    /// opened to Apple's regional domains only for an account kept there, and the choice kept
    /// in the session so every later launch starts there. The cookies stay: each belongs to
    /// the host that set it.
    fn switch_servers(&mut self, domain: Option<&str>) -> Result<()> {
        let regional = domain.is_some_and(|d| d.eq_ignore_ascii_case(REGIONAL_DOMAIN));
        let endpoints = if regional {
            self.regional_endpoints.clone()
        } else {
            self.home_endpoints.clone()
        };
        if endpoints != self.endpoints {
            self.http = apple_client(&endpoints, &self.jar)?;
            self.endpoints = endpoints;
        }
        open_regional(regional);
        self.session.domain = regional.then(|| REGIONAL_DOMAIN.to_string());
        let _ = self.persist();
        Ok(())
    }

    /// Where a test's regional servers are (its mock), instead of Apple's. Test seam.
    pub fn regional_endpoints_at(&mut self, endpoints: Endpoints) {
        self.regional_endpoints = endpoints;
    }

    /// The account opened after a code was taken. An account Apple then names as kept on its
    /// regional servers, where this sign-in isn't taken, is signed in again there with the
    /// password still in flight: Apple sends a new code for them, and the code screen, where
    /// the person is, says to enter it (issue #10).
    async fn open_after_code(&mut self) -> Result<()> {
        match self.account_login().await {
            Ok(data) => {
                self.absorb(&data);
                self.keep();
                Ok(())
            }
            Err(Error::Moved) => {
                let Some((apple_id, password)) = self.sign.credentials() else {
                    return Err(Error::Moved);
                };
                match Box::pin(self.sign_in_as(&apple_id, &password, Code::Send)).await? {
                    SignInOutcome::Complete => Ok(()),
                    SignInOutcome::NeedsTwoFactor => Err(Error::CodeAgain),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn absorb(&mut self, data: &Value) {
        // A completed sign-in — with or without a code — is when a password has proved
        // itself, so that is when it is kept.
        if let Some((apple_id, password)) = self.sign.finish() {
            self.session.apple_id = Some(apple_id);
            self.session.password = Some(password);
        }
        if let Some(ws) = data.get("webservices") {
            self.session.webservices = Some(ws.clone());
        }
        if let Some(d) = data.pointer("/dsInfo/dsid").and_then(|v| v.as_str()) {
            self.session.dsid = Some(d.to_string());
        }
        let text = |path: &str| {
            data.pointer(path)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        if let Some(name) = text("/dsInfo/fullName").or_else(|| {
            let first = text("/dsInfo/firstName")?;
            Some(match text("/dsInfo/lastName") {
                Some(last) => format!("{first} {last}"),
                None => first,
            })
        }) {
            self.session.account_name = Some(name);
        }
        if let Some(email) = text("/dsInfo/primaryEmail").or_else(|| text("/dsInfo/appleId")) {
            self.session.account_email = Some(email);
        }
        if let Some(on) = data
            .get("iCDPEnabled")
            .or_else(|| data.pointer("/dsInfo/iCDPEnabled"))
            .and_then(Value::as_bool)
        {
            self.session.data_protection = Some(on);
        }
        // Web access, new terms and the account to finish, from every answer that carries
        // them, so agreeing at icloud.com clears them at the next `accountLogin` — the
        // heartbeat's, or Try Again's.
        self.session.gate.absorb(data);
        if self.session.gate.standing() == state::Standing::Terms {
            quiet!("quince: Apple has iCloud terms waiting to be agreed to");
        }
    }

    /// Ask Apple for the account again — after the person agreed to new iCloud terms at
    /// icloud.com, say: a fresh `accountLogin` opens the whole web session they were waiting on.
    pub async fn refresh_account(&mut self) -> Result<()> {
        let data = self.account_login().await?;
        self.absorb(&data);
        self.keep();
        Ok(())
    }

    /// Apple is keeping this session from the account's data until something is done at
    /// icloud.com: new terms agreed to, or the account finished. Nothing else can be read
    /// meanwhile, so nothing else is asked.
    #[must_use]
    pub fn waits_on_icloud_com(&self) -> bool {
        self.standing().at_icloud_com()
    }

    /// What, if anything, Apple has said holds the account's data back from this session.
    #[must_use]
    pub fn standing(&self) -> state::Standing {
        self.session.gate.standing()
    }

    /// Query parameters every iCloud service endpoint expects alongside the session cookies.
    #[must_use]
    pub fn service_params(&self) -> Vec<(String, String)> {
        let mut params = vec![
            ("clientBuildNumber".to_string(), CLIENT_BUILD.to_string()),
            (
                "clientMasteringNumber".to_string(),
                CLIENT_MASTERING.to_string(),
            ),
            (
                "clientId".to_string(),
                self.session.client_id.clone().unwrap_or_default(),
            ),
        ];
        if let Some(dsid) = &self.session.dsid {
            params.push(("dsid".to_string(), dsid.clone()));
        }
        params
    }

    /// The account service's root (`…/setup/ws/1`), where storage use is asked for.
    #[must_use]
    pub fn setup_url(&self) -> &str {
        &self.endpoints.setup
    }

    /// Shared HTTP client, for the service crates that ride this session.
    pub fn http(&self) -> &HttpClient {
        &self.http
    }
}

/// The domains Apple's client can reach, each with every host under it: Apple's sign-in,
/// iCloud's services and their files, and push. What Settings
/// lists as what Skykeep talks to, read from here so the list is the lock itself.
pub const APPLE_DOMAINS: [&str; 3] = ["apple.com", "icloud.com", "icloud-content.com"];

/// Apple's own domains for the accounts it keeps at [`REGIONAL_DOMAIN`]: its sign-in, iCloud's
/// services and their files there, each served under Apple's certificate. Reachable only once
/// this computer's account is one of them ([`open_regional`]), so every other account's
/// client reaches exactly [`APPLE_DOMAINS`], and Settings lists exactly what can be reached.
pub const REGIONAL_DOMAINS: [&str; 3] = ["apple.com.cn", "icloud.com.cn", "icloud-content.com.cn"];

/// Whether [`REGIONAL_DOMAINS`] are open: set when the signed-in account is kept there.
static REGIONAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Open (or close) Apple's regional domains, for an account Apple keeps there (or no longer).
pub fn open_regional(open: bool) {
    REGIONAL.store(open, std::sync::atomic::Ordering::SeqCst);
}

/// Every domain Apple's client can reach right now: [`APPLE_DOMAINS`], and [`REGIONAL_DOMAINS`]
/// while the account is kept there. What Settings lists, read from the lock itself.
#[must_use]
pub fn reachable_domains() -> Vec<&'static str> {
    let mut all = APPLE_DOMAINS.to_vec();
    if REGIONAL.load(std::sync::atomic::Ordering::SeqCst) {
        all.extend(REGIONAL_DOMAINS);
    }
    all
}

/// The web origin Apple's services expect from this account: iCloud.com's, or its regional
/// site's while the account is kept there. For requests that name it themselves (push, Drive).
#[must_use]
pub fn web_origin() -> &'static str {
    if REGIONAL.load(std::sync::atomic::Ordering::SeqCst) {
        "https://www.icloud.com.cn"
    } else {
        "https://www.icloud.com"
    }
}

/// Whether this computer counts as Apple's: in this crate's tests, and in a build made with the
/// `loopback` feature (the other crates' tests, whose mock servers run here) — never in the
/// app. A shipped client that reached `localhost` could be pointed, by an address in Apple's
/// data (a contact's photo), at whatever else runs on the person's computer.
const LOOPBACK: bool = cfg!(any(test, feature = "loopback"));

/// Whether `host` is Apple's: under one of [`APPLE_DOMAINS`] — every host Skykeep has any
/// reason to reach. Loopback too, which is this computer, only in tests (see `LOOPBACK`).
#[must_use]
pub fn is_apple_host(host: &str) -> bool {
    apple_host(
        host,
        LOOPBACK,
        REGIONAL.load(std::sync::atomic::Ordering::SeqCst),
    )
}

fn apple_host(host: &str, loopback: bool, regional: bool) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let under = |d: &&str| host == *d || host.ends_with(&format!(".{d}"));
    (loopback && host == "localhost")
        || APPLE_DOMAINS.iter().any(under)
        || (regional && REGIONAL_DOMAINS.iter().any(under))
}

/// Every HTTP client Skykeep makes for iCloud starts here: Apple's servers, and nothing else,
/// can be reached at all (see [`AppleOnly`]). The two other clients are just as narrow:
/// [`listed_only`] for subscribed calendars and [`releases::releases_only`] for updates.
pub fn apple_only() -> reqwest::ClientBuilder {
    let builder = HttpClient::builder()
        .dns_resolver(Arc::new(AppleOnly))
        // The resolver alone is not enough: a URL with an address in it (`https://1.2.3.4/`)
        // is never looked up, and through a proxy the proxy looks the name up, not us. So
        // every request's own URL is checked first, a redirect's included; one that is not
        // Apple's is sent to a "proxy" that cannot be found — by the resolver above, so the
        // refusal reads the same — and never leaves the machine.
        .proxy(reqwest::Proxy::custom(|url| {
            (!url_is_apples(url)).then(|| REFUSED.to_string())
        }));
    // Setting a proxy turns off the system's own (`HTTPS_PROXY` and the rest), which Apple's
    // hosts still go through when one is set, as they did before.
    with_system_proxies(builder)
}

/// The proxy the system names for `scheme` ("https" or "http"), from `HTTPS_PROXY` and the
/// rest, if any.
pub(crate) fn env_proxy(scheme: &str) -> Option<String> {
    let names = if scheme == "https" {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    names
        .iter()
        .find_map(|n| std::env::var(n).ok().filter(|v| !v.trim().is_empty()))
}

/// The system's own proxies (`HTTPS_PROXY` and the rest), put back after a refusing proxy has
/// turned them off.
fn with_system_proxies(mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    for scheme in ["https", "http"] {
        let Some(via) = env_proxy(scheme) else {
            continue;
        };
        let proxy =
            reqwest::Proxy::custom(move |url| (url.scheme() == scheme).then(|| via.clone()))
                .no_proxy(reqwest::NoProxy::from_env());
        builder = builder.proxy(proxy);
    }
    builder
}

/// The client for calendars the person subscribed to by address — and for nothing else. It
/// reaches exactly the hosts in `hosts` (the ones those addresses name), by the same two locks
/// as [`apple_only`]: a resolver that looks up no other name, and a check of every request's
/// own URL, a redirect's included, so an address or a proxy cannot carry it anywhere else.
/// Apple's own client is not widened by it; nothing else in Skykeep uses this one.
pub fn listed_only(hosts: &[String]) -> reqwest::ClientBuilder {
    let listed: Arc<Vec<String>> = Arc::new(hosts.iter().map(|h| normal_host(h)).collect());
    let check = Arc::clone(&listed);
    let builder = HttpClient::builder()
        .dns_resolver(Arc::new(ListedOnly(Arc::clone(&listed))))
        .redirect(reqwest::redirect::Policy::limited(5))
        .proxy(reqwest::Proxy::custom(move |url| {
            (!url_is_listed(url, &check)).then(|| REFUSED.to_string())
        }));
    with_system_proxies(builder)
}

/// The client for a calendar account the person signed in to themselves: Nextcloud, or any
/// CalDAV server. It reaches exactly `hosts`, by the same two locks as [`listed_only`]. Unlike a
/// subscription, whose address anyone can hand the person, this is the person's own server, so
/// one at home or on a tailnet is reached. Redirects are not followed here: a redirect keeps
/// neither the password nor the method, so the CalDAV discovery reads them itself.
pub fn account_only(hosts: &[String]) -> reqwest::ClientBuilder {
    let listed: Arc<Vec<String>> = Arc::new(hosts.iter().map(|h| normal_host(h)).collect());
    let check = Arc::clone(&listed);
    let builder = HttpClient::builder()
        .dns_resolver(Arc::new(AccountOnly(Arc::clone(&listed))))
        .redirect(reqwest::redirect::Policy::none())
        .proxy(reqwest::Proxy::custom(move |url| {
            let named = url
                .host_str()
                .is_some_and(|h| check.contains(&normal_host(h)));
            (!named).then(|| REFUSED.to_string())
        }));
    with_system_proxies(builder)
}

fn normal_host(host: &str) -> String {
    host.trim_end_matches('.')
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

/// Whether a request's URL names one of the listed hosts exactly — and, where it names an
/// address, one out on the internet.
fn url_is_listed(url: &reqwest::Url, listed: &[String]) -> bool {
    url.host_str().is_some_and(|h| {
        let h = normal_host(h);
        listed.contains(&h) && h.parse::<std::net::IpAddr>().ok().is_none_or(is_public_ip)
    })
}

/// Whether an address is out on the internet: not this computer, not the local network (a
/// router's page, a printer, `192.168.…`, `10.…`, `fd…`), not link-local (where cloud machines
/// keep their secrets, `169.254.169.254`), nor any other range no public calendar is on. A
/// subscription is an address anyone can hand the person — and its name can be pointed at any
/// address its owner likes — so one that leads inside is never fetched.
#[must_use]
pub fn is_public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || a >= 240
                // Shared address space (carrier NAT, tailnets) and benchmarking.
                || (a == 100 && (64..128).contains(&b))
                || (a == 198 && (18..20).contains(&b))
                // IETF protocol assignments.
                || (a == 192 && b == 0 && v4.octets()[2] == 0))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            // NAT64 (`64:ff9b::/96`) carries an IPv4 address inside.
            if v6.segments()[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [.., a, b, c, d] = v6.octets();
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0
                || first == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    }
}

/// The subscribed-calendar client's resolver: a listed host is looked up as the system looks
/// up any name; any other is never looked up at all.
struct ListedOnly(Arc<Vec<String>>);

impl reqwest::dns::Resolve for ListedOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = normal_host(name.as_str());
        let allowed = self.0.contains(&host);
        Box::pin(async move {
            if !allowed {
                let refused: Box<dyn std::error::Error + Send + Sync> =
                    format!("{host} is not a calendar you subscribed to").into();
                return Err(refused);
            }
            // Whatever the name says, only addresses out on the internet are dialled: a feed
            // whose name leads to this computer or the local network reaches neither.
            let found: Vec<_> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|a| is_public_ip(a.ip()))
                .collect();
            if found.is_empty() {
                let refused: Box<dyn std::error::Error + Send + Sync> =
                    format!("{host} is not on the internet").into();
                return Err(refused);
            }
            let addrs: reqwest::dns::Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// A calendar account's resolver: its own hosts are looked up as the system looks up any name,
/// wherever they lead; any other is never looked up at all.
struct AccountOnly(Arc<Vec<String>>);

impl reqwest::dns::Resolve for AccountOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = normal_host(name.as_str());
        let allowed = self.0.contains(&host);
        Box::pin(async move {
            if !allowed {
                let refused: Box<dyn std::error::Error + Send + Sync> =
                    format!("{host} is not a calendar account of yours").into();
                return Err(refused);
            }
            let found: Vec<_> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let addrs: reqwest::dns::Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// Where a request that is not Apple's is pointed: a name the resolver refuses.
const REFUSED: &str = "http://refused.skykeep.invalid";

/// Whether a request's URL may be sent: an Apple host by name, or this computer in tests.
fn url_is_apples(url: &reqwest::Url) -> bool {
    url_apples(url, LOOPBACK)
}

fn url_apples(url: &reqwest::Url, loopback: bool) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    match host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<std::net::IpAddr>()
    {
        Ok(ip) => loopback && ip.is_loopback(),
        // Apple's hosts over TLS only. Plain HTTP to one — an address in Apple's data, or a
        // redirect down to it — would carry the session's cookies where anyone on the network
        // can read them (pyicloud#356, #401). This computer, in tests, is the one exception.
        Err(_) => {
            let secure = matches!(url.scheme(), "https" | "wss");
            (secure || (loopback && host == "localhost"))
                && apple_host(
                    host,
                    loopback,
                    REGIONAL.load(std::sync::atomic::Ordering::SeqCst),
                )
        }
    }
}

/// The one way Apple's client finds a server: a host that is not Apple's is never looked up,
/// so no request of Apple's client can reach one — how the code works, not a promise about it.
/// Apple's are looked up as the system looks up any name.
struct AppleOnly;

impl reqwest::dns::Resolve for AppleOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            if !is_apple_host(&host) {
                let refused: Box<dyn std::error::Error + Send + Sync> =
                    format!("{host} is not Apple's, and Apple's client reaches only Apple").into();
                return Err(refused);
            }
            let found = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let addrs: reqwest::dns::Addrs = Box::new(found.collect::<Vec<_>>().into_iter());
            Ok(addrs)
        })
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;

    /// What the shipped client allows: Apple's hosts, and never this computer — by name or
    /// by address.
    #[test]
    fn a_release_client_never_reaches_this_computer() {
        for host in ["localhost", "LOCALHOST.", "127.0.0.1", "[::1]"] {
            assert!(!apple_host(host, false, false), "{host}");
            assert!(!apple_host(host, false, true), "{host}");
        }
        for url in [
            "http://localhost:8080/",
            "http://127.0.0.1:631/",
            "http://127.1.2.3/",
            "http://[::1]:9/",
            "https://localhost./",
        ] {
            let u = reqwest::Url::parse(url).unwrap();
            assert!(!url_apples(&u, false), "{url}");
            assert!(url_apples(&u, true), "{url}");
        }
        assert!(url_apples(
            &reqwest::Url::parse("https://p01-contacts.icloud.com/x").unwrap(),
            false
        ));
    }
}

#[cfg(test)]
mod listed_tests {
    use super::*;

    /// A subscribed calendar's client reaches its own hosts and refuses every other — by
    /// name, by address, and Apple's too — before anything leaves the machine.
    #[tokio::test]
    async fn the_subscription_client_refuses_a_host_nobody_subscribed_to() {
        let http = listed_only(&["calendar.example.org".into()])
            .build()
            .unwrap();
        for url in [
            "https://elsewhere.example.net/feed.ics",
            "https://93.184.216.34/feed.ics",
            "https://www.icloud.com/",
        ] {
            let err = http.get(url).send().await.expect_err(url);
            let text = format!("{err:?}");
            assert!(
                text.contains("not a calendar you subscribed to"),
                "{url} was not refused by the list: {text}"
            );
        }
    }

    #[test]
    fn a_listed_host_is_matched_exactly_and_without_case() {
        let listed = vec![normal_host("Calendar.Example.org.")];
        let ok = |u: &str| url_is_listed(&reqwest::Url::parse(u).unwrap(), &listed);
        assert!(ok("https://calendar.example.org/a.ics"));
        assert!(ok("http://CALENDAR.example.org:8080/a.ics"));
        assert!(!ok("https://evil.calendar.example.org/a.ics"));
        assert!(!ok("https://example.org/a.ics"));
        assert!(!ok("https://calendar.example.org.evil.net/a.ics"));
    }

    /// A calendar account's client reaches its own server, at home too, and nothing else.
    #[tokio::test]
    async fn an_account_client_reaches_its_own_server_and_no_other() {
        let http = account_only(&["127.0.0.1".into()]).build().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let answered = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.unwrap();
            // The request read first, as a server does: Windows resets a socket closed with
            // unread data in it, and the client saw "connection aborted" (10053), not the answer.
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = socket.read(&mut chunk).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
            }
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        });
        let own = http
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(own.status(), 204, "its own server, on this computer");
        answered.await.unwrap();
        let other = http.get("https://example.org/").send().await;
        let text = format!("{:?}", other.unwrap_err());
        assert!(
            text.contains("not a calendar account of yours") || text.contains("refused"),
            "{text}"
        );
    }

    /// A subscription never reaches this computer or the local network: not by an address in
    /// it, not by a name that leads there.
    #[tokio::test]
    async fn a_subscription_never_reaches_inside() {
        use std::net::IpAddr;
        for inside in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.4",
            "192.168.1.1",
            "169.254.169.254",
            "100.100.100.100",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "::",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.0.1",
            "64:ff9b::a9fe:a9fe",
        ] {
            assert!(!is_public_ip(inside.parse::<IpAddr>().unwrap()), "{inside}");
        }
        for out in ["93.184.215.14", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(is_public_ip(out.parse::<IpAddr>().unwrap()), "{out}");
        }
        // Listed, and still refused: an address inside, or a name that leads inside.
        let by_address = vec![normal_host("192.168.1.1"), normal_host("[::1]")];
        for u in ["http://192.168.1.1/a.ics", "http://[::1]:8080/a.ics"] {
            assert!(
                !url_is_listed(&reqwest::Url::parse(u).unwrap(), &by_address),
                "{u}"
            );
        }
        let http = listed_only(&["localhost".into()]).build().unwrap();
        let err = http
            .get("http://localhost:9/feed.ics")
            .send()
            .await
            .expect_err("a name that leads to this computer");
        assert!(
            format!("{err:?}").contains("not on the internet"),
            "{err:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME_T: &str = "https://www.icloud.com";
    const ROOT_T: &str = "https://idmsa.apple.com";

    /// Apple answers 409 when the idmsa origin is sent to an endpoint that expects the
    /// icloud.com one. Only the two SRP calls may carry it.
    #[test]
    fn srp_origin_is_scoped_to_sign_in() {
        let base = base_oauth_headers("auth-test", HOME_T, None, None);
        assert!(
            base.get("origin").is_none(),
            "base headers must not pin an Origin; the client default (icloud.com) applies"
        );

        let srp = with_srp_origin(base_oauth_headers("auth-test", HOME_T, None, None), ROOT_T);
        assert_eq!(srp.get("origin").unwrap(), ROOT_T);
        assert_eq!(srp.get("referer").unwrap(), &format!("{ROOT_T}/")[..]);
    }

    #[test]
    fn session_headers_are_forwarded_when_present() {
        let h = base_oauth_headers(
            "auth-test",
            HOME_T,
            Some("scnt-value"),
            Some("session-value"),
        );
        assert_eq!(h.get("scnt").unwrap(), "scnt-value");
        assert_eq!(h.get("x-apple-id-session-id").unwrap(), "session-value");
    }

    /// Sign-in must not advertise a session that does not exist yet.
    #[test]
    fn init_headers_omit_challenge_state() {
        let stale = base_oauth_headers(
            "auth-test",
            HOME_T,
            Some("stale-scnt"),
            Some("stale-session"),
        );
        assert!(
            stale.get("scnt").is_some(),
            "sanity: the helper does forward them"
        );

        let init = with_srp_origin(base_oauth_headers("auth-test", HOME_T, None, None), ROOT_T);
        assert!(
            init.get("scnt").is_none(),
            "/signin/init must not send scnt"
        );
        assert!(
            init.get("x-apple-id-session-id").is_none(),
            "/signin/init must not send a session id"
        );
        assert_eq!(init.get("origin").unwrap(), ROOT_T);
    }

    /// The first retry should be nearly free; repeated failures must back off hard, because
    /// the penalty for hammering Apple is a temporary account lock.
    #[test]
    fn backoff_grows_and_is_capped() {
        use crate::session::backoff_seconds;
        assert_eq!(backoff_seconds(0), 0, "a first attempt must not be delayed");
        let steps: Vec<u64> = (1..=6).map(backoff_seconds).collect();
        for pair in steps.windows(2) {
            assert!(pair[1] >= pair[0], "backoff must never shrink: {steps:?}");
        }
        assert_eq!(
            backoff_seconds(u32::MAX),
            backoff_seconds(5),
            "and it must be bounded"
        );
    }

    #[test]
    fn a_locked_account_and_a_wrong_password_read_as_what_they_are() {
        let words = Error::BadCredentials.to_string();
        assert!(words.contains("app-specific"), "{words}");
        let locked = r#"{"serviceErrors":[{"code":"-20209","message":"This Apple Account has been locked for security reasons."}]}"#;
        assert!(matches!(
            apple_error("finishing sign-in", StatusCode::FORBIDDEN, locked),
            Error::AccountLocked
        ));
        assert!(Error::AccountLocked
            .to_string()
            .contains("iforgot.apple.com"));
    }

    #[test]
    fn apple_errors_are_classified() {
        let wrong =
            r#"{"serviceErrors":[{"code":"-21669","message":"Incorrect verification code."}]}"#;
        assert!(matches!(
            apple_error("x", StatusCode::CONFLICT, wrong),
            Error::BadCode
        ));

        let throttled = r#"{"serviceErrors":[{"code":"-22222","message":"Too many codes sent."}]}"#;
        assert!(matches!(
            apple_error("x", StatusCode::CONFLICT, throttled),
            Error::RateLimited
        ));

        assert!(matches!(
            apple_error("x", StatusCode::TOO_MANY_REQUESTS, ""),
            Error::RateLimited
        ));

        // An outage at sign-in is said as one, not as Apple's error page.
        let page = "<html><head><title>502 Bad Gateway</title></head><body>nginx</body></html>";
        let said = apple_error("starting sign-in", StatusCode::BAD_GATEWAY, page);
        assert!(matches!(said, Error::Unavailable(502)), "got {said:?}");
        assert!(!said.to_string().contains('<'), "no markup: {said}");
        assert!(
            !said.to_string().contains("Skykeep will"),
            "nothing it won't do"
        );

        // Find My answers 450 on a resumed session; that is "sign in again", not a failure.
        assert!(matches!(
            apple_error("x", StatusCode::from_u16(450).unwrap(), ""),
            Error::ReauthRequired
        ));

        // Anything unrecognised must keep Apple's own words rather than flatten to a status.
        let other = r#"{"serviceErrors":[{"code":"-20101","message":"Account locked."}]}"#;
        match apple_error("signing in", StatusCode::CONFLICT, other) {
            Error::Protocol(m) => {
                assert!(m.contains("Account locked."), "got: {m}");
                assert!(m.contains("-20101"), "got: {m}");
            }
            e => panic!("expected Protocol, got {e:?}"),
        }
    }
}

/// The cookie jar on disk: sealed with a key the keyring keeps, and never in the clear.
#[cfg(test)]
mod sealed_jar_tests {
    use super::*;

    const VALUE: &str = "cookie-value-that-must-not-show";

    fn client(dir: &Path) -> Client {
        Client::with(dir, Endpoints::default(), Store::file(dir).unwrap()).unwrap()
    }

    fn holding_a_cookie(dir: &Path) -> Client {
        let c = client(dir);
        let url = reqwest::Url::parse("https://www.icloud.com/").unwrap();
        let cookie = cookie_store::RawCookie::parse(format!(
            "X-APPLE-WEBAUTH-USER={VALUE}; Domain=icloud.com; Path=/; Max-Age=3600; Secure"
        ))
        .unwrap();
        c.jar.lock().unwrap().insert_raw(&cookie, &url).unwrap();
        c
    }

    fn has_the_cookie(c: &Client) -> bool {
        c.jar.lock().unwrap().iter_any().any(|k| k.value() == VALUE)
    }

    fn cookies_file(dir: &Path) -> std::path::PathBuf {
        dir.join("cookies.json")
    }

    /// The jar as it was written before sealing: plain JSON.
    fn plain_jar(dir: &Path) -> Vec<u8> {
        let c = holding_a_cookie(dir);
        let mut json = Vec::new();
        cookie_store::serde::json::save(&c.jar.lock().unwrap(), &mut json).unwrap();
        assert!(String::from_utf8_lossy(&json).contains(VALUE));
        json
    }

    /// No keyring to keep a key: where the key would be is a folder, so nothing can be kept.
    fn no_keyring(dir: &Path) {
        std::fs::create_dir(dir.join("cookie-key")).unwrap();
    }

    fn nothing_but(dir: &Path, names: &[&str]) {
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        let mut want: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn cookies_are_sealed_on_disk_and_read_back_by_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        holding_a_cookie(dir.path()).persist().unwrap();

        let on_disk = std::fs::read(cookies_file(dir.path())).unwrap();
        assert!(
            on_disk.starts_with(sealed::MAGIC),
            "sealed, with its header"
        );
        assert!(
            !String::from_utf8_lossy(&on_disk).contains(VALUE),
            "the cookie is not in the file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(cookies_file(dir.path()))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let next = client(dir.path());
        assert!(has_the_cookie(&next));
        assert!(next.cookies_set_aside().is_none());
    }

    /// A changed byte: set aside cleanly, the file gone, said calmly — never a crash.
    #[test]
    fn a_changed_file_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        holding_a_cookie(dir.path()).persist().unwrap();
        let path = cookies_file(dir.path());
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x40;
        std::fs::write(&path, bytes).unwrap();

        let next = client(dir.path());
        assert!(!has_the_cookie(&next));
        assert!(!path.exists(), "nothing left that cannot be opened");
        assert!(next.cookies_set_aside().is_some(), "and it says so");
        // The next write seals afresh, and that opens.
        holding_a_cookie(dir.path()).persist().unwrap();
        assert!(has_the_cookie(&client(dir.path())));
    }

    #[test]
    fn a_version_this_build_does_not_know_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        holding_a_cookie(dir.path()).persist().unwrap();
        let path = cookies_file(dir.path());
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[sealed::MAGIC.len()] = sealed::VERSION + 1;
        std::fs::write(&path, bytes).unwrap();

        let next = client(dir.path());
        assert!(!has_the_cookie(&next));
        assert!(!path.exists());
        assert!(next.cookies_set_aside().is_some());
    }

    /// The keyring was reset: the file is there and its key is not.
    #[test]
    fn a_file_whose_key_is_gone_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        holding_a_cookie(dir.path()).persist().unwrap();
        std::fs::remove_file(dir.path().join("cookie-key")).unwrap();

        let next = client(dir.path());
        assert!(!has_the_cookie(&next));
        assert!(!cookies_file(dir.path()).exists());
        assert!(next.cookies_set_aside().is_some());
    }

    /// A jar from before sealing is read once and sealed in its place; the plain one is gone,
    /// with nothing beside it.
    #[test]
    fn a_plain_jar_from_before_is_sealed_in_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let json = plain_jar(dir.path());
        std::fs::write(cookies_file(dir.path()), &json).unwrap();

        let c = client(dir.path());
        assert!(has_the_cookie(&c), "nothing lost on the way");
        let on_disk = std::fs::read(cookies_file(dir.path())).unwrap();
        assert!(on_disk.starts_with(sealed::MAGIC));
        assert!(!String::from_utf8_lossy(&on_disk).contains(VALUE));
        nothing_but(dir.path(), &["cookies.json", "cookie-key"]);
        assert!(
            has_the_cookie(&client(dir.path())),
            "and it opens next time"
        );
    }

    /// No keyring, and a plain jar from before: read into memory, and the plain file deleted —
    /// it is not left there, and nothing is written in its place.
    #[test]
    fn a_plain_jar_with_no_keyring_is_kept_in_memory_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let json = plain_jar(dir.path());
        no_keyring(dir.path());
        std::fs::write(cookies_file(dir.path()), &json).unwrap();

        let c = client(dir.path());
        assert!(has_the_cookie(&c), "held for as long as Skykeep runs");
        assert!(
            !cookies_file(dir.path()).exists(),
            "no plaintext left behind"
        );
        nothing_but(dir.path(), &["cookie-key"]);
    }

    /// No keyring: the cookies are not written at all — never in the clear — and saying so is
    /// what `keep` does with the error.
    #[test]
    fn with_no_keyring_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        no_keyring(dir.path());
        let mut c = holding_a_cookie(dir.path());
        assert!(matches!(c.persist(), Err(Error::Keyring(_))));
        assert!(!cookies_file(dir.path()).exists());
        nothing_but(dir.path(), &["cookie-key", "session.json"]);
        c.keep();
        assert!(c.unsaved().is_some(), "said: kept in memory only");
        assert!(has_the_cookie(&c));

        // A keyring comes up: the next write seals them.
        std::fs::remove_dir(dir.path().join("cookie-key")).unwrap();
        c.keep();
        assert!(c.unsaved().is_none());
        assert!(has_the_cookie(&client(dir.path())));
    }

    /// A keyring that will not answer yet is not a reset one: the sealed file is left as it is,
    /// and read once it answers.
    #[test]
    fn a_keyring_that_does_not_answer_leaves_the_file_for_later() {
        let dir = tempfile::tempdir().unwrap();
        holding_a_cookie(dir.path()).persist().unwrap();
        let key = dir.path().join("cookie-key");
        let kept = std::fs::read(&key).unwrap();
        std::fs::remove_file(&key).unwrap();
        no_keyring(dir.path());

        let mut c = client(dir.path());
        assert!(!has_the_cookie(&c));
        assert!(cookies_file(dir.path()).exists(), "untouched");
        assert!(c.cookies_set_aside().is_none());

        std::fs::remove_dir(&key).unwrap();
        std::fs::write(&key, kept).unwrap();
        c.read_cookies_again();
        assert!(has_the_cookie(&c));
    }

    /// A keyring locked at login, or not up yet when Skykeep starts with the desktop: the
    /// session it holds is still there. Read as nothing stored, the launch went to the sign-in
    /// screen — never asking the keyring again, so its unlock prompt never came — and signing
    /// in replaced the session and its trust token, with a code to type.
    #[tokio::test]
    async fn a_keyring_that_does_not_answer_is_asked_again_not_read_as_signed_out() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = holding_a_cookie(dir.path());
        c.session.session_token = Some("token".into());
        c.persist().unwrap();
        assert!(cookies_file(dir.path()).exists());
        // Where the session is kept will not answer.
        let session = dir.path().join("session.json");
        std::fs::remove_file(&session).unwrap();
        std::fs::create_dir(&session).unwrap();

        let mut c = client(dir.path());
        assert!(c.has_stored_session(), "worth a resume, which asks again");
        let said = c.resume().await.unwrap_err();
        assert!(matches!(said, Error::Keyring(_)), "got {said:?}");
        let words = said.to_string();
        assert!(words.contains("keyring") || words.contains("Credential Manager"));
        assert!(
            !words.contains("Is a directory"),
            "no raw OS words: {words}"
        );

        // A first run with no keyring kept nothing: the sign-in screen is right then.
        let fresh = tempfile::tempdir().unwrap();
        std::fs::create_dir(fresh.path().join("session.json")).unwrap();
        assert!(!client(fresh.path()).has_stored_session());
    }

    /// Signing out takes the key with the cookies.
    #[test]
    fn signing_out_forgets_the_key_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = holding_a_cookie(dir.path());
        c.persist().unwrap();
        c.sign_out().unwrap();
        assert!(!cookies_file(dir.path()).exists());
        assert!(!dir.path().join("cookie-key").exists());
    }

    /// Signed out with the keyring refusing, the cookies still go, and neither reopening nor
    /// resuming brings back the session the keyring kept.
    #[tokio::test]
    async fn a_locked_keyring_at_sign_out_never_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = holding_a_cookie(dir.path());
        c.session.session_token = Some("token".into());
        c.persist().unwrap();
        // The key's entry cannot be deleted, as with a locked keyring.
        let key = dir.path().join("cookie-key");
        std::fs::remove_file(&key).unwrap();
        std::fs::create_dir(&key).unwrap();
        std::fs::write(key.join("x"), b"x").unwrap();
        c.sign_out().unwrap();
        assert!(!cookies_file(dir.path()).exists());
        // What the keyring kept, as if never erased.
        std::fs::write(
            dir.path().join("session.json"),
            r#"{"session_token":"token"}"#,
        )
        .unwrap();
        let mut reopened = client(dir.path());
        assert!(!reopened.has_stored_session());
        assert!(!reopened.resume().await.unwrap());
    }

    /// Another Apple ID signing in starts clean: the last account's trust token, client id and
    /// cookies are gone before anything is sent. The same Apple ID typed differently keeps them.
    #[test]
    fn another_apple_id_inherits_nothing_of_the_last() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = holding_a_cookie(dir.path());
        c.session.apple_id = Some("first@example.com".into());
        c.session.trust_token = Some("trust".into());
        c.persist().unwrap();
        let id = c.session.client_id();

        c.forget_other_account("  FIRST@example.com ").unwrap();
        assert_eq!(
            c.session.trust_token.as_deref(),
            Some("trust"),
            "the same account keeps its trust"
        );
        assert_eq!(c.session.client_id(), id);
        assert!(cookies_file(dir.path()).exists());

        c.forget_other_account("second@example.com").unwrap();
        assert!(
            c.session.trust_token.is_none(),
            "another account starts without the last one's trust"
        );
        assert_ne!(c.session.client_id(), id, "nor its client id");
        assert!(!cookies_file(dir.path()).exists(), "nor its cookies");
        assert!(!client(dir.path()).has_stored_session());
    }
}

#[cfg(test)]
mod security_key_tests {
    use super::{offers_security_key, two_factor_places};

    /// Both layouts Apple uses for a security-key account are found: the challenge at the top
    /// and under `fsaVerification`. Neither names a device or a number.
    #[test]
    fn a_security_key_is_found_in_either_layout() {
        let challenge =
            serde_json::json!({ "challenge": "c", "keyHandles": ["k"], "rpId": "apple.com" });
        let top = serde_json::json!({ "fsaChallenge": challenge });
        let inner = serde_json::json!({ "fsaVerification": { "fsaChallenge": challenge } });
        for answer in [&top, &inner] {
            assert!(
                two_factor_places(answer)
                    .iter()
                    .any(|p| offers_security_key(p)),
                "{answer}"
            );
        }
        let codes = serde_json::json!({ "trustedDeviceCount": 1 });
        assert!(!two_factor_places(&codes)
            .iter()
            .any(|p| offers_security_key(p)));
    }
}
