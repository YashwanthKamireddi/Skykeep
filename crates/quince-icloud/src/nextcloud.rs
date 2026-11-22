//! Signing in to a Nextcloud the way its own apps do (Login Flow v2).
//!
//! The person gives their server's address and approves Skykeep in their browser, on their own
//! Nextcloud, behind whatever two-factor sign-in it asks for. What comes back is an app
//! password for Skykeep alone, which they can revoke in Nextcloud's settings. Their own
//! password never passes through here.

use crate::caldav::safe_for_a_password;
use crate::{Error, Result};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;

/// The address a person typed, made one that can be signed in to: `cloud.example.org`, or a
/// page of Nextcloud pasted from the browser, comes down to where Nextcloud itself is.
pub fn normalise_server(raw: &str) -> Result<Url> {
    let mut url = normalise_address(raw)?;
    let path = url.path().to_string();
    let cut = [
        "/index.php",
        "/apps/",
        "/login",
        "/remote.php",
        "/ocs/",
        "/settings/",
        "/f/",
        "/s/",
    ]
    .iter()
    .filter_map(|mark| path.find(mark))
    .min()
    .unwrap_or(path.len());
    let base = path[..cut].trim_end_matches('/').to_string();
    url.set_path(&base);
    Ok(url)
}

/// An address as typed, checked: `https://` unless it says otherwise, no password in it, and
/// only somewhere a password can be sent.
pub fn normalise_address(raw: &str) -> Result<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(Error::Refused("Enter your server’s address.".into()));
    }
    let text = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let mut url = Url::parse(&text).map_err(|_| {
        Error::Refused("That address can’t be read. Check it and try again.".into())
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::Refused(
            "Use an address that begins with https://.".into(),
        ));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(Error::Refused(
            "That address has no server in it. Check it and try again.".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::Refused(
            "Leave the username and password out of the address.".into(),
        ));
    }
    if !safe_for_a_password(&url) {
        return Err(Error::Refused(
            "Skykeep only sends a password over an encrypted connection. Use an https:// address."
                .into(),
        ));
    }
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// A server's address with `tail` after it.
fn at(server: &Url, tail: &str) -> Result<Url> {
    let joined = format!("{}/{tail}", server.as_str().trim_end_matches('/'));
    Url::parse(&joined).map_err(|_| Error::Refused("That address can’t be read.".into()))
}

/// Where the calendars of a Nextcloud are listed from.
pub fn dav_root(server: &Url) -> Result<Url> {
    at(server, "remote.php/dav/")
}

/// A place the server named, on the server the person named: a Nextcloud behind a proxy often
/// names its own inside address, and none of those is where this computer should go.
fn on_server(server: &Url, named: &str) -> Result<Url> {
    let named = Url::parse(named)
        .map_err(|_| Error::Protocol("the server named an address that can't be read".into()))?;
    let mut url = server.clone();
    url.set_path(named.path());
    url.set_query(named.query());
    Ok(url)
}

/// A sign-in under way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    /// The page the person approves Skykeep on.
    pub login: String,
    server: Url,
    poll: Url,
    token: String,
}

#[derive(Deserialize)]
struct Begun {
    poll: PollAt,
    login: String,
}

#[derive(Deserialize)]
struct PollAt {
    token: String,
    endpoint: String,
}

/// What approving Skykeep gave it.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Granted {
    pub login_name: String,
    pub app_password: String,
}

impl std::fmt::Debug for Granted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Granted({})", self.login_name)
    }
}

/// Ask the server to start a sign-in. `http` is the account's own client.
pub async fn begin(http: &Client, server: &Url) -> Result<Flow> {
    let response = http.post(at(server, "index.php/login/v2")?).send().await?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND || status.is_redirection() {
        return Err(Error::Refused(
            "There’s no Nextcloud at that address. Check it and try again.".into(),
        ));
    }
    if !status.is_success() {
        return Err(Error::Refused(format!(
            "That server couldn’t start a sign-in (it answered {}).",
            status.as_u16()
        )));
    }
    let text = crate::bounded::reply(response).await?;
    let begun: Begun = serde_json::from_str(&text).map_err(|_| {
        Error::Refused("There’s no Nextcloud at that address. Check it and try again.".into())
    })?;
    Ok(Flow {
        login: on_server(server, &begun.login)?.to_string(),
        poll: on_server(server, &begun.poll.endpoint)?,
        token: begun.poll.token,
        server: server.clone(),
    })
}

