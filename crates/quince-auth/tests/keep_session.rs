//! `Client::ensure_session`, the check every sync pass makes first, against a mock Apple: a
//! lapsed session is taken up again by itself, a real sign-out is said once and never asked
//! about again, and nothing about it hammers Apple.

use quince_auth::{Client, Endpoints, Kept, Store};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn endpoints_for(server: &MockServer) -> Endpoints {
    let base = server.uri();
    Endpoints {
        auth_root: "https://idmsa.test.invalid".into(),
        home: "https://web.test.invalid".into(),
        auth: format!("{base}/appleauth/auth"),
        setup: format!("{base}/setup/ws/1"),
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as E, Engine};
    E.encode(bytes)
}

async fn mount_init(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("scnt", "scnt-from-init")
                .insert_header("X-Apple-ID-Session-Id", "session-from-init")
                .set_body_json(json!({
                    "salt": b64(&[0x77; 16]),
                    "b": b64(&[0x5e; 256]),
                    "c": "challenge-c",
                    "iteration": 20173,
                    "protocol": "s2k",
                })),
        )
        .mount(server)
        .await;
}

async fn mount_complete(server: &MockServer, status: u16) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(status)
                .insert_header("X-Apple-Session-Token", "session-token-abc")
                .set_body_json(json!({})),
        )
        .mount(server)
        .await;
}

async fn mount_trust_and_login(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/appleauth/auth/2sv/trust"))
        .respond_with(
            ResponseTemplate::new(204)
                .insert_header("X-Apple-TwoSV-Trust-Token", "trust-token-xyz")
                .insert_header("X-Apple-Session-Token", "session-token-def"),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "1234567890" },
            "webservices": {
                "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active" }
            }
        })))
        .mount(server)
        .await;
}

fn client_for(server: &MockServer, dir: &std::path::Path) -> Client {
    Client::with(dir, endpoints_for(server), Store::file(dir).unwrap()).unwrap()
}

fn requests_to<'a>(reqs: &'a [Request], suffix: &str) -> Vec<&'a Request> {
    reqs.iter()
        .filter(|r| r.url.path().ends_with(suffix))
        .collect()
}

async fn asked(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

/// Signed in once, all the way through the code, into `dir`.
async fn signed_in(dir: &std::path::Path) {
    let server = MockServer::start().await;
    mount_init(&server).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;
    let mut client = client_for(&server, dir);
    client
        .sign_in("someone@example.invalid", "hunter2")
        .await
        .unwrap();
    client.submit_two_factor("123456").await.unwrap();
}

/// Apple, days later: the cookies and the session token have both lapsed.
async fn lapsed(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(421))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(421))
        .up_to_n_times(1)
        .mount(server)
        .await;
    mount_init(server).await;
}

/// A session whose cookies and token have lapsed is taken up again before the pass, with no
/// code; once found alive, the passes right after it ask Apple nothing more.
#[tokio::test]
async fn a_lapsed_session_is_taken_up_again_and_asked_about_once() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    lapsed(&server).await;
    mount_complete(&server, 200).await;
    mount_trust_and_login(&server).await;

    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    let reqs = server.received_requests().await.unwrap();
    // The heartbeat's look, the resume's own, then the sign-in with the kept password.
    assert!(!requests_to(&reqs, "/validate").is_empty());
    assert_eq!(requests_to(&reqs, "/signin/complete").len(), 1);
    assert!(requests_to(&reqs, "/securitycode").is_empty(), "no code");

    let before = asked(&server).await;
    for _ in 0..5 {
        assert_eq!(client.ensure_session().await, Kept::Alive);
    }
    assert_eq!(
        asked(&server).await,
        before,
        "five passes starting together cost no more requests"
    );
    // A pass that met "sign in again" anyway has the next one ask.
    client.session_refused(client.session_generation());
    client.ensure_session().await;
    assert!(asked(&server).await > before);
}

