//! CalDAV: the calendars of a Nextcloud account or any other CalDAV server.
//!
//! Nothing here is Apple's. Every request goes through a client that reaches the account's own
//! hosts and nothing else (`quince_auth::account_only`), and the server's answers are treated
//! as anything from the network is: what can't be read is left out or refused, never a panic.

use crate::{Error, Result};
use quick_xml::events::Event as Xml;
use quick_xml::{Reader, XmlVersion};
use reqwest::header::{HeaderValue, CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH, LOCATION};
use reqwest::{Client, Method, Response, StatusCode, Url};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};

/// The most one answer is read to. A calendar's listing is a few hundred kilobytes; a server
/// sending more than this is sending something else.
const MOST_BYTES: usize = 64 * 1024 * 1024;

/// How many events are asked for in one request.
const BATCH: usize = 100;

const NS: &str = r#"xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:i="http://apple.com/ns/ical/""#;

const WRONG_PASSWORD: &str = "That username or password wasn’t accepted.";

fn propfind_of(props: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><d:propfind {NS}><d:prop>{props}</d:prop></d:propfind>"#
    )
}

fn method(name: &'static str) -> Method {
    Method::from_bytes(name.as_bytes()).unwrap_or(Method::GET)
}

/// One `<response>` of a multistatus answer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Answer {
    href: String,
    props: HashMap<String, Prop>,
}

/// One property of it, by what a caller needs of one: its text, the addresses in it, the names
/// of the elements in it, and the components it names.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Prop {
    text: String,
    hrefs: Vec<String>,
    children: Vec<String>,
    comps: Vec<String>,
}

fn local(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// The text of `&amp;` and its kind.
fn entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    })
}

/// Reading a multistatus answer.
#[derive(Default)]
struct Walk {
    out: Vec<Answer>,
    current: Option<Answer>,
    /// The elements open, by name.
    stack: Vec<String>,
    /// What the `propstat` being read holds, kept only if its status says found.
    found: HashMap<String, Prop>,
    good: bool,
    /// How deep the `prop` being read is.
    at_prop: Option<usize>,
    prop_name: Option<String>,
    prop: Prop,
    text: String,
}

impl Walk {
    fn open(&mut self, name: String, comp: Option<String>, empty: bool) {
        if name == "response" && !empty {
            self.current = Some(Answer::default());
        } else if name == "propstat" && !empty {
            self.found.clear();
            self.good = true;
        }
        match self.at_prop {
            Some(depth) if self.stack.len() == depth + 1 => {
                if empty {
                    self.found.insert(name.clone(), Prop::default());
                } else {
                    self.prop_name = Some(name.clone());
                    self.prop = Prop::default();
                }
            }
            Some(_) => {
                self.prop.children.push(name.clone());
                if let Some(c) = comp {
                    self.prop.comps.push(c);
                }
            }
            None => {
                if name == "prop" && !empty {
                    self.at_prop = Some(self.stack.len());
                }
            }
        }
        self.text.clear();
        if !empty {
            self.stack.push(name);
        }
    }

    fn close(&mut self) {
        let Some(name) = self.stack.pop() else { return };
        let depth = self.stack.len();
        match self.at_prop {
            Some(d) if d == depth && name == "prop" => self.at_prop = None,
            // A property's own end.
            Some(d) if depth == d + 1 => {
                if let Some(n) = self.prop_name.take() {
                    if self.prop.text.is_empty() {
                        self.prop.text = self.text.trim().to_string();
                    }
                    self.found.insert(n, std::mem::take(&mut self.prop));
                }
            }
            Some(d) if depth > d + 1 => {
                if name == "href" {
                    self.prop.hrefs.push(self.text.trim().to_string());
                } else if self.prop.text.is_empty() {
                    self.prop.text = self.text.trim().to_string();
                }
            }
            _ => match name.as_str() {
                "href" => {
                    if let Some(a) = self.current.as_mut().filter(|a| a.href.is_empty()) {
                        a.href = self.text.trim().to_string();
                    }
                }
                "status" => {
                    self.good = self
                        .text
                        .split_whitespace()
                        .nth(1)
                        .is_some_and(|c| c.starts_with('2'));
                }
                "propstat" => {
                    if self.good {
                        if let Some(a) = self.current.as_mut() {
                            a.props.extend(self.found.drain());
                        }
                    }
                    self.found.clear();
                }
                "response" => {
                    if let Some(a) = self.current.take() {
                        self.out.push(a);
                    }
                }
                _ => {}
            },
        }
        self.text.clear();
    }
}

