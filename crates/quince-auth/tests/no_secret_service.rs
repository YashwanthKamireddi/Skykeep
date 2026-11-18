//! A Linux desktop with no Secret Service at all (a bare window manager, a session over SSH):
//! the real keyring store, not the file stand-in the other tests use, with D-Bus pointed at a
//! socket that is not there. Other clients refuse to sign in here, or fall back to writing the
//! tokens to a plain file. Skykeep must sign in, hold the session in memory, say so, and leave
//! nothing on disk that a sign-in could be taken from.
//!
//! A file of its own: the D-Bus address is set for this whole process before anything reads it,
//! and so this never reaches the keyring of the machine it runs on either.
#![cfg(target_os = "linux")]

use quince_auth::{Client, Endpoints, SignInOutcome, Store};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// What the person typed at the password field.
const TYPED: &str = "hunter2-typed";
/// Everything Apple hands over that would sign someone in, if it were read off the disk.
const SECRETS: &[&str] = &[
    TYPED,
    "session-token-first",
    "session-token-trusted",
    "trust-token-kept",
    "scnt-from-init",
    "id-session-from-init",
    "COOKIE-SECRET-WEBAUTH",
    "someone@example.invalid",
];

fn b64(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD as E, Engine};
    E.encode(bytes)
}

async fn apple() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/init"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("scnt", "scnt-from-init")
                .insert_header("X-Apple-ID-Session-Id", "id-session-from-init")
                .set_body_json(json!({
                    "salt": b64(&[0x77; 16]),
                    "b": b64(&[0x5e; 256]),
                    "c": "challenge-c",
                    "iteration": 20173,
                    "protocol": "s2k",
                })),
        )
        .mount(&server)
        .await;
    // 409: the code is wanted.
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/signin/complete"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("X-Apple-Session-Token", "session-token-first")
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/appleauth/auth/verify/trusteddevice/securitycode"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/appleauth/auth/2sv/trust"))
        .respond_with(
            ResponseTemplate::new(204)
                .insert_header("X-Apple-TwoSV-Trust-Token", "trust-token-kept")
                .insert_header("X-Apple-Session-Token", "session-token-trusted"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/accountLogin"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "X-APPLE-WEBAUTH-USER=COOKIE-SECRET-WEBAUTH; Path=/")
                .set_body_json(json!({
                    "dsInfo": { "dsid": "1234567890" },
                    "webservices": {
                        "ckdatabasews": { "url": "https://p00-ckdatabasews.test.invalid:443", "status": "active" }
                    }
                })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/setup/ws/1/validate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer, dir: &std::path::Path) -> Client {
    let base = server.uri();
    let endpoints = Endpoints {
        auth_root: "https://idmsa.test.invalid".into(),
        home: "https://web.test.invalid".into(),
        auth: format!("{base}/appleauth/auth"),
        setup: format!("{base}/setup/ws/1"),
    };
    Client::with(dir, endpoints, Store::keyring(dir).unwrap()).unwrap()
}

/// Every file under `dir`, whatever it is named, read whole.
fn every_file(dir: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                todo.push(p);
            } else {
                out.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
    }
    out
}

#[tokio::test]
async fn with_no_secret_service_sign_in_works_in_memory_and_nothing_is_written_in_the_clear() {
    // No session bus where this points: every Secret Service call fails as it does on a
    // desktop that runs none. Set before the first call, which is where the connection is made.
    let nowhere = tempfile::tempdir().unwrap();
    std::env::set_var(
        "DBUS_SESSION_BUS_ADDRESS",
        format!("unix:path={}", nowhere.path().join("no-bus").display()),
    );

    let server = apple().await;
    let dir = tempfile::tempdir().unwrap();
    let mut c = client(&server, dir.path());
    assert!(
        !c.has_stored_session(),
        "nothing to read, and no hang reading it"
    );

    let outcome = c
        .sign_in("someone@example.invalid", TYPED)
        .await
        .expect("the password went through: no keyring is no reason to fail");
    assert_eq!(outcome, SignInOutcome::NeedsTwoFactor);
    c.submit_two_factor("123456")
        .await
        .expect("signed in, held in memory");
    assert!(c.session().webservices.is_some(), "signed in");
    assert_eq!(c.session().trust_token.as_deref(), Some("trust-token-kept"));
    let why = c.unsaved().expect("said that the session is not kept");
    assert!(!why.is_empty());

    // Still signed in while it runs: the heartbeat goes ahead, and it is still said.
    assert!(c.heartbeat().await.unwrap());
    assert!(c.unsaved().is_some());

    // Nothing on disk a sign-in could be taken from: no token, cookie, password or address,
    // under any file name (a plain session.json fallback is what other clients write).
    for (p, bytes) in every_file(dir.path()) {
        let text = String::from_utf8_lossy(&bytes);
        for secret in SECRETS {
            assert!(
                !text.contains(secret),
                "{secret:?} written in the clear to {}",
                p.display()
            );
        }
        assert_ne!(
            p.file_name().unwrap(),
            "cookies.json",
            "no cookie jar without a key to seal it"
        );
    }

    // And it is true that it is not kept: the next start finds no session.
    drop(c);
    let next = client(&server, dir.path());
    assert!(!next.has_stored_session());
    assert!(next.session().trust_token.is_none());
}