/// Whether the person has approved Skykeep yet. `None`: not yet.
pub async fn poll(http: &Client, flow: &Flow) -> Result<Option<Granted>> {
    let response = http
        .post(flow.poll.clone())
        .form(&[("token", flow.token.as_str())])
        .send()
        .await?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(Error::Refused(format!(
            "The sign-in stopped (the server answered {}).",
            status.as_u16()
        )));
    }
    let granted = response
        .json()
        .await
        .map_err(|_| Error::Protocol("the server's answer can't be read".into()))?;
    Ok(Some(granted))
}

/// Let go of the app password, so nothing is left signed in on the server. What can't be done
/// now is for the person to do in Nextcloud's settings.
pub async fn revoke(http: &Client, server: &str, user: &str, password: &str) -> Result<()> {
    let server =
        Url::parse(server).map_err(|_| Error::Refused("That address can’t be read.".into()))?;
    let response = http
        .delete(at(&server, "ocs/v2.php/core/apppassword")?)
        .basic_auth(user, Some(password))
        .header("OCS-APIRequest", "true")
        .send()
        .await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(Error::Refused(
            "The server didn’t let go of the sign-in.".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn http() -> Client {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    #[test]
    fn whatever_is_pasted_comes_down_to_the_server() {
        let n = |s: &str| normalise_server(s).unwrap().to_string();
        assert_eq!(n("cloud.example.org"), "https://cloud.example.org/");
        assert_eq!(
            n("  https://cloud.example.org/ "),
            "https://cloud.example.org/"
        );
        assert_eq!(
            n("https://cloud.example.org/index.php/apps/calendar/dayGridMonth/now"),
            "https://cloud.example.org/"
        );
        assert_eq!(
            n("https://example.org/nextcloud/apps/files/?dir=/"),
            "https://example.org/nextcloud"
        );
        assert_eq!(
            n("https://example.org/nextcloud/"),
            "https://example.org/nextcloud"
        );
        assert_eq!(
            n("https://example.org:8443/login"),
            "https://example.org:8443/"
        );
        assert_eq!(
            n("http://192.168.1.20/nextcloud"),
            "http://192.168.1.20/nextcloud"
        );
    }

    #[test]
    fn an_address_a_password_cant_safely_go_to_is_refused_in_words() {
        for bad in [
            "",
            "http://cloud.example.org",
            "ftp://example.org",
            "https://me:pw@example.org",
        ] {
            let said = normalise_server(bad).unwrap_err().to_string();
            assert!(!said.is_empty(), "{bad}");
        }
        assert!(normalise_server("http://cloud.example.org")
            .unwrap_err()
            .to_string()
            .contains("encrypted"));
    }

    /// The whole flow, in the shapes Nextcloud answers with — its addresses naming an inside
    /// host, as one behind a proxy does.
    #[tokio::test]
    async fn signing_in_starts_waits_and_ends_with_an_app_password() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/nextcloud/index.php/login/v2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "poll": { "token": "tok", "endpoint": "http://nextcloud.internal/nextcloud/index.php/login/v2/poll" },
                "login": "http://nextcloud.internal/nextcloud/index.php/login/v2/flow/abc"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/nextcloud/index.php/login/v2/poll"))
            .and(body_string("token=tok"))
            .respond_with(ResponseTemplate::new(404))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/nextcloud/index.php/login/v2/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "server": "http://nextcloud.internal/nextcloud",
                "loginName": "felix",
                "appPassword": "secret-app-password"
            })))
            .mount(&server)
            .await;
        let base = Url::parse(&format!("{}/nextcloud", server.uri())).unwrap();
        let flow = begin(&http(), &base).await.unwrap();
        assert_eq!(
            flow.login,
            format!("{}/nextcloud/index.php/login/v2/flow/abc", server.uri())
        );
        assert_eq!(
            poll(&http(), &flow).await.unwrap(),
            None,
            "not approved yet"
        );
        let granted = poll(&http(), &flow).await.unwrap().unwrap();
        assert_eq!(granted.login_name, "felix");
        assert_eq!(granted.app_password, "secret-app-password");
    }

    #[tokio::test]
    async fn another_kind_of_server_is_said_to_be_one() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>welcome</html>"))
            .mount(&server)
            .await;
        let base = Url::parse(&server.uri()).unwrap();
        let said = begin(&http(), &base).await.unwrap_err().to_string();
        assert!(said.contains("no Nextcloud"), "{said}");
    }

    #[test]
    fn the_calendars_live_under_the_servers_own_path() {
        let s = Url::parse("https://example.org/nextcloud").unwrap();
        assert_eq!(
            dav_root(&s).unwrap().as_str(),
            "https://example.org/nextcloud/remote.php/dav/"
        );
    }
}