fn comp_name(e: &quick_xml::events::BytesStart<'_>, element: &str) -> Option<String> {
    if element != "comp" {
        return None;
    }
    let attribute = e.try_get_attribute("name").ok().flatten()?;
    attribute
        .normalized_value(XmlVersion::default())
        .ok()
        .map(|v| v.into_owned())
}

/// Read a multistatus answer, by local element names: servers differ in their prefixes, and
/// the names asked for are not ambiguous. Only properties a `propstat` reports as found count.
fn multistatus(xml: &str) -> Result<Vec<Answer>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut walk = Walk::default();
    loop {
        match reader.read_event() {
            Err(_) => return Err(Error::Protocol("the server's answer can't be read".into())),
            Ok(Xml::Eof) => break,
            Ok(Xml::Start(e)) => {
                let name = local(e.local_name().as_ref());
                let comp = comp_name(&e, &name);
                walk.open(name, comp, false);
            }
            Ok(Xml::Empty(e)) => {
                let name = local(e.local_name().as_ref());
                let comp = comp_name(&e, &name);
                walk.open(name, comp, true);
            }
            Ok(Xml::Text(t)) => walk.text.push_str(&t.xml10_content()),
            Ok(Xml::CData(t)) => walk.text.push_str(&t.xml10_content()),
            Ok(Xml::GeneralRef(r)) => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    walk.text.push(c);
                } else if let Some(s) = entity(&r.xml10_content()) {
                    walk.text.push_str(s);
                }
            }
            Ok(Xml::End(_)) => walk.close(),
            Ok(_) => {}
        }
    }
    Ok(walk.out)
}

/// A calendar on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCalendar {
    pub url: String,
    pub title: String,
    pub colour: Option<String>,
    pub ctag: Option<String>,
    pub writable: bool,
    pub order: i64,
}

/// One event's file on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub href: String,
    pub etag: Option<String>,
    pub ics: String,
}

/// `#RRGGBB`, from what a server calls a colour (`#RRGGBBAA` usually).
fn colour_of(raw: &str) -> Option<String> {
    let c = raw.trim();
    (c.starts_with('#')
        && (c.len() == 7 || c.len() == 9)
        && c[1..].bytes().all(|b| b.is_ascii_hexdigit()))
    .then(|| c[..7].to_ascii_lowercase())
}