/// Apple truly ending the session (the password changed elsewhere) is said as needing a
/// sign-in, and every pass after it is answered here: Apple is not asked again, and the
/// password it refused is never tried a second time.
#[tokio::test]
async fn a_real_sign_out_stops_and_is_not_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    for p in ["/setup/ws/1/validate", "/setup/ws/1/accountLogin"] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(421))
            .mount(&server)
            .await;
    }
    mount_init(&server).await;
    mount_complete(&server, 401).await;

    let mut client = client_for(&server, dir.path());
    let kept = client.ensure_session().await;
    assert_eq!(kept, Kept::SignIn);
    assert!(kept.ended());
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(requests_to(&reqs, "/signin/init").len(), 1);
    let before = asked(&server).await;
    for _ in 0..10 {
        assert_eq!(client.ensure_session().await, Kept::SignIn);
    }
    assert_eq!(asked(&server).await, before, "nothing more asked of Apple");
}

/// Trust run out: Apple wants a code. Said as that, without a pass putting a code on the
/// person's phone, and not asked about again until the code is given.
#[tokio::test]
async fn a_code_wanted_is_said_and_waited_for() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    lapsed(&server).await;
    mount_complete(&server, 409).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    mount_trust_and_login(&server).await;

    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::Code);
    let before = asked(&server).await;
    assert_eq!(client.ensure_session().await, Kept::Code);
    assert_eq!(asked(&server).await, before);
    let reqs = server.received_requests().await.unwrap();
    assert!(
        requests_to(&reqs, "/securitycode").is_empty(),
        "no code asked for by a pass"
    );

    // The person enters the code: the session is asked about again, and alive.
    client.submit_two_factor("123456").await.unwrap();
    assert!(!client.awaiting_code());
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    assert_eq!(client.ensure_session().await, Kept::Alive);
}

/// Apple busy is not a sign-out: said as away, the session kept, and the next pass asks again.
#[tokio::test]
async fn apple_busy_is_away_not_signed_out() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    for p in ["/setup/ws/1/validate", "/setup/ws/1/accountLogin"] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
    }
    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::Away);
    let before = asked(&server).await;
    assert_eq!(client.ensure_session().await, Kept::Away);
    assert!(
        asked(&server).await > before,
        "asked again on the next pass"
    );
    assert!(
        client.session().session_token.is_some(),
        "the session is kept"
    );
}

/// Nobody signed in: nothing is asked of Apple at all.
#[tokio::test]
async fn signed_out_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::SignedOut);
    assert_eq!(asked(&server).await, 0);
}

/// Apple's `/signin/init` answering `400` with service error -900007, "unknown", as it does
/// now and then (steilerDev/icloud-photos-sync#1237).
async fn mount_init_unknown_once(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "serviceErrors": [{ "code": "-900007", "message": "unknown" }]
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(server)
        .await;
}

/// The cookies and the session token lapsed: every look at the session is turned away.
async fn turned_away(server: &MockServer) {
    for p in ["/setup/ws/1/validate", "/setup/ws/1/accountLogin"] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(421))
            .mount(server)
            .await;
    }
}

/// One odd answer while a lapsed session signs back in is not a sign-out: the session is kept
/// and said as away, and the next attempt, once the sign-in's cooldown has passed, takes it up
/// again with no code.
#[tokio::test]
async fn an_unknown_answer_from_signin_init_is_away_not_signed_out() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    // Lapsed for the first pass (its heartbeat and its resume) and the cooled-down one after.
    for p in ["/setup/ws/1/validate", "/setup/ws/1/accountLogin"] {
        Mock::given(method("POST"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(421))
            .up_to_n_times(if p.ends_with("validate") { 4 } else { 2 })
            .with_priority(1)
            .mount(&server)
            .await;
    }
    mount_init_unknown_once(&server).await;
    mount_init(&server).await;
    mount_complete(&server, 200).await;
    mount_trust_and_login(&server).await;

    let mut client = client_for(&server, dir.path());
    let kept = client.ensure_session().await;
    assert_eq!(kept, Kept::Away);
    assert!(!kept.ended());
    assert!(
        client.session().session_token.is_some(),
        "the session is kept"
    );
    assert!(client.session().password.is_some(), "and the password");

    // Past the sign-in's own cooldown (five seconds after one failed attempt), the next pass
    // signs back in.
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(client.ensure_session().await, Kept::Alive);
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(requests_to(&reqs, "/signin/init").len(), 2);
    assert!(requests_to(&reqs, "/securitycode").is_empty(), "no code");
}

