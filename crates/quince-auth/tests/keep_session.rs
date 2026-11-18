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
    client.session_doubted();
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