fn sha1_hex(text: &str) -> String {
    let mut out = String::new();
    for b in Sha1::digest(text.as_bytes()) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The path a calendar or event lives at, whole address or not.
#[must_use]
pub fn path_of(address: &str) -> String {
    Url::parse(address).map_or_else(|_| address.to_string(), |u| u.path().to_string())
}

/// A calendar's guid: stable for as long as it lives at the same path.
#[must_use]
pub fn calendar_guid(account: &str, url: &str) -> String {
    let hash = sha1_hex(&path_of(url));
    format!("{}{account}:{}", quince_core::dav::PREFIX, &hash[..12])
}

/// An event's guid, from where it lives. Its file's own name where that is plain — a new event
/// is named here, so its guid is known before the server has it — and a hash where not. Never
/// with `__` in it, which is what marks one occurrence of a repeating event.
#[must_use]
pub fn event_guid(calendar: &str, href: &str) -> String {
    let file = href
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(href);
    let stem = file.strip_suffix(".ics").unwrap_or(file);
    let plain = !stem.is_empty()
        && stem.len() <= 64
        && stem.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if plain {
        format!("{calendar}:{stem}")
    } else {
        format!("{calendar}:{}", &sha1_hex(file)[..32])
    }
}

/// An address the server gave, read against the one it answered.
pub fn join(base: &str, href: &str) -> Result<String> {
    Url::parse(base)
        .and_then(|b| b.join(href))
        .map(String::from)
        .map_err(|_| Error::Protocol("the server named an address that can't be read".into()))
}

fn with_slash(url: String) -> String {
    if url.ends_with('/') {
        url
    } else {
        format!("{url}/")
    }
}

/// What a status means when it is not success.
fn failure(status: StatusCode) -> Error {
    match status.as_u16() {
        401 => Error::ReauthRequired,
        403 => Error::Refused("The server didn’t allow this change.".into()),
        404 | 410 => Error::NotFound,
        412 => Error::Conflict,
        507 => Error::Refused("The server has run out of space.".into()),
        423 | 429 | 500..=599 => Error::Busy(None),
        code => Error::Refused(format!("The server turned that down (it answered {code}).")),
    }
}

/// An account's connection. Not `Debug`: it holds the password.
#[derive(Clone)]
pub struct Remote {
    http: Client,
    user: String,
    password: String,
}

impl Remote {
    #[must_use]
    pub fn new(http: Client, user: &str, password: &str) -> Self {
        Self {
            http,
            user: user.to_string(),
            password: password.to_string(),
        }
    }

    async fn raw(
        &self,
        verb: Method,
        url: &str,
        depth: Option<&'static str>,
        body: Option<String>,
    ) -> Result<Response> {
        let mut r = self
            .http
            .request(verb, url)
            .basic_auth(&self.user, Some(&self.password));
        if let Some(d) = depth {
            r = r.header("Depth", d);
        }
        if let Some(b) = body {
            r = r
                .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                .body(b);
        }
        Ok(r.send().await?)
    }

    async fn read(mut response: Response) -> Result<String> {
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MOST_BYTES {
                return Err(Error::Protocol("the server's answer is too large".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn ask(
        &self,
        verb: Method,
        url: &str,
        depth: &'static str,
        body: String,
    ) -> Result<Vec<Answer>> {
        let response = self.raw(verb, url, Some(depth), Some(body)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(failure(status));
        }
        multistatus(&Self::read(response).await?)
    }

    /// The account's calendars of events, from where they are listed. A list of tasks alone,
    /// and the server's own inbox, outbox and bin, are no calendars here.
    pub async fn calendars(&self, home: &str) -> Result<Vec<RemoteCalendar>> {
        let body = propfind_of(
            "<d:resourcetype/><d:displayname/><cs:getctag/><i:calendar-color/><i:calendar-order/>\
             <c:supported-calendar-component-set/><d:current-user-privilege-set/>",
        );
        let answers = self.ask(method("PROPFIND"), home, "1", body).await?;
        let mut out = Vec::new();
        for a in answers {
            let is_calendar = a
                .props
                .get("resourcetype")
                .is_some_and(|p| p.children.iter().any(|c| c == "calendar"));
            if !is_calendar {
                continue;
            }
            let holds_events = a
                .props
                .get("supported-calendar-component-set")
                .is_none_or(|p| {
                    p.comps.is_empty() || p.comps.iter().any(|c| c.eq_ignore_ascii_case("VEVENT"))
                });
            if !holds_events {
                continue;
            }
            let writable = a.props.get("current-user-privilege-set").is_none_or(|p| {
                p.children
                    .iter()
                    .any(|c| matches!(c.as_str(), "write" | "write-content" | "all"))
            });
            let text = |name: &str| {
                a.props
                    .get(name)
                    .map(|p| p.text.clone())
                    .filter(|t| !t.is_empty())
            };
            let url = with_slash(join(home, &a.href)?);
            out.push(RemoteCalendar {
                title: text("displayname").unwrap_or_else(|| {
                    a.href
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or("Calendar")
                        .to_string()
                }),
                colour: text("calendar-color").as_deref().and_then(colour_of),
                ctag: text("getctag"),
                order: text("calendar-order")
                    .and_then(|o| o.parse().ok())
                    .unwrap_or(0),
                writable,
                url,
            });
        }
        Ok(out)
    }

    /// Every event's address in a calendar, and its tag.
    pub async fn etags(&self, calendar: &str) -> Result<HashMap<String, Option<String>>> {
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><c:calendar-query {NS}><d:prop><d:getetag/></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"/></c:comp-filter></c:filter></c:calendar-query>"#
        );
        let answers = self.ask(method("REPORT"), calendar, "1", body).await?;
        Ok(answers
            .into_iter()
            // Never the calendar itself, which some servers list among its contents.
            .filter(|a| !a.href.is_empty() && !a.href.ends_with('/'))
            .map(|a| {
                let tag = a
                    .props
                    .get("getetag")
                    .map(|p| p.text.clone())
                    .filter(|t| !t.is_empty());
                (a.href, tag)
            })
            .collect())
    }

    /// These events' files, a batch at a time.
    pub async fn fetch(&self, calendar: &str, hrefs: &[String]) -> Result<Vec<Resource>> {
        let mut out = Vec::new();
        for batch in hrefs.chunks(BATCH) {
            let wanted: String = batch
                .iter()
                .map(|h| format!("<d:href>{}</d:href>", xml_text(h)))
                .collect();
            let body = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><c:calendar-multiget {NS}><d:prop><d:getetag/><c:calendar-data/></d:prop>{wanted}</c:calendar-multiget>"#
            );
            let answers = self.ask(method("REPORT"), calendar, "1", body).await?;
            for a in answers {
                let Some(ics) = a
                    .props
                    .get("calendar-data")
                    .map(|p| p.text.clone())
                    .filter(|t| t.contains("BEGIN:VCALENDAR"))
                else {
                    continue;
                };
                let etag = a
                    .props
                    .get("getetag")
                    .map(|p| p.text.clone())
                    .filter(|t| !t.is_empty());
                out.push(Resource {
                    href: a.href,
                    etag,
                    ics,
                });
            }
        }
        Ok(out)
    }

    /// One event's file as the server holds it now.
    pub async fn get(&self, url: &str) -> Result<Resource> {
        let response = self.raw(Method::GET, url, None, None).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(failure(status));
        }
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string);
        let ics = Self::read(response).await?;
        if !ics.contains("BEGIN:VCALENDAR") {
            return Err(Error::Protocol("the server sent something else".into()));
        }
        Ok(Resource {
            href: path_of(url),
            etag,
            ics,
        })
    }

    async fn put(
        &self,
        url: &str,
        ics: &str,
        header: (reqwest::header::HeaderName, HeaderValue),
    ) -> Result<()> {
        let response = self
            .http
            .put(url)
            .basic_auth(&self.user, Some(&self.password))
            .header(CONTENT_TYPE, "text/calendar; charset=utf-8")
            .header(header.0, header.1)
            .body(ics.to_string())
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(failure(status))
        }
    }

    /// A new event's file. `Conflict`: one is already there.
    pub async fn create(&self, url: &str, ics: &str) -> Result<()> {
        self.put(url, ics, (IF_NONE_MATCH, HeaderValue::from_static("*")))
            .await
    }

    /// An event's file changed, only if it is still the version `etag` names. `Conflict`:
    /// it has changed since.
    pub async fn update(&self, url: &str, ics: &str, etag: Option<&str>) -> Result<()> {
        let tag = match etag.and_then(|t| HeaderValue::from_str(t).ok()) {
            Some(tag) => tag,
            // No tag to hold it to: only that something is there.
            None => HeaderValue::from_static("*"),
        };
        self.put(url, ics, (IF_MATCH, tag)).await
    }

    /// An event's file gone, only if it is still the version `etag` names.
    pub async fn delete(&self, url: &str, etag: Option<&str>) -> Result<()> {
        let mut r = self
            .http
            .delete(url)
            .basic_auth(&self.user, Some(&self.password));
        if let Some(tag) = etag.and_then(|t| HeaderValue::from_str(t).ok()) {
            r = r.header(IF_MATCH, tag);
        }
        let status = r.send().await?.status();
        if status.is_success() || status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
            Ok(())
        } else {
            Err(failure(status))
        }
    }

    /// A calendar renamed or recoloured.
    pub async fn set_calendar(
        &self,
        url: &str,
        title: Option<&str>,
        colour: Option<&str>,
    ) -> Result<()> {
        let mut props = String::new();
        if let Some(t) = title {
            props.push_str(&format!("<d:displayname>{}</d:displayname>", xml_text(t)));
        }
        if let Some(c) = colour {
            props.push_str(&format!(
                "<i:calendar-color>{}</i:calendar-color>",
                xml_text(c)
            ));
        }
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><d:propertyupdate {NS}><d:set><d:prop>{props}</d:prop></d:set></d:propertyupdate>"#
        );
        let response = self.raw(method("PROPPATCH"), url, None, Some(body)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(failure(status));
        }
        Ok(())
    }

    /// A calendar deleted, with everything in it.
    pub async fn delete_calendar(&self, url: &str) -> Result<()> {
        self.delete(url, None).await
    }
}

fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Where an account's calendars were found, and the hosts it took to find them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub home: String,
    pub hosts: Vec<String>,
}

fn host_name(url: &Url) -> Option<String> {
    url.host_str()
        .map(|h| h.trim_matches(['[', ']']).to_ascii_lowercase())
}

/// Whether a password may be sent to `url`: encrypted, or plainly within the local network,
/// named by address so a name can't lead it elsewhere.
#[must_use]
pub fn safe_for_a_password(url: &Url) -> bool {
    url.scheme() == "https"
        || (url.scheme() == "http"
            && url
                .host_str()
                .map(|h| h.trim_matches(['[', ']']))
                .and_then(|h| h.parse::<std::net::IpAddr>().ok())
                .is_some_and(|ip| !is_public(ip)))
}

fn is_public(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            // Shared address space (100.64.0.0/10) is not among them: tailnets use it, but so
            // does carrier-grade NAT at many providers, where a password in the clear would
            // cross the provider's network. A tailnet's server is reached over HTTPS.
            !(v4.is_private() || v4.is_loopback() || v4.is_link_local())
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// Builds the client for a set of hosts: `quince_auth::account_only`, from the app.
pub type ClientFor<'a> = &'a (dyn Fn(&[String]) -> Result<Client> + Send + Sync);

/// Find where an account's calendars are listed, trying each address until one answers.
pub async fn discover(
    client_for: ClientFor<'_>,
    starts: &[Url],
    user: &str,
    password: &str,
) -> Result<Found> {
    let mut last =
        Error::Refused("No calendars were found at that address. Check it and try again.".into());
    for start in starts {
        match discover_from(client_for, start, user, password).await {
            Ok(found) => return Ok(found),
            // A wrong password is the same wrong password at every address.
            Err(Error::ReauthRequired) => return Err(Error::Refused(WRONG_PASSWORD.into())),
            Err(e @ Error::Network(_)) => last = e,
            Err(_) => {}
        }
    }
    Err(last)
}

async fn discover_from(
    client_for: ClientFor<'_>,
    start: &Url,
    user: &str,
    password: &str,
) -> Result<Found> {
    let body = propfind_of("<d:current-user-principal/><c:calendar-home-set/>");
    let mut hosts: Vec<String> = host_name(start).into_iter().collect();
    let mut url = start.clone();
    // Redirects read here, not by the client: a redirect it followed would come back a GET
    // with no password. `.well-known/caldav` is one by design (RFC 6764).
    for _ in 0..6 {
        let remote = Remote::new(client_for(&hosts)?, user, password);
        let response = remote
            .raw(
                method("PROPFIND"),
                url.as_str(),
                Some("0"),
                Some(body.clone()),
            )
            .await?;
        let status = response.status();
        if status.is_redirection() {
            let next = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|l| url.join(l).ok())
                .ok_or_else(|| Error::Protocol("a redirect to nowhere".into()))?;
            if !safe_for_a_password(&next) {
                return Err(Error::Refused(
                    "That server redirects to an address without encryption.".into(),
                ));
            }
            if let Some(h) = host_name(&next) {
                if !hosts.contains(&h) {
                    hosts.push(h);
                }
            }
            url = next;
            continue;
        }
        if !status.is_success() {
            return Err(failure(status));
        }
        let answers = multistatus(&Remote::read(response).await?)?;
        let prop_href = |name: &str| {
            answers
                .iter()
                .find_map(|a| a.props.get(name).and_then(|p| p.hrefs.first().cloned()))
        };
        if let Some(home) = prop_href("calendar-home-set") {
            return Ok(Found {
                home: with_slash(join(url.as_str(), &home)?),
                hosts,
            });
        }
        let principal = prop_href("current-user-principal")
            .ok_or_else(|| Error::Protocol("no account was named at that address".into()))?;
        let principal = join(url.as_str(), &principal)?;
        let body = propfind_of("<c:calendar-home-set/>");
        let answers = remote
            .ask(method("PROPFIND"), &principal, "0", body)
            .await?;
        let home = answers
            .iter()
            .find_map(|a| {
                a.props
                    .get("calendar-home-set")
                    .and_then(|p| p.hrefs.first().cloned())
            })
            .ok_or_else(|| Error::Protocol("that account has no calendars".into()))?;
        return Ok(Found {
            home: with_slash(join(&principal, &home)?),
            hosts,
        });
    }
    Err(Error::Protocol("too many redirects".into()))
}