/// Opening the app while Apple gives that answer opens the library that was kept, not the
/// sign-in screen.
#[tokio::test]
async fn an_unknown_answer_at_launch_opens_the_library() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    turned_away(&server).await;
    mount_init_unknown_once(&server).await;

    let mut client = client_for(&server, dir.path());
    assert!(matches!(
        client.reopen().await.unwrap(),
        quince_auth::Reopen::Library
    ));
    assert!(client.session().session_token.is_some());
}

/// A locked account is a definite refusal: the session still ends, said as needing a sign-in.
#[tokio::test]
async fn a_locked_account_still_ends_the_session() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    turned_away(&server).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "serviceErrors": [{
                "code": "-20209",
                "message": "This Apple Account has been locked for security reasons."
            }]
        })))
        .mount(&server)
        .await;

    let mut client = client_for(&server, dir.path());
    let kept = client.ensure_session().await;
    assert_eq!(kept, Kept::SignIn);
    assert!(kept.ended());
}

const CK: &str = "https://p00-ckdatabasews.test.invalid:443";

/// `/validate` answering 200 with `body`.
async fn validate_answers(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Apple answering `/validate` with `"webservices": {}`, as it has for accounts with a beta-OS
/// device (icloudpd#1390): the services known are kept, on disk too, and the session is said as
/// away rather than taken up with no services.
#[tokio::test]
async fn an_empty_services_list_keeps_the_known_ones() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    validate_answers(
        &server,
        json!({ "webservices": {}, "configBag": { "urls": {} } }),
    )
    .await;
    let mut client = client_for(&server, dir.path());
    assert!(client.resume().await.is_err(), "away, not resumed");
    assert_eq!(
        client.session().service_url("ckdatabasews").as_deref(),
        Some(CK)
    );
    assert!(client.session().session_token.is_some());
    assert!(matches!(
        client.reopen().await.unwrap(),
        quince_auth::Reopen::Library
    ));

    // What was saved is the good list too.
    let again = client_for(&server, dir.path());
    assert_eq!(
        again.session().service_url("ckdatabasews").as_deref(),
        Some(CK)
    );
}

/// The same from `accountLogin` (icloudpd#1361): the session is taken up, its services kept.
#[tokio::test]
async fn an_empty_services_list_from_account_login_keeps_the_known_ones() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(421))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "1234567890" },
            "webservices": {}
        })))
        .mount(&server)
        .await;
    let mut client = client_for(&server, dir.path());
    assert!(client.resume().await.unwrap());
    assert_eq!(
        client.session().service_url("ckdatabasews").as_deref(),
        Some(CK)
    );
}

/// A partial list keeps the services it leaves out and takes the ones it names, new or moved.
#[tokio::test]
async fn a_partial_services_list_is_merged_key_by_key() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    let drive = "https://p00-drivews.test.invalid:443";
    validate_answers(
        &server,
        json!({ "webservices": { "drivews": { "url": drive, "status": "active" } } }),
    )
    .await;
    let mut client = client_for(&server, dir.path());
    assert!(client.resume().await.unwrap());
    assert_eq!(
        client.session().service_url("ckdatabasews").as_deref(),
        Some(CK)
    );
    assert_eq!(
        client.session().service_url("drivews").as_deref(),
        Some(drive)
    );

    // Apple moving a service to another partition: the new address is the one used.
    let moved = "https://p42-ckdatabasews.test.invalid:443";
    server.reset().await;
    validate_answers(
        &server,
        json!({ "webservices": { "ckdatabasews": { "url": moved, "status": "active" } } }),
    )
    .await;
    assert!(client.resume().await.unwrap());
    assert_eq!(
        client.session().service_url("ckdatabasews").as_deref(),
        Some(moved)
    );
    assert_eq!(
        client.session().service_url("drivews").as_deref(),
        Some(drive)
    );
}

/// `/signin/init` answering 200 with a body that isn't JSON is Apple's answer not understood,
/// and counts toward the sign-in's cooldown: the resume right after waits rather than starting
/// a sign-in again. Read as this computer failing, it never counted, and every resume (each
/// heartbeat, each sync) signed in again with no wait between (#242).
#[tokio::test]
async fn an_unreadable_signin_start_waits_before_the_next() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;

    let server = MockServer::start().await;
    turned_away(&server).await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let mut client = client_for(&server, dir.path());
    assert!(client.resume().await.is_err());
    assert!(client.session().password.is_some(), "the password is kept");
    assert!(matches!(
        client.resume().await,
        Err(quince_auth::Error::CoolingDown { .. })
    ));
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        requests_to(&reqs, "/signin/init").len(),
        1,
        "one sign-in started"
    );
}

/// `/validate` answering 200 for a session; `accountLogin` as `login` says.
async fn validate_ok_and_login(server: &MockServer, login: u16) {
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "1234567890" },
            "webservices": {
                "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active" }
            }
        })))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(if login == 200 {
            ResponseTemplate::new(200).set_body_json(json!({
                "dsInfo": { "dsid": "1234567890" },
                "webservices": {
                    "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active" }
                }
            }))
        } else {
            ResponseTemplate::new(login)
        })
        .mount(server)
        .await;
}

/// `/validate` vouching for a session every service refuses (measured 26–27 Nov 2026: 200 from
/// `/validate`, 421 "Missing X-APPLE-WEBAUTH-TOKEN cookie" from every service, every sync failing
/// quietly for a day). A refused pass has the next look trade the tokens for fresh cookies
/// (`accountLogin`) instead of asking `/validate` again; Apple taking them is the session alive.
#[tokio::test]
async fn a_session_validate_vouches_for_but_services_refuse_is_renewed() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    validate_ok_and_login(&server, 200).await;

    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    let reqs = server.received_requests().await.unwrap();
    assert!(
        requests_to(&reqs, "/accountLogin").is_empty(),
        "alive by `/validate`"
    );

    let g = client.session_generation();
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        requests_to(&reqs, "/accountLogin").len(),
        1,
        "the cookies traded afresh"
    );
    assert_eq!(
        client.session_generation(),
        g + 1,
        "counted, so a refused pass is sent again"
    );
}

/// Another refusal straight after a renewal (a slow pass still on the old cookies, or a service
/// refusing for its own reasons) waits out the gap: Apple is not asked to renew again at once.
/// A request the person is waiting on may try sooner, but never at every request.
#[tokio::test]
async fn a_refusal_straight_after_a_renewal_waits_out_the_gap() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    validate_ok_and_login(&server, 200).await;
    let mut client = client_for(&server, dir.path());
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    let logins = |reqs: &[Request]| requests_to(reqs, "/accountLogin").len();
    assert_eq!(logins(&server.received_requests().await.unwrap()), 1);
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    assert_eq!(
        logins(&server.received_requests().await.unwrap()),
        1,
        "not renewed again at once: looked at with `/validate`"
    );
    let g = client.session_generation();
    client.session_refused_now(g);
    client.ensure_session().await;
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        requests_to(&reqs, "/accountLogin").len(),
        1,
        "the person's request too waits the short gap"
    );
}