/// What a pass has to fetch and let go of, from what the server lists and what is held here.
#[must_use]
pub fn differences(
    theirs: &HashMap<String, Option<String>>,
    ours: &HashMap<String, Option<String>>,
) -> (Vec<String>, Vec<String>) {
    let mut wanted: Vec<String> = theirs
        .iter()
        .filter(|(href, tag)| ours.get(*href) != Some(*tag) || tag.is_none())
        .map(|(href, _)| href.clone())
        .collect();
    let listed: HashSet<&String> = theirs.keys().collect();
    let mut gone: Vec<String> = ours
        .keys()
        .filter(|h| !listed.contains(h))
        .cloned()
        .collect();
    wanted.sort();
    gone.sort();
    (wanted, gone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method as verb, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// As a Nextcloud 30 lists a calendar home: a calendar, a task list, a read-only share, the
    /// scheduling inbox and the bin. Prefixes as Sabre writes them.
    const HOME: &str = r##"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:cal="urn:ietf:params:xml:ns:caldav" xmlns:cs="http://calendarserver.org/ns/" xmlns:x1="http://apple.com/ns/ical/" xmlns:oc="http://owncloud.org/ns" xmlns:nc="http://nextcloud.org/ns">
 <d:response><d:href>/remote.php/dav/calendars/felix/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:displayname/><cs:getctag/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response><d:href>/remote.php/dav/calendars/felix/personal/</d:href>
  <d:propstat><d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype>
    <d:displayname>Pers&#246;nlich &amp; Co</d:displayname>
    <cs:getctag>http://sabre.io/ns/sync/42</cs:getctag>
    <x1:calendar-color>#0082C9FF</x1:calendar-color>
    <x1:calendar-order>3</x1:calendar-order>
    <cal:supported-calendar-component-set><cal:comp name="VEVENT"/><cal:comp name="VTODO"/></cal:supported-calendar-component-set>
    <d:current-user-privilege-set><d:privilege><d:read/></d:privilege><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response><d:href>/remote.php/dav/calendars/felix/tasks/</d:href>
  <d:propstat><d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Tasks</d:displayname>
    <cal:supported-calendar-component-set><cal:comp name="VTODO"/></cal:supported-calendar-component-set>
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response><d:href>/remote.php/dav/calendars/felix/shared_with_me/</d:href>
  <d:propstat><d:prop>
    <d:resourcetype><d:collection/><cal:calendar/></d:resourcetype><d:displayname>Team</d:displayname>
    <cal:supported-calendar-component-set><cal:comp name="VEVENT"/></cal:supported-calendar-component-set>
    <d:current-user-privilege-set><d:privilege><d:read/></d:privilege></d:current-user-privilege-set>
  </d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response><d:href>/remote.php/dav/calendars/felix/inbox/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/><cal:schedule-inbox/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response><d:href>/remote.php/dav/calendars/felix/trashbin/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/><nc:trash-bin/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"##;

    fn client(_: &[String]) -> Result<Client> {
        Ok(Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?)
    }

    /// The header a client signs a request with, built from the test's own name and password
    /// rather than written out, so no encoded credential sits in the code.
    fn basic(user: &str, password: &str) -> String {
        use base64::Engine as _;
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
        )
    }

    fn remote() -> Remote {
        Remote::new(client(&[]).unwrap(), "felix", "app-password")
    }

    #[tokio::test]
    async fn a_calendar_home_is_read_as_a_nextcloud_lists_it() {
        let server = MockServer::start().await;
        Mock::given(verb("PROPFIND"))
            .and(path("/remote.php/dav/calendars/felix/"))
            .and(header("Depth", "1"))
            .and(header(
                "Authorization",
                basic("felix", "app-password").as_str(),
            ))
            .respond_with(ResponseTemplate::new(207).set_body_string(HOME))
            .mount(&server)
            .await;
        let home = format!("{}/remote.php/dav/calendars/felix/", server.uri());
        let found = remote().calendars(&home).await.unwrap();
        assert_eq!(found.len(), 2, "{found:?}");
        let personal = &found[0];
        assert_eq!(personal.title, "Persönlich & Co");
        assert_eq!(personal.colour.as_deref(), Some("#0082c9"));
        assert_eq!(personal.ctag.as_deref(), Some("http://sabre.io/ns/sync/42"));
        assert_eq!(personal.order, 3);
        assert!(personal.writable);
        assert_eq!(
            personal.url,
            format!("{}/remote.php/dav/calendars/felix/personal/", server.uri())
        );
        assert_eq!(found[1].title, "Team");
        assert!(!found[1].writable, "shared for reading only");
    }

    #[tokio::test]
    async fn the_files_are_read_with_their_tags_and_entities_undone() {
        let server = MockServer::start().await;
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Tom &amp; Jerry\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let body = format!(
            r#"<d:multistatus xmlns:d="DAV:" xmlns:cal="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/c/a.ics</d:href><d:propstat><d:prop><d:getetag>&quot;tag-1&quot;</d:getetag><cal:calendar-data>{ics}</cal:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response><d:response><d:href>/c/gone.ics</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response></d:multistatus>"#
        );
        Mock::given(verb("REPORT"))
            .respond_with(ResponseTemplate::new(207).set_body_string(body))
            .mount(&server)
            .await;
        let url = format!("{}/c/", server.uri());
        let got = remote()
            .fetch(&url, &["/c/a.ics".into(), "/c/gone.ics".into()])
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].etag.as_deref(), Some("\"tag-1\""));
        assert!(got[0].ics.contains("SUMMARY:Tom & Jerry"), "{}", got[0].ics);
        assert!(got[0].ics.contains("\r\n") || got[0].ics.contains('\n'));
    }

    #[tokio::test]
    async fn a_write_is_held_to_the_version_it_was_made_from() {
        let server = MockServer::start().await;
        Mock::given(verb("PUT"))
            .and(path("/c/a.ics"))
            .and(header("If-Match", "\"old\""))
            .respond_with(ResponseTemplate::new(412))
            .mount(&server)
            .await;
        Mock::given(verb("PUT"))
            .and(path("/c/b.ics"))
            .and(header("If-None-Match", "*"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;
        let a = format!("{}/c/a.ics", server.uri());
        assert!(matches!(
            remote().update(&a, "x", Some("\"old\"")).await,
            Err(Error::Conflict)
        ));
        let b = format!("{}/c/b.ics", server.uri());
        remote().create(&b, "x").await.unwrap();
    }

    #[tokio::test]
    async fn an_event_already_gone_is_deleted_all_the_same() {
        let server = MockServer::start().await;
        Mock::given(verb("DELETE"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        remote()
            .delete(&format!("{}/c/a.ics", server.uri()), None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_refused_password_says_so() {
        let server = MockServer::start().await;
        Mock::given(verb("PROPFIND"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let home = format!("{}/h/", server.uri());
        assert!(matches!(
            remote().calendars(&home).await,
            Err(Error::ReauthRequired)
        ));
        let start = Url::parse(&home).unwrap();
        let said = discover(&client, &[start], "u", "p").await.unwrap_err();
        assert_eq!(said.to_string(), WRONG_PASSWORD);
    }

    /// The route a generic server takes: `.well-known` redirects, the principal names the
    /// home, and the method and password survive the redirect.
    #[tokio::test]
    async fn the_calendars_are_found_through_a_well_known_redirect() {
        let server = MockServer::start().await;
        Mock::given(verb("PROPFIND"))
            .and(path("/.well-known/caldav"))
            .respond_with(ResponseTemplate::new(301).insert_header("Location", "/dav/"))
            .mount(&server)
            .await;
        Mock::given(verb("PROPFIND"))
            .and(path("/dav/"))
            .and(header("Authorization", basic("u", "p").as_str()))
            .respond_with(ResponseTemplate::new(207).set_body_string(
                r#"<multistatus xmlns="DAV:"><response><href>/dav/</href><propstat><prop><current-user-principal><href>/dav/principals/u/</href></current-user-principal></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#,
            ))
            .mount(&server)
            .await;
        Mock::given(verb("PROPFIND"))
            .and(path("/dav/principals/u/"))
            .respond_with(ResponseTemplate::new(207).set_body_string(
                r#"<multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><response><href>/dav/principals/u/</href><propstat><prop><C:calendar-home-set><href>/dav/calendars/u</href></C:calendar-home-set></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#,
            ))
            .mount(&server)
            .await;
        let start = Url::parse(&format!("{}/.well-known/caldav", server.uri())).unwrap();
        let found = discover(&client, &[start], "u", "p").await.unwrap();
        assert_eq!(found.home, format!("{}/dav/calendars/u/", server.uri()));
    }

    #[test]
    fn a_pass_fetches_what_changed_and_lets_go_of_what_is_gone() {
        let tag = |t: &str| Some(t.to_string());
        let theirs = HashMap::from([
            ("/same.ics".to_string(), tag("1")),
            ("/changed.ics".to_string(), tag("2")),
            ("/new.ics".to_string(), tag("3")),
        ]);
        let ours = HashMap::from([
            ("/same.ics".to_string(), tag("1")),
            ("/changed.ics".to_string(), tag("old")),
            ("/gone.ics".to_string(), tag("4")),
        ]);
        let (wanted, gone) = differences(&theirs, &ours);
        assert_eq!(wanted, ["/changed.ics", "/new.ics"]);
        assert_eq!(gone, ["/gone.ics"]);
    }

    #[test]
    fn an_event_guid_is_its_files_name_or_a_hash_never_with_two_underscores() {
        assert_eq!(
            event_guid("cal", "/c/3F2B-AA.ics"),
            "cal:3F2B-AA",
            "a file named here is known before the server has it"
        );
        for awkward in ["/c/a__b.ics", "/c/sabredav_1.ics", "/c/x%40y.ics"] {
            let guid = event_guid("cal", awkward);
            assert!(!guid.contains("__"), "{guid}");
            assert_eq!(guid.len(), "cal:".len() + 32, "{guid}");
        }
    }

    #[test]
    fn a_password_goes_only_where_it_cant_be_read_on_the_way() {
        let ok = |s: &str| safe_for_a_password(&Url::parse(s).unwrap());
        assert!(ok("https://cloud.example.org/"));
        assert!(ok("http://192.168.1.20/"), "at home, by address");
        assert!(
            !ok("http://100.100.1.1/"),
            "shared address space: a tailnet, or a provider's NAT"
        );
        assert!(ok("https://100.100.1.1/"), "a tailnet, over HTTPS");
        assert!(!ok("http://cloud.example.org/"), "a name can lead anywhere");
        assert!(!ok("http://8.8.8.8/"), "the open internet");
    }

    #[test]
    fn a_colour_is_kept_only_as_plain_hex() {
        assert_eq!(colour_of("#0082C9FF").as_deref(), Some("#0082c9"));
        assert_eq!(colour_of("#abcdef").as_deref(), Some("#abcdef"));
        assert_eq!(colour_of("red"), None);
        assert_eq!(colour_of("#12345€"), None);
    }

    #[test]
    fn nonsense_is_an_error_not_a_panic() {
        assert!(
            multistatus("<a><b></a>").is_err() || multistatus("<a><b></a>").unwrap().is_empty()
        );
        assert!(multistatus("").unwrap().is_empty());
    }
}