/// Apple out of reach while renewing (accountLogin 503): not a reason to sign in again with
/// the password, and not forgotten; and not asked again at every look either: the next try waits
/// out the gap, as after any try.
#[tokio::test]
async fn apple_out_of_reach_while_renewing_is_away_and_not_asked_again_at_once() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    validate_ok_and_login(&server, 503).await;
    mount_init(&server).await;
    let mut client = client_for(&server, dir.path());
    // A first look takes up the session as it is (`/validate`), as at launch.
    assert_eq!(client.ensure_session().await, Kept::Alive);
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::Away);
    for _ in 0..5 {
        client.ensure_session().await;
    }
    let reqs = server.received_requests().await.unwrap();
    assert!(
        requests_to(&reqs, "/signin/init").is_empty(),
        "no password sign-in for a 503"
    );
    assert_eq!(
        requests_to(&reqs, "/accountLogin").len(),
        1,
        "one try in the gap"
    );
}

/// Apple asking to slow down while renewing (accountLogin 429) says nothing about the tokens:
/// away, never the password, never a sign-out.
#[tokio::test]
async fn apple_asking_to_slow_down_while_renewing_is_not_a_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    validate_ok_and_login(&server, 429).await;
    mount_init(&server).await;
    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::Away);
    let reqs = server.received_requests().await.unwrap();
    assert!(
        requests_to(&reqs, "/signin/init").is_empty(),
        "no password sign-in for a 429"
    );
}

/// A 4xx from `accountLogin` other than 401/421 (a 400, say) is taken as the tokens refused only
/// when the next try meets one too: once, it may be a passing odd answer, so it is away, never
/// the password and never a sign-out.
#[tokio::test]
async fn a_single_odd_4xx_while_renewing_is_away_never_the_password() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    validate_ok_and_login(&server, 400).await;
    mount_init(&server).await;
    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::Away);
    let reqs = server.received_requests().await.unwrap();
    assert!(
        requests_to(&reqs, "/signin/init").is_empty(),
        "no password for one odd 400"
    );
}

/// Apple refusing the token (accountLogin 421) has ended the session: it is taken up as any lapse
/// is, with the kept password, quietly, and alive again with no code and nobody asked.
#[tokio::test]
async fn the_token_refused_is_taken_up_with_the_kept_password() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(421))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    validate_ok_and_login(&server, 200).await;
    mount_init(&server).await;
    mount_complete(&server, 200).await;
    mount_trust_and_login(&server).await;
    let mut client = client_for(&server, dir.path());
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(
        requests_to(&reqs, "/signin/complete").len(),
        1,
        "the kept password, once"
    );
    assert!(requests_to(&reqs, "/securitycode").is_empty(), "no code");
}

/// Apple refusing the renewal itself (accountLogin refused, then the kept password refused): the
/// session is over, said as needing a sign-in, and not asked about again.
#[tokio::test]
async fn apple_refusing_the_renewal_is_a_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    validate_ok_and_login(&server, 421).await;
    mount_init(&server).await;
    mount_complete(&server, 401).await;
    let mut client = client_for(&server, dir.path());
    client.session_refused(client.session_generation());
    assert_eq!(client.ensure_session().await, Kept::SignIn);
    let before = asked(&server).await;
    for _ in 0..5 {
        assert_eq!(client.ensure_session().await, Kept::SignIn);
    }
    assert_eq!(asked(&server).await, before, "nothing more asked of Apple");
}

/// New iCloud terms waiting: Apple answers `accountLogin` 200 but opens the session only part-way,
/// with no web cookies. That is no renewal: the count does not move, so the refused services are
/// not passed again for nothing at every pass.
#[tokio::test]
async fn a_session_held_by_new_terms_is_not_counted_as_renewed() {
    let dir = tempfile::tempdir().unwrap();
    signed_in(dir.path()).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "1234567890" },
            "webservices": {
                "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active" }
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dsInfo": { "dsid": "1234567890" },
            "termsUpdateNeeded": true,
            "webservices": {
                "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active" }
            }
        })))
        .mount(&server)
        .await;
    let mut client = client_for(&server, dir.path());
    assert_eq!(client.ensure_session().await, Kept::Alive);
    let g = client.session_generation();
    client.session_refused(g);
    client.ensure_session().await;
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(requests_to(&reqs, "/accountLogin").len(), 1);
    assert_eq!(
        client.session_generation(),
        g,
        "held by the terms, not renewed"
    );
}
