//! Apple's contacts web service.
//!
//! Unlike reminders, this account's contacts still come from the legacy endpoint rather than
//! CloudKit, so `/co/startup` is the whole of it.

use crate::{Error, Result};
use quince_core::contacts::{
    Card, Contact, ContactDetails, ContactMethod, PostalAddress, ADDRESS_FIELDS, DETAIL_KEYS,
};
use serde_json::Value;

/// Editing a card's postal addresses part by part.
#[path = "contacts_address.rs"]
pub mod address;

/// Merging cards that are one person, on Apple's own cards.
#[path = "contacts_merge.rs"]
pub mod merge;

fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Read one of Apple's method arrays — phones, emails and the rest share a shape.
///
/// Apple names the value `field`, with an optional `label`. Entries with no value are
/// skipped rather than stored as blanks.
fn methods_from(v: &Value, key: &str, kind: &str) -> Vec<ContactMethod> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    Some(ContactMethod {
                        kind: kind.to_string(),
                        label: string(entry, "label"),
                        value: string(entry, "field")?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Apple's postal address: a structured object, not a string.
///
/// Flattened into the lines a person would write on an envelope, in the order the address's
/// country writes them. Reading it as a string — which is what every other method array
/// holds — simply skipped it, so the one contact with an address showed none.
fn address_from(entry: &Value) -> Option<String> {
    postal_from(entry).map(|a| a.formatted())
}

/// One of Apple's `streetAddresses` entries, part by part. Measured on a real account:
/// `{ "label": "HOME", "field": { "street", "city", "state", "postalCode", "country" } }`,
/// the street holding its own line breaks; `countryCode`, `subLocality` and
/// `subAdministrativeArea` are read when a card has them. A field that is a plain string
/// reads as the street. `None` when nothing is in it.
fn postal_from(entry: &Value) -> Option<PostalAddress> {
    let field = entry.get("field")?;
    let mut address = PostalAddress {
        label: string(entry, "label"),
        ..PostalAddress::default()
    };
    if let Some(text) = field.as_str() {
        address.street = Some(text.trim().to_string()).filter(|s| !s.is_empty());
    } else {
        for key in ADDRESS_FIELDS {
            let value = field
                .get(address_wire(key))
                .or_else(|| field.get(key))
                .and_then(Value::as_str);
            address.set_field(key, value);
        }
    }
    (!address.is_empty()).then_some(address)
}

/// The key Apple's card holds an address part under. Apple spells one part with a small `a`
/// (`subadministrativeArea`, contacts2 bundle schema, 2026-10); the library names it as the
/// Contacts framework does. Read under either, written as Apple spells it.
#[must_use]
pub(crate) fn address_wire(key: &str) -> &str {
    if key == "subAdministrativeArea" {
        "subadministrativeArea"
    } else {
        key
    }
}

/// Every card's addresses, part by part, as the library keeps them beside the card.
#[must_use]
pub fn parse_addresses(body: &Value) -> Vec<(String, Vec<PostalAddress>)> {
    body.get("contacts")
        .and_then(Value::as_array)
        .map(|cards| {
            cards
                .iter()
                .filter_map(|card| {
                    let guid = string(card, "contactId")?;
                    let addresses = card
                        .get("streetAddresses")
                        .and_then(Value::as_array)
                        .map(|list| list.iter().filter_map(postal_from).collect())
                        .unwrap_or_default();
                    Some((guid, addresses))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn contact_from(v: &Value) -> Option<Contact> {
    let guid = string(v, "contactId")?;
    let mut methods = methods_from(v, "phones", "phone");
    methods.extend(methods_from(v, "emailAddresses", "email"));
    methods.extend(methods_from(v, "urls", "url"));

    // A postal address nests its parts rather than holding a string.
    methods.extend(
        v.get("streetAddresses")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(ContactMethod {
                            kind: "address".into(),
                            label: string(entry, "label"),
                            value: address_from(entry)?,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );

    methods.extend(methods_from(v, "relatedNames", "related"));
    methods.extend(methods_from(v, "dates", "date"));
    // Instant messaging is labelled by its service, as icloud.com's editor labels it, and
    // the name is inside: `{ "field": { "userName", "IMService" } }`.
    methods.extend(
        v.get("IMs")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(ContactMethod {
                            kind: "im".into(),
                            label: entry_label("im", entry),
                            value: entry_value("im", entry)?,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );

    // A profile carries both an encoded handle and the readable one a person typed. The
    // readable one is the point; `x-apple:%2B91%2090000` is not an answer to "who is this".
    methods.extend(
        v.get("profiles")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(ContactMethod {
                            kind: "profile".into(),
                            label: string(entry, "label"),
                            value: entry_value("profile", entry)?,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );

    Some(Contact {
        guid,
        first_name: string(v, "firstName"),
        middle_name: string(v, "middleName"),
        // Apple hands out a URL for the picture, a signature that changes with it, and the
        // square its owner chose. Only the URL is always there.
        photo_url: v
            .pointer("/photo/url")
            .and_then(Value::as_str)
            .map(photo_url),
        photo_signature: v
            .pointer("/photo/signature")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        photo_crop: v.pointer("/photo/crop").and_then(crop),
        last_name: string(v, "lastName"),
        company: string(v, "companyName"),
        is_company: v.get("isCompany").and_then(Value::as_bool).unwrap_or(false),
        birthday: string(v, "birthday"),
        notes: string(v, "notes"),
        etag: string(v, "etag"),
        methods,
    })
}

/// The rest of a card's name and work, by Apple's keys.
fn details_from(v: &Value) -> ContactDetails {
    let mut d = ContactDetails::default();
    for key in DETAIL_KEYS {
        d.set(key, v.get(key).and_then(Value::as_str));
    }
    d
}

/// Every card's prefix, suffix, nickname, phonetic names, job title and department.
#[must_use]
pub fn parse_details(body: &Value) -> Vec<(String, ContactDetails)> {
    body.get("contacts")
        .and_then(Value::as_array)
        .map(|cards| {
            cards
                .iter()
                .filter_map(|card| Some((string(card, "contactId")?, details_from(card))))
                .collect()
        })
        .unwrap_or_default()
}

/// Each card whole, as one read found it.
#[must_use]
pub fn parse_cards(body: &Value) -> Vec<Card> {
    body.get("contacts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    Some(Card {
                        contact: contact_from(v)?,
                        details: details_from(v),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Where each kind of detail lives on Apple's card.
const METHOD_KEYS: [(&str, &str); 7] = [
    ("phones", "phone"),
    ("emailAddresses", "email"),
    ("urls", "url"),
    ("relatedNames", "related"),
    ("dates", "date"),
    ("profiles", "profile"),
    ("IMs", "im"),
];

/// What an entry of one kind holds, as icloud.com's editor reads it: most hold a string in
/// `field`; a profile the handle a person typed (`user`, else its display name, else its
/// link); instant messaging the name inside `field`.
fn entry_value(kind: &str, entry: &Value) -> Option<String> {
    match kind {
        "profile" => string(entry, "user")
            .or_else(|| string(entry, "displayname"))
            .or_else(|| string(entry, "field")),
        "im" => entry
            .pointer("/field/userName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string),
        _ => string(entry, "field"),
    }
}

/// An entry's label: for instant messaging, its service.
fn entry_label(kind: &str, entry: &Value) -> Option<String> {
    match kind {
        "im" => entry
            .pointer("/field/IMService")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => entry
            .get("label")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    }
}

/// Apple's entry for a detail — or, given the entry it replaces, that entry changed as
/// icloud.com's editor changes one: the value and label set, everything else on it kept.
fn build_entry(kind: &str, value: &str, label: Option<&str>, from: Option<&Value>) -> Value {
    let mut e = from.and_then(Value::as_object).cloned().unwrap_or_default();
    match kind {
        "im" => {
            let mut field = e
                .get("field")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            field.insert("userName".into(), Value::String(value.into()));
            match label {
                Some(l) => field.insert("IMService".into(), Value::String(l.into())),
                None => field.remove("IMService"),
            };
            e.insert("field".into(), Value::Object(field));
        }
        "profile" => {
            e.insert("user".into(), Value::String(value.into()));
            set_label(&mut e, label);
        }
        _ => {
            e.insert("field".into(), Value::String(value.into()));
            set_label(&mut e, label);
        }
    }
    Value::Object(e)
}

fn set_label(e: &mut serde_json::Map<String, Value>, label: Option<&str>) {
    match label {
        Some(l) => e.insert("label".into(), Value::String(l.into())),
        None => e.remove("label"),
    };
}

/// A crop as Apple's card holds it, from the `x,y,width,height` the library keeps.
fn crop_value(crop: Option<&str>) -> Option<Value> {
    let n: Vec<i64> = crop?
        .split(',')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let [x, y, width, height] = n[..] else {
        return None;
    };
    (width > 0 && height > 0)
        .then(|| serde_json::json!({ "x": x, "y": y, "width": width, "height": height }))
}

/// A contact's picture as Apple serves it, whole: fetched from its `url` with the session's
/// parameters, or decoded where the url is the picture itself (a `data:` URL, as one chosen
/// on this device and not yet at Apple is). Shared by the desktop's faces and the phone's sync.
///
/// # Errors
/// When Apple can't be reached or refuses, or a `data:` URL can't be read.
pub async fn photo_bytes(
    http: &reqwest::Client,
    params: &[(String, String)],
    url: &str,
) -> Result<Vec<u8>> {
    if let Some(data) = url.strip_prefix("data:") {
        use base64::Engine;
        let (_, encoded) = data
            .split_once(',')
            .ok_or_else(|| Error::Protocol("that picture could not be read".into()))?;
        return base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Error::Protocol(format!("that picture could not be read: {e}")));
    }
    let response = http.get(url).query(params).send().await?;
    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return Err(Error::Busy);
    }
    if !status.is_success() {
        return Err(Error::Protocol(format!(
            "Apple returned {status} for a contact's picture"
        )));
    }
    Ok(response.bytes().await?.to_vec())
}

#[must_use]
/// Apple's own address book service appends a vCard property to some picture URLs, leaving
/// `.../card/<32 hex>X-SHARED-PHOTO-DISPLAY-PREF:ALWAYS_ASK`. That URL answers 404. Four of
/// seventeen pictures on a real account arrive this way, so the leak is trimmed back to the
/// card id — which does answer — rather than those people silently having no picture.
fn photo_url(url: &str) -> String {
    let Some((base, last)) = url.rsplit_once('/') else {
        return url.to_string();
    };
    let id: String = last.chars().take_while(char::is_ascii_hexdigit).collect();
    if id.len() == 32 && id.len() < last.len() {
        format!("{base}/{id}")
    } else {
        url.to_string()
    }
}

/// The chosen square, flattened to the `x,y,width,height` the library keeps.
fn crop(value: &Value) -> Option<String> {
    let at = |key: &str| value.get(key).and_then(Value::as_i64);
    let (x, y, width, height) = (at("x")?, at("y")?, at("width")?, at("height")?);
    (width > 0 && height > 0).then(|| format!("{x},{y},{width},{height}"))
}

pub fn parse(body: &Value) -> Vec<Contact> {
    body.get("contacts")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(contact_from).collect())
        .unwrap_or_default()
}

/// The tokens the next write to Contacts carries, kept through a run of writes.
///
/// Every write changes the address book, and with it the sync token: the token a write was
/// sent with is spent once Apple takes it. A run of writes (an import in batches, a merge, the
/// queue sent in order) therefore never reuses one. A write Apple answers with new tokens
/// leaves those for the next; one answered without them, or not taken, leaves none, and the
/// next write reads the book again first. Tokens a read of the book just returned are fresh,
/// and are what the first write carries.
#[derive(Debug, Default)]
pub struct Ledger {
    held: std::sync::Mutex<Option<Tokens>>,
}

impl Ledger {
    /// No tokens yet: the first write reads them.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Tokens read with the book a moment ago, for the next write.
    #[must_use]
    pub fn holding(tokens: Tokens) -> Self {
        Ledger {
            held: std::sync::Mutex::new(Some(tokens)),
        }
    }

    /// Tokens a read of the book just returned: the next write carries them.
    pub fn hold(&self, tokens: Tokens) {
        *self.held() = Some(tokens);
    }

    /// Whether it holds tokens no write has spent yet. Without them the next write reads the
    /// book first.
    #[must_use]
    pub fn has_tokens(&self) -> bool {
        self.held().is_some()
    }

    /// A write made outside the ledger (with tokens it read itself) has moved the book on:
    /// the tokens held are spent.
    pub fn spend(&self) {
        *self.held() = None;
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Option<Tokens>> {
        self.held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// What the next write carries: tokens not yet spent, or the book's own, read now.
    async fn tokens(
        &self,
        http: &reqwest::Client,
        root: &str,
        params: Vec<(String, String)>,
    ) -> Result<Tokens> {
        if let Some(tokens) = self.held().clone() {
            return Ok(tokens);
        }
        let body = startup(http, root, params).await?;
        self.read(&body)
    }

    /// A read of the whole book: its tokens are the current ones.
    fn read(&self, body: &Value) -> Result<Tokens> {
        let tokens = Tokens::of_startup(body)?;
        *self.held() = Some(tokens.clone());
        Ok(tokens)
    }

    /// After a write: the tokens Apple answered with, if it did; otherwise none — the next
    /// write reads them again rather than send these twice.
    fn answered(&self, body: Option<&Value>) {
        let mut held = self.held();
        let next = body.and_then(|b| {
            let sync = b.get("syncToken").and_then(Value::as_str)?.to_string();
            let pref = b
                .get("prefToken")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .or_else(|| held.as_ref().map(|t| t.pref.clone()))?;
            // The same token back is no new token: the write may still have moved the book.
            (held.as_ref().map(|t| t.sync.as_str()) != Some(sync.as_str()))
                .then_some(Tokens { sync, pref })
        });
        *held = next;
    }
}

/// The tokens Apple hands out at startup, which every write has to carry.
///
/// `syncToken` is how the service knows which version of the address book a change is based
/// on; `prefToken` carries the sort order the account is set to. Writing without them is
/// refused.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tokens {
    pub sync: String,
    pub pref: String,
}

impl Tokens {
    /// The tokens a `/co/startup` answer carries.
    fn of_startup(body: &Value) -> Result<Self> {
        let token = |key: &str| {
            body.get(key)
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
        };
        Ok(Tokens {
            sync: token("syncToken")?,
            pref: token("prefToken")?,
        })
    }

    /// As kept in the library's sync position for Contacts.
    #[must_use]
    pub fn stored(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// From the library's sync position; `None` for anything else (a library from before
    /// positions were kept), which reads the book whole.
    #[must_use]
    pub fn from_stored(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

/// What a Contacts pass found: the book read again, or Apple saying nothing changed since the
/// tokens the library kept.
pub enum Pass {
    Read(AddressBook, Tokens),
    Unchanged(Tokens),
}

/// A pass over Contacts that reads the book only when it changed. Apple's `/co/changeset`
/// answers "unchanged" in a few hundred milliseconds; a change (or tokens too old) is answered
/// 420, and only then is the whole book read — Apple sends no partial changes to read instead.
/// `since` is `None` for a full read (the first, and the daily one).
///
/// # Errors
/// When Apple cannot be reached or refuses.
pub async fn pass(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    since: Option<Tokens>,
) -> Result<Pass> {
    if let Some(mut tokens) = since {
        match changed(http, root, params.clone(), &mut tokens).await {
            Ok(false) => return Ok(Pass::Unchanged(tokens)),
            Ok(true) => {}
            // A session gone is the caller's to mend; anything else, the book is read.
            Err(Error::ReauthRequired) => return Err(Error::ReauthRequired),
            Err(_) => {}
        }
    }
    let (book, tokens) = fetch_book_with_tokens(http, root, params).await?;
    Ok(Pass::Read(book, tokens))
}

/// Read the address book along with the tokens a write will need.
///
/// The tokens come from the same response as the contacts, so asking for both at once is
/// one request rather than two, and the tokens are guaranteed to match the data.
/// The whole address book — people and lists — with the tokens a write needs, in one read.
pub async fn fetch_book_with_tokens(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<(AddressBook, Tokens)> {
    let body = startup(http, root, params).await?;
    let token = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
    };
    Ok((
        AddressBook {
            contacts: parse(&body),
            lists: parse_lists(&body),
            me: me_card(&body),
            addresses: parse_addresses(&body),
            details: parse_details(&body),
        },
        Tokens {
            sync: token("syncToken")?,
            pref: token("prefToken")?,
        },
    ))
}

/// Whether the address book changed since `tokens` were read, from `/co/changeset` —
/// icloud.com's own check, which answers in a few hundred milliseconds.
///
/// Measured: with nothing changed it answers 200 with the same sync token and no contacts.
/// After a change made anywhere it answers 420: the token is behind, read the book again.
/// A 200 that carries contacts or a new token is a change too, and its tokens are kept.
pub async fn changed(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &mut Tokens,
) -> Result<bool> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));
    query.push(("prefToken".into(), tokens.pref.clone()));
    query.push(("syncToken".into(), tokens.sync.clone()));
    let url = format!("{}/co/changeset", root.trim_end_matches('/'));
    let response = http.post(url).query(&query).send().await?;
    match response.status().as_u16() {
        420 => return Ok(true),
        450 => return Err(Error::ReauthRequired),
        code if !(200..300).contains(&code) => {
            return Err(Error::Protocol(format!("/co/changeset answered {code}")));
        }
        _ => {}
    }
    let body: Value = response.json().await.unwrap_or(Value::Null);
    Ok(changeset_moved(&body, tokens))
}

/// Whether a 200 from `/co/changeset` says something changed; its tokens are taken either way.
fn changeset_moved(body: &Value, tokens: &mut Tokens) -> bool {
    let carries = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    };
    let gone = body
        .get("deletes")
        .and_then(Value::as_object)
        .is_some_and(|d| {
            d.values()
                .any(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        });
    let mut moved = carries("contacts") || carries("groups") || gone;
    if let Some(sync) = body.get("syncToken").and_then(Value::as_str) {
        moved |= sync != tokens.sync;
        tokens.sync = sync.to_string();
    }
    if let Some(pref) = body.get("prefToken").and_then(Value::as_str) {
        tokens.pref = pref.to_string();
    }
    moved
}

pub async fn fetch_with_tokens(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<(Vec<Contact>, Tokens)> {
    let body = startup(http, root, params).await?;
    let token = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
    };
    Ok((
        parse(&body),
        Tokens {
            sync: token("syncToken")?,
            pref: token("prefToken")?,
        },
    ))
}

/// Every card as Apple wrote it, with the tokens a write needs, in one read of the book: what
/// a run of queued changes is sent from, rather than the whole book read again for each one.
pub async fn cards_with_tokens(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<(Vec<Value>, Tokens)> {
    let (cards, _, tokens) = cards_and_lists_with_tokens(http, root, params).await?;
    Ok((cards, tokens))
}

/// [`cards_with_tokens`] with the lists too, from the same read: a run that changes lists as
/// well as cards sends both from one read of the book.
pub async fn cards_and_lists_with_tokens(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<(Vec<Value>, Vec<ContactList>, Tokens)> {
    let body = startup(http, root, params).await?;
    let token = |key: &str| {
        body.get(key)
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| Error::Protocol(format!("/co/startup carried no {key}")))
    };
    let tokens = Tokens {
        sync: token("syncToken")?,
        pref: token("prefToken")?,
    };
    let cards = body
        .get("contacts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok((cards, parse_lists(&body), tokens))
}

/// [`card_written`] for a card without the rest of its name and work: those stay as Apple
/// has them.
fn card_merged(contact: &Contact, existing: Option<&Value>, base: Option<&Contact>) -> Value {
    card_written(contact, None, existing, base, None)
}

/// The card to send: Apple's own as it stands now, with the fields this person changed —
/// and only those — set from the form. `base` is the card as the form opened it; a field the
/// form left as it found it keeps Apple's current value, so a name or number changed on the
/// phone while the form was open is not put back. Without a base every modelled field is
/// written, as a new card needs. `details` (with the form's `base_details`) are the rest of
/// the name and work; `None` leaves Apple's.
fn card_written(
    contact: &Contact,
    details: Option<&ContactDetails>,
    existing: Option<&Value>,
    base: Option<&Contact>,
    base_details: Option<&ContactDetails>,
) -> Value {
    // Apple's card is the starting point, and Apple is not obliged to send an object; a
    // write that panicked on network input would take the whole window down with it.
    let mut card = existing
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let Some(object) = card.as_object_mut() else {
        return serde_json::json!({});
    };

    object.insert("contactId".into(), Value::String(contact.guid.clone()));
    object.insert("isCompany".into(), Value::Bool(contact.is_company));
    let unchanged =
        |pick: fn(&Contact) -> &Option<String>| base.is_some_and(|b| pick(b) == pick(contact));
    for (key, value, pick) in [
        (
            "firstName",
            &contact.first_name,
            (|c: &Contact| &c.first_name) as fn(&Contact) -> &Option<String>,
        ),
        ("middleName", &contact.middle_name, |c: &Contact| {
            &c.middle_name
        }),
        ("lastName", &contact.last_name, |c: &Contact| &c.last_name),
        ("companyName", &contact.company, |c: &Contact| &c.company),
        ("birthday", &contact.birthday, |c: &Contact| &c.birthday),
        ("notes", &contact.notes, |c: &Contact| &c.notes),
    ] {
        if existing.is_some() && unchanged(pick) {
            continue;
        }
        // A middle name is written only when one is given or one is being taken away: the
        // forms before this one never held it, and a card made without one needs no key.
        if key == "middleName" && value.is_none() && base.is_none_or(|b| b.middle_name.is_none()) {
            continue;
        }
        match value {
            Some(text) if !text.trim().is_empty() => {
                object.insert(key.into(), Value::String(text.trim().to_string()));
            }
            // Clearing a field means sending it empty; removing the key would leave what
            // Apple already has.
            _ => {
                object.insert(key.into(), Value::String(String::new()));
            }
        }
    }

    // The rest of the name and work: written when given, and then only what changed — on a
    // new card, only what holds something.
    if let Some(details) = details {
        for key in DETAIL_KEYS {
            let mine = details.get(key).cloned().flatten();
            let write = if existing.is_none() {
                mine.is_some()
            } else {
                base_details.is_none_or(|b| b.get(key).cloned().flatten() != mine)
            };
            if write {
                object.insert(key.into(), Value::String(mine.unwrap_or_default()));
            }
        }
    }

    for (key, kind) in METHOD_KEYS {
        let of = |c: &Contact| -> Vec<(String, Option<String>)> {
            c.methods
                .iter()
                .filter(|m| m.kind == kind && !m.value.trim().is_empty())
                .map(|m| (m.value.trim().to_string(), m.label.clone()))
                .collect()
        };
        let mine = of(contact);
        if existing.is_some() && base.is_some_and(|b| of(b) == mine) {
            continue;
        }
        // The kinds this app read but never wrote are written only by a form that knows
        // them: one from before, with no base, leaves them as Apple has them.
        if base.is_none() && existing.is_some() && !matches!(kind, "phone" | "email" | "url") {
            continue;
        }
        if existing.is_none() && mine.is_empty() && !matches!(kind, "phone" | "email" | "url") {
            continue;
        }
        let theirs = existing
            .and_then(|card| card.get(key))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // Edited from a base over Apple's current card: the edit laid over Apple's list, not
        // the form's list written whole — a number added on the phone while the form was open
        // here stays, as a name changed there does.
        let value = match (base, existing) {
            (Some(base), Some(_)) => merge_methods(kind, &theirs, &of(base), &mine),
            _ => Value::Array(
                mine.iter()
                    .map(|(v, l)| build_entry(kind, v, l.as_deref(), None))
                    .collect(),
            ),
        };
        object.insert(key.into(), value);
    }

    // The picture: a new one goes as icloud.com sends it — the image itself as a `data:` URL,
    // with the square chosen — a new square for the same picture keeps the rest of Apple's
    // entry, and a picture taken away is `null`.
    let photo_of = |c: &Contact| (c.photo_url.clone(), c.photo_crop.clone());
    let photo_changed = match base {
        Some(b) => photo_of(b) != photo_of(contact),
        None => existing.is_none() && contact.photo_url.is_some(),
    };
    if photo_changed {
        let value = match &contact.photo_url {
            None => Value::Null,
            Some(url) => {
                let same_picture = base.is_some_and(|b| b.photo_url.as_deref() == Some(url));
                let mut photo = existing
                    .filter(|_| same_picture)
                    .and_then(|e| e.get("photo"))
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                if !same_picture {
                    photo.insert("url".into(), Value::String(url.clone()));
                }
                match crop_value(contact.photo_crop.as_deref()) {
                    Some(crop) => photo.insert("crop".into(), crop),
                    None => photo.remove("crop"),
                };
                Value::Object(photo)
            }
        };
        object.insert("photo".into(), value);
    }

    // Laid over Apple's current card, the write is against that card's version; otherwise
    // against the one the form read.
    let fresh = base
        .is_some()
        .then(|| existing.and_then(|e| e.get("etag")).and_then(Value::as_str))
        .flatten();
    if let Some(etag) = fresh
        .map(ToString::to_string)
        .or_else(|| contact.etag.clone())
    {
        object.insert("etag".into(), Value::String(etag));
    }
    card
}

/// One kind of a card's details, as Apple holds it now (`theirs`), with the change the form
/// made (`base` to `mine`) laid over it: what the form took out goes, what it put in is added —
/// an edited entry in the old one's place, keeping what else Apple kept on it — and everything
/// else Apple has stays as Apple has it, entries made on another device included.
fn merge_methods(
    kind: &str,
    theirs: &[Value],
    base: &[(String, Option<String>)],
    mine: &[(String, Option<String>)],
) -> Value {
    let matches = |entry: &Value, (value, label): &(String, Option<String>)| {
        entry_value(kind, entry).as_deref() == Some(value.as_str())
            && entry_label(kind, entry).as_deref() == label.as_deref()
    };
    // Counted, not just looked for: a number on the card twice with one copy taken out is one
    // removal, which a plain "is it still there" never saw.
    let mut left: Vec<&(String, Option<String>)> = mine.iter().collect();
    let mut removed: Vec<&(String, Option<String>)> = Vec::new();
    for b in base {
        match left.iter().position(|m| *m == b) {
            Some(i) => {
                left.remove(i);
            }
            None => removed.push(b),
        }
    }
    let mut added = left
        .into_iter()
        .filter(|m| !theirs.iter().any(|t| matches(t, m)))
        .collect::<Vec<_>>()
        .into_iter();
    let mut out = Vec::with_capacity(theirs.len() + mine.len());
    for t in theirs {
        if let Some(i) = removed.iter().position(|r| matches(t, r)) {
            removed.remove(i);
            // Edited rather than taken out: the new entry where the old one was.
            if let Some((v, l)) = added.next() {
                out.push(build_entry(kind, v, l.as_deref(), Some(t)));
            }
        } else {
            out.push(t.clone());
        }
    }
    out.extend(added.map(|(v, l)| build_entry(kind, v, l.as_deref(), None)));
    Value::Array(out)
}

async fn write(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    method: reqwest::Method,
    card: Value,
) -> Result<Value> {
    let tokens = ledger.tokens(http, root, params.clone()).await?;
    let answer = write_with(http, root, params, &tokens, method, card).await;
    ledger.answered(answer.as_ref().ok());
    answer
}

async fn write_with(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    method: reqwest::Method,
    card: Value,
) -> Result<Value> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    // Fixed rather than the machine's own, and deliberately. Asked with en_US, fr_FR and
    // ja_JP, Apple sent back byte-identical contacts, ordering and section headers — the
    // parameter changes nothing this app reads, and sorting happens locally anyway.
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));
    query.push(("prefToken".into(), tokens.pref.clone()));
    query.push(("syncToken".into(), tokens.sync.clone()));

    let url = format!("{}/co/contacts/card", root.trim_end_matches('/'));
    let response = http
        .request(method, url)
        .query(&query)
        .json(&serde_json::json!({ "contacts": [card] }))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    // Apple answers a stale syncToken with a conflict rather than an error, and the fix is
    // to read the address book again — saying so beats a status code.
    if status.as_u16() == 412 || status.as_u16() == 409 {
        return Err(Error::Conflict);
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        // Apple can't take it now (429, 503 under load): busy, asked again later — never a
        // broken answer, which the outbox gives up on after a few tries.
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(Error::Busy);
        }
        return Err(Error::Protocol(format!(
            "contacts returned {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    // A card Apple refused comes back as 200 with the reason in `errors` — a 409 when the
    // card changed on the phone since it was read. Read as success, the edit was closed as
    // saved and quietly lost.
    refused_group(&body)?;
    Ok(body)
}

/// Create a contact, and return the id Apple will know it by.
pub async fn create(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    contact: &Contact,
) -> Result<String> {
    // Apple keeps a card that is only a number or an address — "No Name" on the phone — so
    // only a card with nothing in it at all is turned away.
    let blank = |v: &Option<String>| v.as_deref().is_none_or(|s| s.trim().is_empty());
    if blank(&contact.first_name)
        && blank(&contact.last_name)
        && blank(&contact.company)
        && contact.methods.iter().all(|m| m.value.trim().is_empty())
    {
        return Err(Error::Refused(
            "Add a name, a phone number or an email address first.".into(),
        ));
    }
    // Apple's own client mints the id rather than asking the service for one.
    let guid = new_contact_id();
    // A fresh id: no card can hold it yet.
    write_new(http, root, params, ledger, &guid, contact).await?;
    Ok(guid)
}

/// An id for a card made here, minted before it is sent — Apple's own client does the same —
/// so the card can be shown at once, queued, and sent again without making a second.
pub fn new_contact_id() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// Whether a card is empty of everything Apple would keep.
pub fn blank_card(contact: &Contact) -> bool {
    let blank = |v: &Option<String>| v.as_deref().is_none_or(|s| s.trim().is_empty());
    blank(&contact.first_name)
        && blank(&contact.last_name)
        && blank(&contact.company)
        && contact.methods.iter().all(|m| m.value.trim().is_empty())
}

/// [`create`] under an id chosen beforehand. Sent again, it makes no second card: a card
/// Apple already holds under that id is left as it is.
pub async fn create_named(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    guid: &str,
    contact: &Contact,
) -> Result<()> {
    if blank_card(contact) {
        return Err(Error::Refused(
            "Add a name, a phone number or an email address first.".into(),
        ));
    }
    if card(http, root, params.clone(), ledger, guid)
        .await?
        .is_some()
    {
        return Ok(());
    }
    write_new(http, root, params, ledger, guid, contact).await
}

async fn write_new(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    guid: &str,
    contact: &Contact,
) -> Result<()> {
    write_new_card(http, root, params, ledger, guid, contact, None).await
}

async fn write_new_card(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    guid: &str,
    contact: &Contact,
    details: Option<&ContactDetails>,
) -> Result<()> {
    let mut fresh = contact.clone();
    fresh.guid = guid.to_string();
    fresh.etag = None;

    write(
        http,
        root,
        params,
        ledger,
        reqwest::Method::POST,
        card_written(&fresh, details, None, None, None),
    )
    .await?;
    Ok(())
}

/// [`create_named`] for a whole card: the rest of its name and work, and a picture, go too.
pub async fn create_card_named(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    guid: &str,
    card: &Card,
) -> Result<()> {
    if blank_card(&card.contact) && card.details.is_empty() {
        return Err(Error::Refused(
            "Add a name, a phone number or an email address first.".into(),
        ));
    }
    if self::card(http, root, params.clone(), ledger, guid)
        .await?
        .is_some()
    {
        return Ok(());
    }
    write_new_card(
        http,
        root,
        params,
        ledger,
        guid,
        &card.contact,
        Some(&card.details),
    )
    .await
}

/// [`update_from`] for a whole card: only what changed from `base`, the rest of the name and
/// work and the picture included, over Apple's current card.
pub async fn update_card(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    card: &Card,
    existing: Option<&Value>,
    base: Option<&Card>,
) -> Result<()> {
    if existing.is_none() {
        return Err(Error::Refused(
            "This contact was deleted on another device, so it wasn't saved.".into(),
        ));
    }
    write(
        http,
        root,
        params,
        ledger,
        reqwest::Method::PUT,
        card_written(
            &card.contact,
            Some(&card.details),
            existing,
            base.map(|b| &b.contact),
            base.map(|b| &b.details),
        ),
    )
    .await?;
    Ok(())
}

/// Save changes to a contact.
///
/// `existing` is Apple's own card for this person, so fields this app does not model are
/// written back untouched instead of being dropped.
pub async fn update(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    contact: &Contact,
    existing: Option<&Value>,
) -> Result<()> {
    update_from(http, root, params, ledger, contact, existing, None).await
}

/// [`update`], writing only what changed from `base` — the card as the form opened it — over
/// Apple's current card. A card no longer at Apple is refused, not rebuilt from the fields
/// this app models: that would drop its photo, addresses and everything else.
pub async fn update_from(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    contact: &Contact,
    existing: Option<&Value>,
    base: Option<&Contact>,
) -> Result<()> {
    if existing.is_none() && base.is_some() {
        return Err(Error::Refused(
            "This contact was deleted on another device, so it wasn't saved.".into(),
        ));
    }
    write(
        http,
        root,
        params,
        ledger,
        reqwest::Method::PUT,
        card_merged(contact, existing, base),
    )
    .await?;
    Ok(())
}

/// Remove a contact.
pub async fn delete(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    contact: &Contact,
) -> Result<()> {
    // The tag Apple holds now, not the one stored at the last sync: a card edited on the
    // phone since then was refused as a conflict, and the delete failed for no reason a
    // person could see. Gone already is done.
    let Some(current) = card(http, root, params.clone(), ledger, &contact.guid).await? else {
        return Ok(());
    };
    let etag = current
        .get("etag")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| contact.etag.clone());
    let mut query = params;
    query.push(("method".into(), "DELETE".into()));
    write(
        http,
        root,
        query,
        ledger,
        reqwest::Method::POST,
        serde_json::json!({
            "contactId": contact.guid,
            "etag": etag,
        }),
    )
    .await?;
    Ok(())
}

/// Apple's card for one person, as it currently stands. The book is read whole for it, so
/// its tokens go to `ledger`: the write that follows carries the tokens of the very version
/// it was made against.
pub async fn card(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    guid: &str,
) -> Result<Option<Value>> {
    let body = startup(http, root, params).await?;
    if ledger.read(&body).is_err() {
        *ledger.held() = None;
    }
    Ok(body
        .get("contacts")
        .and_then(Value::as_array)
        .and_then(|cards| {
            cards
                .iter()
                .find(|c| c.get("contactId").and_then(Value::as_str) == Some(guid))
                .cloned()
        }))
}

/// A Contacts list — Apple's "group" — by name, with the contacts in it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactList {
    pub guid: String,
    pub name: String,
    pub members: Vec<String>,
    pub etag: Option<String>,
}

/// The address book as `/co/startup` gives it: the people, and the lists they are in.
pub struct AddressBook {
    pub contacts: Vec<quince_core::contacts::Contact>,
    pub lists: Vec<ContactList>,
    /// The card that is this person's own, if they have said.
    pub me: Option<String>,
    /// Each card's postal addresses, part by part.
    pub addresses: Vec<(String, Vec<PostalAddress>)>,
    /// The rest of each card's name and work.
    pub details: Vec<(String, ContactDetails)>,
}

/// Where the my-card id is kept between passes.
pub const ME_KEY: &str = "contacts.me";

/// Where the lists are kept between passes. They are few and small, so beside the library's
/// other settings rather than in a table of their own.
pub const LISTS_KEY: &str = "contacts.lists";

/// The lists in a `/co/startup` answer, in the account's own order.
pub fn parse_lists(body: &Value) -> Vec<ContactList> {
    let order: Vec<String> = body
        .pointer("/collections/0/groupsOrder")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut lists: Vec<ContactList> = body
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| {
            Some(ContactList {
                guid: string(g, "groupId")?,
                name: string(g, "name").unwrap_or_else(|| "Untitled List".into()),
                members: g
                    .get("contactIds")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                etag: string(g, "etag"),
            })
        })
        .collect();
    lists.sort_by_key(|l| {
        order
            .iter()
            .position(|o| *o == l.guid)
            .unwrap_or(usize::MAX)
    });
    lists
}

/// The lists the library holds from the last pass.
pub fn stored_lists(library: &quince_core::Library) -> Vec<ContactList> {
    library
        .state(LISTS_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write a list as icloud.com does — the whole of it, every time: a new one is `POST`ed with
/// an id made here, a rename or a change of members is the full list `PUT` back with its
/// etag, a delete names it. Membership travels as the entire set of contact ids.
async fn write_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    method: Option<&str>,
    group: Value,
) -> Result<Value> {
    let tokens = ledger.tokens(http, root, params.clone()).await?;
    let answer = write_list_with(http, root, params, &tokens, method, group).await;
    ledger.answered(answer.as_ref().ok());
    answer
}

async fn write_list_with(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    method: Option<&str>,
    group: Value,
) -> Result<Value> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));
    query.push(("prefToken".into(), tokens.pref.clone()));
    query.push(("syncToken".into(), tokens.sync.clone()));
    if let Some(m) = method {
        query.push(("method".into(), m.into()));
    }
    let url = format!("{}/co/groups/card/", root.trim_end_matches('/'));
    let response = http
        .post(url)
        .query(&query)
        .json(&serde_json::json!({ "groups": [group] }))
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if status.as_u16() == 412 || status.as_u16() == 409 {
        return Err(Error::Conflict);
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        // Apple can't take it now (429, 503 under load): busy, asked again later — never a
        // broken answer, which the outbox gives up on after a few tries.
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(Error::Busy);
        }
        return Err(Error::Protocol(format!(
            "contacts returned {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    refused_group(&body)?;
    Ok(body)
}

/// Apple answers a refused write — a card or a list — with 200 and the reason in `errors`:
/// a 409 when the etag sent is no longer the one Apple holds. Read as success, a lost write
/// would look saved.
fn refused_group(body: &Value) -> Result<()> {
    let Some(error) = body
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|e| e.first())
    else {
        return Ok(());
    };
    match error.get("errorCode").and_then(Value::as_i64) {
        Some(409) | Some(412) => Err(Error::Conflict),
        code => Err(Error::Protocol(format!(
            "Contacts refused the list ({}): {}",
            code.map_or_else(|| "no code".into(), |c| c.to_string()),
            error
                .get("errorMessage")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
        ))),
    }
}

/// Make a list, with these people in it. Answers with its id.
pub async fn create_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    name: &str,
    members: &[String],
) -> Result<String> {
    let guid = uuid::Uuid::new_v4().to_string().to_uppercase();
    create_list_named(http, root, params, ledger, &guid, name, members).await?;
    Ok(guid)
}

/// [`create_list`] under an id chosen beforehand, so a create sent again after a lost answer
/// is the same list: the caller checks the book for it first.
pub async fn create_list_named(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    guid: &str,
    name: &str,
    members: &[String],
) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::Refused("A list needs a name.".into()));
    }
    write_list(
        http,
        root,
        params,
        ledger,
        None,
        serde_json::json!({ "groupId": guid, "name": name, "contactIds": members }),
    )
    .await?;
    Ok(())
}

/// Rename a list, or change who is in it: the whole list, sent back with its etag.
pub async fn update_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    list: &ContactList,
) -> Result<()> {
    write_list(
        http,
        root,
        params,
        ledger,
        Some("PUT"),
        serde_json::json!({ "groupId": list.guid, "etag": list.etag, "name": list.name, "contactIds": list.members }),
    )
    .await?;
    Ok(())
}

/// Delete a list. The people in it stay in Contacts.
pub async fn delete_list(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    guid: &str,
) -> Result<()> {
    write_list(
        http,
        root,
        params,
        ledger,
        Some("DELETE"),
        serde_json::json!({ "groupId": guid }),
    )
    .await?;
    Ok(())
}

/// A plain-text call to Contacts with the address book's tokens, as icloud.com makes its
/// export, import and my-card calls. Answers with Apple's reply as text.
async fn co_text(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    method: reqwest::Method,
    path: &str,
    body: String,
) -> Result<String> {
    let tokens = ledger.tokens(http, root, params.clone()).await?;
    let answer = co_text_with(http, root, params, &tokens, method, path, body).await;
    let json = answer
        .as_ref()
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(t).ok());
    ledger.answered(json.as_ref().filter(|_| answer.is_ok()));
    answer
}

async fn co_text_with(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    tokens: &Tokens,
    method: reqwest::Method,
    path: &str,
    body: String,
) -> Result<String> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));
    query.push(("prefToken".into(), tokens.pref.clone()));
    query.push(("syncToken".into(), tokens.sync.clone()));
    let url = format!("{}/{path}", root.trim_end_matches('/'));
    let response = http
        .request(method, url)
        .query(&query)
        .header("content-type", "text/plain")
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if status.as_u16() == 412 || status.as_u16() == 409 {
        return Err(Error::Conflict);
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        // Apple can't take it now (429, 503 under load): busy, asked again later — never a
        // broken answer, which the outbox gives up on after a few tries.
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(Error::Busy);
        }
        return Err(Error::Protocol(format!(
            "contacts returned {status}: {}",
            text.chars().take(200).collect::<String>()
        )));
    }
    Ok(text)
}

/// These people as one vCard file, written by Apple — every field it holds, photos too.
pub async fn export_vcards(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    ids: &[String],
    file_name: &str,
) -> Result<String> {
    let body = serde_json::json!({ "contactIdsToExport": ids, "exportFileName": file_name });
    let text = co_text(
        http,
        root,
        params,
        ledger,
        reqwest::Method::POST,
        "co/export/",
        body.to_string(),
    )
    .await?;
    if !text.trim_start().starts_with("BEGIN:VCARD") {
        return Err(Error::Protocol(
            "Contacts did not send back a vCard.".into(),
        ));
    }
    Ok(text)
}

/// Each `BEGIN:VCARD … END:VCARD` in a file, whole, in order.
#[must_use]
pub fn split_vcards(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut card = String::new();
    let mut inside = false;
    for line in text.lines() {
        let upper = line.trim().to_ascii_uppercase();
        if upper == "BEGIN:VCARD" {
            inside = true;
            card.clear();
        }
        if inside {
            card.push_str(line.trim_end_matches('\r'));
            card.push_str("\r\n");
        }
        if inside && upper == "END:VCARD" {
            inside = false;
            out.push(std::mem::take(&mut card));
        }
    }
    out
}

/// A quoted-printable value's text: `=XX` as the byte it stands for, read as UTF-8.
fn quoted_printable(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'='
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(bytes[i + 1]) << 4 | hex(bytes[i + 2]));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether a vCard is someone already in the book — the same names and nothing the book's
/// card lacks (each phone and email on it already there). Importing an export of the same
/// book once added a copy of every person (2026-10-20, 123 of them); Apple's own Contacts
/// leaves such a card out, and so does this.
#[must_use]
pub fn already_in(card: &str, book: &[Contact]) -> bool {
    let unfold = card
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "");
    let mut name = (String::new(), String::new(), String::new());
    let mut org = String::new();
    let mut reach: Vec<String> = Vec::new();
    // Read as the import sheet reads it: a vCard 2.1 value in quoted-printable (an Android
    // export of a name in any script but English) decoded, its soft line breaks joined. Left
    // encoded, nobody in such a file was ever "already in", and importing it again copied all.
    let mut lines: Vec<String> = Vec::new();
    let mut raw = unfold.lines();
    while let Some(line) = raw.next() {
        let qp = line
            .split_once(':')
            .is_some_and(|(k, _)| k.to_ascii_uppercase().contains("QUOTED-PRINTABLE"));
        if !qp {
            lines.push(line.to_string());
            continue;
        }
        let mut whole = line.to_string();
        while whole.ends_with('=') {
            whole.pop();
            match raw.next() {
                Some(next) => whole.push_str(next),
                None => break,
            }
        }
        let (key, value) = whole.split_once(':').unwrap_or((&whole, ""));
        lines.push(format!("{key}:{}", quoted_printable(value)));
    }
    for line in &lines {
        let line = line.as_str();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let field = key.split(';').next().unwrap_or("").to_ascii_uppercase();
        let field = field.rsplit('.').next().unwrap_or("").to_string();
        let value = value.trim().replace("\\,", ",").replace("\\;", ";");
        match field.as_str() {
            "N" => {
                let mut parts = line[line.find(':').unwrap_or(0) + 1..].split(';');
                let part = |p: Option<&str>| p.unwrap_or("").trim().replace("\\,", ",");
                let last = part(parts.next());
                let first = part(parts.next());
                let middle = part(parts.next());
                name = (first, middle, last);
            }
            "ORG" => org = value.split(';').next().unwrap_or("").trim().to_string(),
            "TEL" => reach.push(phone_key(&value)),
            "EMAIL" => reach.push(value.to_lowercase()),
            _ => {}
        }
    }
    reach.retain(|r| !r.is_empty());
    let same = |a: &Option<String>, b: &str| {
        a.as_deref()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case(b.trim())
    };
    book.iter().any(|c| {
        let named = same(&c.first_name, &name.0)
            && same(&c.middle_name, &name.1)
            && same(&c.last_name, &name.2)
            && same(&c.company, &org);
        if !named {
            return false;
        }
        let theirs: Vec<String> = c
            .methods
            .iter()
            .filter(|m| m.kind == "phone" || m.kind == "email")
            .map(|m| {
                if m.kind == "phone" {
                    phone_key(&m.value)
                } else {
                    m.value.to_lowercase()
                }
            })
            .collect();
        // A card with nothing to tell it apart is the same only if the book's has nothing too.
        if reach.is_empty() {
            return theirs.is_empty()
                && !(name.0.is_empty() && name.2.is_empty() && org.is_empty());
        }
        reach.iter().all(|r| theirs.contains(r))
    })
}

/// A phone number as digits only, its last ten: "+91 90000 00002" and "090000 00002" are one.
fn phone_key(v: &str) -> String {
    let digits: String = v.chars().filter(char::is_ascii_digit).collect();
    digits[digits.len().saturating_sub(10)..].to_string()
}

/// Cards gathered into batches Apple takes in one import: under `max_bytes` and `max_cards`.
#[must_use]
pub fn batch_vcards(cards: &[String], max_bytes: usize, max_cards: usize) -> Vec<String> {
    let mut out = Vec::new();
    let (mut batch, mut n) = (String::new(), 0);
    for card in cards {
        if n > 0 && (batch.len() + card.len() > max_bytes || n >= max_cards) {
            out.push(std::mem::take(&mut batch));
            n = 0;
        }
        batch.push_str(card);
        n += 1;
    }
    if n > 0 {
        out.push(batch);
    }
    out
}

/// One CSV record per row (RFC 4180: quoted fields, doubled quotes, line breaks in quotes).
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut field) = (Vec::new(), String::new());
    let (mut quoted, mut chars) = (
        false,
        text.trim_start_matches('\u{feff}').chars().peekable(),
    );
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|f| !f.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

/// A birthday as a spreadsheet writes it — `1990-03-14`, Outlook's `3/14/1990`, or `--03-14`
/// with no year — as (year, month, day). Outlook's empty `0/0/00`, and anything that is not
/// plainly a date, is left out rather than guessed at.
fn csv_birthday(text: &str) -> Option<(Option<u16>, u8, u8)> {
    let t = text.trim();
    let num = |s: &str| s.trim().parse::<u16>().ok();
    let (year, month, day) = if let Some(rest) = t.strip_prefix("--") {
        let (m, d) = rest.split_once('-')?;
        (None, num(m)?, num(d)?)
    } else if let Some((y, rest)) = t.split_once('-') {
        let (m, d) = rest.split_once('-')?;
        (Some(num(y)?), num(m)?, num(d.get(..2).unwrap_or(d))?)
    } else {
        let mut parts = t.split('/');
        let (m, d, y) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() {
            return None;
        }
        let y = y.split_whitespace().next()?;
        if y.len() != 4 {
            return None;
        }
        (Some(num(y)?), num(m)?, num(d)?)
    };
    let (month, day) = (u8::try_from(month).ok()?, u8::try_from(day).ok()?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    match year {
        Some(y) if !(1000..=9999).contains(&y) => None,
        _ => Some((year, month, day)),
    }
}

fn vcard_escape(v: &str) -> String {
    // Line ends as a spreadsheet writes them inside a cell (CRLF, or a lone CR) are one line
    // end: a bare CR left in a card reads as a break in the middle of the property.
    v.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

/// Contacts exported as CSV (Google Contacts, Outlook, most address books) as vCards: names,
/// company, every phone and email column, birthday and notes, by their usual headings.
/// Answers the cards and how many rows held nothing to make one from.
#[must_use]
pub fn csv_to_vcards(text: &str) -> (Vec<String>, usize) {
    let rows = csv_rows(text);
    let Some((head, body)) = rows.split_first() else {
        return (Vec::new(), 0);
    };
    let head: Vec<String> = head.iter().map(|h| h.trim().to_ascii_lowercase()).collect();
    let col = |names: &[&str]| head.iter().position(|h| names.contains(&h.as_str()));
    let given = col(&["first name", "given name", "firstname"]);
    let middle = col(&["middle name", "additional name"]);
    let family = col(&["last name", "family name", "surname", "lastname"]);
    let full = col(&["name", "full name", "display name"]);
    let company = col(&[
        "company",
        "organization",
        "organization 1 - name",
        "organization name",
    ]);
    // Outlook's "Title" is the name's prefix ("Dr."), its job title "Job Title": with both
    // there, "Title" was taken as the job and "Dr." became the person's title.
    let job = col(&["job title", "organization 1 - title", "organization title"]);
    let bare_title = col(&["title"]);
    let title = job.or(bare_title);
    let prefix = col(&["name prefix", "prefix"]).or(job.and(bare_title));
    let suffix = col(&["name suffix", "suffix"]);
    let birthday = col(&["birthday"]);
    let notes = col(&["notes", "note"]);
    let is_phone = |h: &str| {
        // Not "Phonetic First Name": the reading of a name is not a number.
        (h.contains("phone") || h.contains("mobile"))
            && !h.contains("type")
            && !h.contains("label")
            && !h.starts_with("phonetic")
    };
    let is_email = |h: &str| {
        (h.contains("e-mail") || h.contains("email"))
            && !h.contains("type")
            && !h.contains("label")
            && !h.contains("display")
    };
    let (mut cards, mut skipped) = (Vec::new(), 0);
    for row in body {
        let get = |i: Option<usize>| {
            i.and_then(|i| row.get(i))
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
        };
        let (g, m, f) = (get(given), get(middle), get(family));
        let name = get(full).map(ToString::to_string).or_else(|| {
            let joined = [g, m, f]
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            (!joined.is_empty()).then_some(joined)
        });
        let mut phones = Vec::new();
        let mut emails = Vec::new();
        for (i, h) in head.iter().enumerate() {
            let Some(v) = row.get(i).map(|v| v.trim()).filter(|v| !v.is_empty()) else {
                continue;
            };
            // Google keeps several numbers in one cell, " ::: " between them.
            for one in v.split(":::").map(str::trim).filter(|x| !x.is_empty()) {
                if is_phone(h) {
                    phones.push(one.to_string());
                } else if is_email(h) {
                    emails.push(one.to_string());
                }
            }
        }
        let org = get(company);
        if name.is_none() && org.is_none() && phones.is_empty() && emails.is_empty() {
            skipped += 1;
            continue;
        }
        let mut card = String::from("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        card.push_str(&format!(
            "N:{};{};{};{};{}\r\n",
            vcard_escape(f.unwrap_or("")),
            vcard_escape(g.unwrap_or(if f.is_none() {
                name.as_deref().unwrap_or("")
            } else {
                ""
            })),
            vcard_escape(m.unwrap_or("")),
            vcard_escape(get(prefix).unwrap_or("")),
            vcard_escape(get(suffix).unwrap_or(""))
        ));
        card.push_str(&format!(
            "FN:{}\r\n",
            vcard_escape(name.as_deref().or(org).unwrap_or(""))
        ));
        if let Some(o) = org {
            card.push_str(&format!("ORG:{}\r\n", vcard_escape(o)));
        }
        if let Some(t) = get(title) {
            card.push_str(&format!("TITLE:{}\r\n", vcard_escape(t)));
        }
        for p in &phones {
            card.push_str(&format!("TEL;TYPE=CELL:{}\r\n", vcard_escape(p)));
        }
        for e in &emails {
            card.push_str(&format!("EMAIL;TYPE=INTERNET:{}\r\n", vcard_escape(e)));
        }
        match get(birthday).and_then(csv_birthday) {
            Some((None, month, day)) => card.push_str(&format!(
                "BDAY;X-APPLE-OMIT-YEAR=1604:1604-{month:02}-{day:02}\r\n"
            )),
            Some((Some(year), month, day)) => {
                card.push_str(&format!("BDAY:{year:04}-{month:02}-{day:02}\r\n"));
            }
            None => {}
        }
        if let Some(n) = get(notes) {
            card.push_str(&format!("NOTE:{}\r\n", vcard_escape(n)));
        }
        card.push_str("END:VCARD\r\n");
        cards.push(card);
    }
    (cards, skipped)
}

/// Add the people in a vCard file to Contacts. Answers with the ids Apple gave them, and
/// how many cards it could not read.
pub async fn import_vcards(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    vcard: &str,
) -> Result<(Vec<String>, usize)> {
    // Apple's own ceiling for an import, checked here so the refusal says why.
    if vcard.len() > 10 * 1024 * 1024 {
        return Err(Error::Refused(
            "That file is larger than the 10 MB Contacts takes at once.".into(),
        ));
    }
    if !vcard.contains("BEGIN:VCARD") {
        return Err(Error::Refused("That file holds no contacts.".into()));
    }
    let text = co_text(
        http,
        root,
        params,
        ledger,
        reqwest::Method::POST,
        "co/import/",
        vcard.to_string(),
    )
    .await?;
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let ids = body
        .get("importedContactIds")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    let failed = body
        .get("errors")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    Ok((ids, failed))
}

/// Say which card is this person's own — "My Card" on every device.
pub async fn set_me_card(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    ledger: &Ledger,
    id: &str,
) -> Result<()> {
    let body = serde_json::json!({ "meCardId": id });
    co_text(
        http,
        root,
        params,
        ledger,
        reqwest::Method::PUT,
        "co/mecardid",
        body.to_string(),
    )
    .await?;
    Ok(())
}

/// Which card is this person's own, as `/co/startup` says.
pub fn me_card(body: &Value) -> Option<String> {
    body.get("meCardId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// Fetch the address book and reconcile it into the library.
pub async fn sync(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
    library: &mut quince_core::Library,
) -> Result<usize> {
    let book = fetch(http, root, params).await?;
    apply(library, book)
}

/// Ask Apple for the address book. Touches no local state, so a refresh can do the network
/// half without holding the library the interface is reading.
pub async fn fetch(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<AddressBook> {
    let body = startup(http, root, params).await?;
    Ok(AddressBook {
        contacts: parse(&body),
        lists: parse_lists(&body),
        me: me_card(&body),
        addresses: parse_addresses(&body),
        details: parse_details(&body),
    })
}

/// The whole of `/co/startup`: the address book, the tokens, and everything else Apple
/// sends with it.
async fn startup(
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Value> {
    let mut query = params;
    query.push(("clientVersion".into(), "2.1".into()));
    // Fixed rather than the machine's own, and deliberately. Asked with en_US, fr_FR and
    // ja_JP, Apple sent back byte-identical contacts, ordering and section headers — the
    // parameter changes nothing this app reads, and sorting happens locally anyway.
    query.push(("locale".into(), "en_US".into()));
    query.push(("order".into(), "last,first".into()));

    let url = format!("{}/co/startup", root.trim_end_matches('/'));
    let response = http.get(url).query(&query).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if status.as_u16() == 450 {
        return Err(Error::ReauthRequired);
    }
    if !status.is_success() {
        crate::approval_needed(status, &text)?;
        // Apple can't take it now (429, 503 under load): busy, asked again later — never a
        // broken answer, which the outbox gives up on after a few tries.
        if status.as_u16() == 429 || status.is_server_error() {
            return Err(Error::Busy);
        }
        return Err(Error::Protocol(format!(
            "/co/startup returned {status}: {}",
            text.chars().take(160).collect::<String>()
        )));
    }

    let mut body: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Protocol(format!("/co/startup returned invalid JSON: {e}")))?;

    // `contactsOrder` names every card in the book; `contacts` may hold only the first of
    // them, as icloud.com reads a large book a page at a time (`co/contacts/card/?offset&
    // limit`). The rest is read the same way, so a large address book is never cut short —
    // and stored short, the cards it missed would read as deleted.
    let wanted = body
        .get("contactsOrder")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut have = body
        .get("contacts")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let mut pages = 0;
    while have < wanted && pages < 200 {
        pages += 1;
        // As icloud.com sends every Contacts request: with the book's tokens beside the rest.
        let mut page_query = query.clone();
        for key in ["syncToken", "prefToken"] {
            if let Some(token) = body.get(key).and_then(Value::as_str) {
                page_query.push((key.into(), token.to_string()));
            }
        }
        page_query.push(("offset".into(), have.to_string()));
        page_query.push(("limit".into(), "500".into()));
        let url = format!("{}/co/contacts/card/", root.trim_end_matches('/'));
        let response = http.get(url).query(&page_query).send().await?;
        if response.status().as_u16() == 450 {
            return Err(Error::ReauthRequired);
        }
        if !response.status().is_success() {
            return Err(Error::Protocol(format!(
                "Apple stopped part-way through the address book ({}), so it was not stored short.",
                response.status()
            )));
        }
        let page: Value = response.json().await?;
        let more = page
            .get("contacts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if more.is_empty() {
            return Err(Error::Protocol(
                "Apple listed more contacts than it would send, so the address book was not stored short."
                    .into(),
            ));
        }
        have += more.len();
        if let Some(Value::Array(all)) = body.get_mut("contacts") {
            all.extend(more);
        }
    }
    // Stopped at the page limit with cards still unread: stored, the book would be short, and
    // every card it missed would read as deleted.
    if have < wanted {
        return Err(Error::Protocol(
            "The address book is larger than Skykeep reads in one go, so it was not stored short."
                .into(),
        ));
    }
    Ok(body)
}

/// Store a fetched address book. Touches no network.
pub fn apply(library: &mut quince_core::Library, book: AddressBook) -> Result<usize> {
    library.atomically(|library| apply_whole(library, book))
}

fn apply_whole(library: &mut quince_core::Library, book: AddressBook) -> Result<usize> {
    library.put_contacts(&book.contacts)?;
    library.put_addresses(&book.addresses, true)?;
    library.put_contact_details(&book.details, true)?;
    library.set_state(
        LISTS_KEY,
        &serde_json::to_string(&book.lists).unwrap_or_else(|_| "[]".into()),
    )?;
    library.set_state(ME_KEY, book.me.as_deref().unwrap_or(""))?;
    library.mark_synced("contacts", None)?;
    Ok(book.contacts.len())
}

#[cfg(test)]
mod busy_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Apple under load answers 503 (seen on Android, several syncs at once): busy, asked
    /// again later — not "something unexpected", which the outbox stops trying after a few.
    #[tokio::test]
    async fn an_overloaded_apple_is_busy_not_broken() {
        for status in [429, 503] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/co/startup"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let got = fetch(&reqwest::Client::new(), &server.uri(), vec![]).await;
            assert!(matches!(got, Err(Error::Busy)), "{status}: {:?}", got.err());
        }
    }
}

#[cfg(test)]
mod list_tests {

    /// The whole book read again after one card's address moved on the phone: the pass names
    /// that card alone, and a book read again unchanged names none — so Contacts redraws one
    /// row, or nothing, rather than reading every card again.
    #[test]
    fn a_whole_book_read_again_names_only_the_card_that_moved() {
        use quince_core::contacts::{Contact, ContactDetails, ContactMethod, PostalAddress};
        let person = |guid: &str, last: &str| Contact {
            guid: guid.into(),
            first_name: Some("Sam".into()),
            middle_name: None,
            last_name: Some(last.into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            etag: Some("e1".into()),
            methods: vec![ContactMethod {
                kind: "email".into(),
                label: Some("home".into()),
                value: format!("{guid}@example.com"),
            }],
        };
        let book = |city: &str| super::AddressBook {
            contacts: vec![person("A", "Rivera"), person("B", "Turner")],
            lists: Vec::new(),
            me: Some("A".into()),
            addresses: vec![(
                "B".into(),
                vec![PostalAddress {
                    label: Some("HOME".into()),
                    city: Some(city.into()),
                    ..PostalAddress::default()
                }],
            )],
            details: vec![(
                "A".into(),
                ContactDetails {
                    nickname: Some("Al".into()),
                    ..ContactDetails::default()
                },
            )],
        };
        let mut library = quince_core::Library::in_memory().unwrap();
        super::apply(&mut library, book("Lisbon")).unwrap();

        let (_, same) = library.changing_items(&["sync_state"], |l| {
            super::apply(l, book("Lisbon")).unwrap();
        });
        assert_eq!(
            same.for_service("contacts"),
            Some(quince_core::changes::Items::default()),
            "the same book read again changes no card"
        );

        let (_, moved) = library.changing_items(&["sync_state"], |l| {
            super::apply(l, book("Porto")).unwrap();
        });
        let items = moved.for_service("contacts").expect("placed");
        assert_eq!(items.changed, ["B"]);
        assert!(items.added.is_empty() && items.removed.is_empty());
    }

    /// The tokens a pass keeps come back as they went in; anything else kept there (a library
    /// from before) reads as none, so the book is read whole rather than trusted.
    #[test]
    fn kept_tokens_come_back_whole_or_not_at_all() {
        let t = super::Tokens {
            sync: "S1".into(),
            pref: "P1".into(),
        };
        assert_eq!(super::Tokens::from_stored(&t.stored()), Some(t));
        assert_eq!(super::Tokens::from_stored("an old position"), None);
    }

    /// The phone renamed the person while the form here was open on their notes: saving
    /// writes the notes and keeps the phone's name — and Apple's numbers, untouched here —
    /// against the version Apple holds now.
    #[test]
    fn a_save_writes_only_what_the_form_changed() {
        use quince_core::contacts::{Contact, ContactMethod};
        let person = |last: &str, notes: Option<&str>| Contact {
            guid: "C1".into(),
            first_name: Some("Asha".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some(last.into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: notes.map(Into::into),
            etag: Some("old".into()),
            methods: vec![ContactMethod {
                kind: "phone".into(),
                label: None,
                value: "+91 1".into(),
            }],
        };
        let base = person("Rao", None);
        let mine = person("Rao", Some("Call after 6"));
        let apple = serde_json::json!({
            "contactId": "C1", "etag": "new", "firstName": "Asha", "lastName": "Rao-Iyer",
            "phones": [{ "field": "+91 1" }, { "field": "+91 2", "label": "work" }],
            "streetAddresses": [{ "field": { "city": "Pune" } }],
        });
        let card = super::card_merged(&mine, Some(&apple), Some(&base));
        assert_eq!(card["lastName"], "Rao-Iyer", "the phone's rename stands");
        assert_eq!(card["notes"], "Call after 6", "the edit here is written");
        assert_eq!(
            card["phones"].as_array().unwrap().len(),
            2,
            "numbers not edited here keep Apple's"
        );
        assert_eq!(card["streetAddresses"][0]["field"]["city"], "Pune");
        assert_eq!(card["etag"], "new", "against the version Apple holds now");
    }
    /// The form changed one number while the phone added another: the change goes over
    /// Apple's list, and the phone's number stays — written whole, the form's list dropped it.
    #[test]
    fn a_number_added_on_the_phone_survives_an_edit_of_another() {
        use quince_core::contacts::ContactMethod;
        let phone = |v: &str, l: Option<&str>| ContactMethod {
            kind: "phone".into(),
            label: l.map(Into::into),
            value: v.into(),
        };
        let mut base = super::parse(
            &serde_json::json!({ "contacts": [{ "contactId": "C1", "firstName": "A" }] }),
        )
        .remove(0);
        base.methods = vec![phone("+1 100", Some("home")), phone("+1 200", Some("work"))];
        let mut mine = base.clone();
        mine.methods = vec![phone("+1 101", Some("home")), phone("+1 200", Some("work"))];
        let apple = serde_json::json!({
            "contactId": "C1", "etag": "e2", "firstName": "A",
            "phones": [
                { "field": "+1 100", "label": "home" },
                { "field": "+1 200", "label": "work" },
                { "field": "+1 300", "label": "mobile" }
            ],
        });
        let card = super::card_merged(&mine, Some(&apple), Some(&base));
        let fields: Vec<&str> = card["phones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["field"].as_str().unwrap())
            .collect();
        assert_eq!(
            fields,
            ["+1 101", "+1 200", "+1 300"],
            "the edit in place, the phone's kept"
        );

        // Taken out here: gone, the phone's still kept.
        let mut fewer = base.clone();
        fewer.methods = vec![phone("+1 200", Some("work"))];
        let card = super::card_merged(&fewer, Some(&apple), Some(&base));
        let fields: Vec<&str> = card["phones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["field"].as_str().unwrap())
            .collect();
        assert_eq!(fields, ["+1 200", "+1 300"]);
    }

    #[test]
    fn a_refused_group_write_is_an_error_not_a_save() {
        let busy = serde_json::json!({ "errors": [{ "groupId": "G", "etag": "e", "errorCode": 409, "errorMessage": "Conflict" }] });
        assert!(matches!(
            super::refused_group(&busy),
            Err(crate::Error::Conflict)
        ));
        let other = serde_json::json!({ "errors": [{ "groupId": "G", "errorCode": 400, "errorMessage": "Bad" }] });
        assert!(matches!(
            super::refused_group(&other),
            Err(crate::Error::Protocol(_))
        ));
        assert!(super::refused_group(&serde_json::json!({ "groups": [], "errors": [] })).is_ok());
    }

    use super::*;

    /// Lists come out in the account's own order — `groupsOrder` — with their members.
    #[test]
    fn lists_are_read_in_the_accounts_order() {
        let body = serde_json::json!({
            "groups": [
                { "groupId": "B", "name": "Family", "contactIds": ["c1", "c2"], "etag": "e1" },
                { "groupId": "A", "name": "Work", "contactIds": [], "etag": "e2" }
            ],
            "collections": [{ "groupsOrder": ["A", "B"] }]
        });
        let lists = parse_lists(&body);
        assert_eq!(
            lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["Work", "Family"]
        );
        assert_eq!(lists[1].members, ["c1", "c2"]);
        assert_eq!(lists[1].etag.as_deref(), Some("e1"));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_card_already_in_the_book_is_known() {
        use quince_core::contacts::{Contact, ContactMethod};
        let person = |first: &str, last: &str, phones: &[&str]| Contact {
            guid: "G".into(),
            first_name: Some(first.into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some(last.into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: phones
                .iter()
                .map(|p| ContactMethod {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: (*p).into(),
                })
                .collect(),
        };
        let book = vec![person("Asha", "Rao", &["+91 90000 00002"])];
        let card = |n: &str, tel: &str| {
            format!("BEGIN:VCARD\r\nVERSION:3.0\r\nN:{n}\r\nFN:x\r\nTEL;type=CELL:{tel}\r\nEND:VCARD\r\n")
        };
        assert!(
            super::already_in(&card("Rao;Asha;;;", "090000 00002"), &book),
            "the same person, the number written another way"
        );
        assert!(
            !super::already_in(&card("Rao;Asha;;;", "+91 90000 00004"), &book),
            "a number the book lacks is news"
        );
        assert!(
            !super::already_in(&card("Rao;Ashwin;;;", "+91 90000 00002"), &book),
            "another name is someone else"
        );
    }

    use super::*;

    /// A Google Contacts export: names, a company with a comma, two numbers in one cell, an
    /// email. An empty row is not a contact; a row with only a note is skipped and counted.
    #[test]
    fn a_csv_export_becomes_vcards() {
        let csv = "First Name,Last Name,Organization 1 - Name,Phone 1 - Value,E-mail 1 - Value,Notes\r\n\
                   Zoya,Ahuja,\"Acme, Inc\",+91 90000 00001 ::: +91 80000 00000,zoya@example.com,\"Line one\nline two\"\r\n\
                   ,,,,,\r\n\
                   ,,,,,only a note\r\n";
        let (cards, skipped) = csv_to_vcards(csv);
        assert_eq!((cards.len(), skipped), (1, 1));
        let c = &cards[0];
        assert!(c.contains("N:Ahuja;Zoya;;;"), "{c}");
        assert!(c.contains("FN:Zoya Ahuja"));
        assert!(c.contains("ORG:Acme\\, Inc"));
        assert_eq!(c.matches("TEL;TYPE=CELL:").count(), 2);
        assert!(c.contains("EMAIL;TYPE=INTERNET:zoya@example.com"));
        assert!(c.contains("NOTE:Line one\\nline two"));
    }

    /// A vCard 2.1 card from an Android export, its name in quoted-printable, is the person
    /// already in the book: importing the same export again copied everyone not named in ASCII.
    #[test]
    fn a_quoted_printable_card_is_known_when_its_person_is_in_the_book() {
        let book = vec![Contact {
            guid: "1".into(),
            first_name: Some("\u{906}\u{936}\u{93e}".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Rao".into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: vec![quince_core::contacts::ContactMethod {
                kind: "phone".into(),
                label: None,
                value: "+1 555 010 0150".into(),
            }],
        }];
        let card = "BEGIN:VCARD\r\nVERSION:2.1\r\n\
                    N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Rao;=E0=A4=86=E0=A4=B6=\r\n=E0=A4=BE;;;\r\n\
                    TEL;CELL:+1 555 010 0150\r\nEND:VCARD\r\n";
        assert!(already_in(card, &book));
        let stranger = card.replace("Rao;", "Iyer;");
        assert!(!already_in(&stranger, &book));
    }

    /// Outlook's CSV: "Title" is the name's prefix and "Job Title" the job; birthdays come as
    /// M/D/YYYY, and an empty one as 0/0/00.
    #[test]
    fn an_outlook_csv_keeps_its_prefix_title_and_birthday() {
        let csv = "Title,First Name,Last Name,Job Title,Mobile Phone,E-mail Address,Birthday\r\n\
                   Dr.,Mira,Okafor,Pilot,(555) 010-0103,mira@example.com,3/14/1990\r\n\
                   ,Tomas,Reyes,,(555) 010-0104,,0/0/00\r\n";
        let (cards, _) = csv_to_vcards(csv);
        assert!(cards[0].contains("N:Okafor;Mira;;Dr.;\r\n"), "{}", cards[0]);
        assert!(cards[0].contains("TITLE:Pilot\r\n"), "{}", cards[0]);
        assert!(!cards[0].contains("TITLE:Dr."), "{}", cards[0]);
        assert!(cards[0].contains("BDAY:1990-03-14\r\n"), "{}", cards[0]);
        assert!(!cards[1].contains("BDAY"), "{}", cards[1]);
        // A lone "Title" column is still the job, as Google's older exports have it.
        let (one, _) = csv_to_vcards("First Name,Title\r\nAnn,Editor\r\n");
        assert!(one[0].contains("TITLE:Editor"), "{}", one[0]);
    }

    #[test]
    fn spreadsheet_birthdays_are_read_or_left_out() {
        assert_eq!(csv_birthday("1990-03-14"), Some((Some(1990), 3, 14)));
        assert_eq!(csv_birthday("3/14/1990"), Some((Some(1990), 3, 14)));
        assert_eq!(csv_birthday("--03-14"), Some((None, 3, 14)));
        assert_eq!(csv_birthday("0/0/00"), None);
        assert_eq!(csv_birthday("3/14/90"), None);
        assert_eq!(csv_birthday("someday"), None);
        assert_eq!(csv_birthday("13/40/1990"), None);
    }

    /// A note written over several lines in a spreadsheet (Windows line ends inside the quoted
    /// cell) comes in as one note, with no bare carriage return left in the card.
    #[test]
    fn a_csv_note_over_several_lines_is_one_clean_note() {
        let csv = "First Name,Notes\r\nAnn,\"line one\r\nline two\"\r\n";
        let (cards, _) = csv_to_vcards(csv);
        let c = &cards[0];
        assert!(c.contains("NOTE:line one\\nline two\r\n"), "{c:?}");
        assert!(!c.replace("\r\n", "").contains('\r'), "a bare CR in {c:?}");
    }

    /// Google's export has Phonetic First/Middle/Last Name columns, and its job title under
    /// "Organization Title": the reading of a name is not a phone number, and the title stays.
    #[test]
    fn a_google_csv_keeps_phonetic_names_out_of_the_numbers() {
        let csv = "First Name,Phonetic First Name,Last Name,Phonetic Last Name,Organization Name,Organization Title,Phone 1 - Label,Phone 1 - Value\r\n\
                   Hana,\u{306f}\u{306a},Mori,\u{3082}\u{308a},Example Works,Designer,Mobile,+1 555 010 0177\r\n";
        let (cards, _) = csv_to_vcards(csv);
        let c = &cards[0];
        assert_eq!(c.matches("TEL").count(), 1, "{c}");
        assert!(c.contains("TEL;TYPE=CELL:+1 555 010 0177"), "{c}");
        assert!(c.contains("TITLE:Designer"), "{c}");
        assert!(c.contains("ORG:Example Works"), "{c}");
    }

    /// A card that holds the same number twice, one copy taken out in the form: one copy goes,
    /// one stays. Both used to stay, as though nothing had been asked.
    #[test]
    fn taking_out_one_of_two_copies_of_a_number_takes_out_one() {
        let a = || serde_json::json!({ "field": "+1 555 010 0123", "label": "MOBILE" });
        let theirs = [a(), a()];
        let one = ("+1 555 010 0123".to_string(), Some("MOBILE".to_string()));
        let out = merge_methods(
            "phone",
            &theirs,
            &[one.clone(), one.clone()],
            std::slice::from_ref(&one),
        );
        assert_eq!(out.as_array().map(Vec::len), Some(1), "{out}");
        // Both taken out: none left.
        let out = merge_methods("phone", &theirs, &[one.clone(), one], &[]);
        assert_eq!(out.as_array().map(Vec::len), Some(0), "{out}");
    }

    /// A file of many cards is read card by card and sent in batches under Apple's limits.
    #[test]
    fn many_cards_are_split_and_batched() {
        let one = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:A\r\nEND:VCARD\r\n";
        let text = one.repeat(5);
        let cards = split_vcards(&text);
        assert_eq!(cards.len(), 5);
        assert_eq!(batch_vcards(&cards, 1_000_000, 2).len(), 3);
        assert_eq!(batch_vcards(&cards, one.len() * 2, 100).len(), 3);
        assert_eq!(batch_vcards(&cards, 1_000_000, 100).concat(), text);
    }

    /// A book whose startup names more cards than it holds is read to the end, a page at a
    /// time with its tokens, as icloud.com reads one — never stored short.
    #[tokio::test]
    async fn a_large_book_is_read_to_the_end() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let card = |id: &str| serde_json::json!({ "contactId": id, "firstName": id });
        Mock::given(method("GET"))
            .and(path("/co/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "syncToken": "S", "prefToken": "P",
                "contactsOrder": ["a", "b", "c"], "contacts": [card("a")], "groups": []
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/co/contacts/card/"))
            .and(query_param("offset", "1"))
            .and(query_param("syncToken", "S"))
            .and(query_param("prefToken", "P"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "contacts": [card("b"), card("c")] })),
            )
            .mount(&server)
            .await;
        let book = fetch(&reqwest::Client::new(), &server.uri(), vec![])
            .await
            .unwrap();
        let ids: Vec<_> = book.contacts.iter().map(|c| c.guid.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    /// Apple naming cards it then will not send is a failure, not a smaller address book.
    #[tokio::test]
    async fn a_book_apple_will_not_finish_is_refused_not_stored_short() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/co/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "syncToken": "S", "prefToken": "P", "contactsOrder": ["a", "b"],
                "contacts": [{ "contactId": "a" }], "groups": []
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/co/contacts/card/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "contacts": [] })),
            )
            .mount(&server)
            .await;
        assert!(fetch(&reqwest::Client::new(), &server.uri(), vec![])
            .await
            .is_err());
    }

    /// A book still unread when the page limit is reached is refused too: returned as it
    /// stood, every card past the limit read as deleted.
    #[tokio::test]
    async fn a_book_past_the_page_limit_is_refused_not_stored_short() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let order: Vec<String> = (0..300).map(|i| format!("c{i}")).collect();
        Mock::given(method("GET"))
            .and(path("/co/startup"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "syncToken": "S", "prefToken": "P", "contactsOrder": order,
                "contacts": [{ "contactId": "c0" }], "groups": []
            })))
            .mount(&server)
            .await;
        // One card a page: 200 pages leave a hundred unread.
        Mock::given(method("GET"))
            .and(path("/co/contacts/card/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "contacts": [{ "contactId": "x" }] })),
            )
            .mount(&server)
            .await;
        assert!(fetch(&reqwest::Client::new(), &server.uri(), vec![])
            .await
            .is_err());
    }

    #[test]
    fn a_changeset_says_whether_anything_moved() {
        let mut t = Tokens {
            sync: "S1".into(),
            pref: "P".into(),
        };
        assert!(!changeset_moved(
            &serde_json::json!({"syncToken":"S1","prefToken":"P","contacts":[]}),
            &mut t
        ));
        assert!(changeset_moved(
            &serde_json::json!({"syncToken":"S2"}),
            &mut t
        ));
        assert_eq!(t.sync, "S2");
        assert!(changeset_moved(
            &serde_json::json!({"syncToken":"S2","deletes":{"contactIds":["x"]}}),
            &mut t
        ));
        assert!(changeset_moved(
            &serde_json::json!({"contacts":[{"contactId":"y"}]}),
            &mut t
        ));
    }

    use serde_json::json;

    #[test]
    fn a_contact_is_read_with_its_methods() {
        let contacts = parse(&json!({ "contacts": [{
            "contactId": "abc",
            "firstName": "Meera",
            "lastName": "Iyer",
            "phones": [{ "field": "+919000000001", "label": "MOBILE" }],
            "emailAddresses": [{ "field": "meera@example.com" }],
            "etag": "e1"
        }]}));
        assert_eq!(contacts.len(), 1);
        let c = &contacts[0];
        assert_eq!(c.display_name(), "Meera Iyer");
        assert_eq!(c.methods.len(), 2);
        assert_eq!(c.methods[0].kind, "phone");
        assert_eq!(c.methods[0].label.as_deref(), Some("MOBILE"));
        assert_eq!(c.methods[1].kind, "email");
    }

    /// Most fields are optional — this account has contacts with only a first name and a
    /// number — so absence must not lose the record.
    #[test]
    fn a_contact_with_almost_nothing_still_parses() {
        let contacts = parse(&json!({ "contacts": [{
            "contactId": "abc",
            "firstName": "Amma",
            "phones": [{ "field": "+919000000001" }]
        }]}));
        assert_eq!(contacts[0].display_name(), "Amma");
        assert!(contacts[0].last_name.is_none());
        assert!(contacts[0].methods[0].label.is_none());
    }

    #[test]
    fn a_contact_without_an_id_is_skipped() {
        assert!(parse(&json!({ "contacts": [{ "firstName": "Nobody" }] })).is_empty());
    }

    /// Blank values must not become empty rows in the detail view.
    #[test]
    fn blank_values_are_dropped() {
        let contacts = parse(&json!({ "contacts": [{
            "contactId": "abc", "firstName": "  ", "lastName": "Iyer",
            "phones": [{ "field": "" }, { "field": "+9111" }]
        }]}));
        assert!(contacts[0].first_name.is_none());
        assert_eq!(contacts[0].methods.len(), 1);
    }

    #[test]
    fn an_empty_body_is_an_empty_address_book() {
        assert!(parse(&json!({})).is_empty());
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// A card as an iPhone keeps it, full of what this app does not show: instant messaging
    /// (IMPP), social profiles, related names, custom labels, dates, a picture with its crop,
    /// and fields of Apple's own. A first name changed here, and only that, goes back; every
    /// other field is Apple's, byte for byte — the whole card too, through `card_written`.
    #[test]
    fn an_edit_keeps_every_field_this_app_does_not_show() {
        let existing = json!({
            "contactId": "C1", "etag": "e7", "isCompany": false,
            "firstName": "Ada", "lastName": "Lovelace", "companyName": "Analytical",
            "jobTitle": "Mathematician", "nickName": "Countess",
            "phones": [
                { "label": "MOBILE", "field": "+44 20 7946 0000" },
                { "label": "_$!<Pager>!$_", "field": "555-0100" },
                { "label": "Lab bench", "field": "555-0199" }
            ],
            "emailAddresses": [{ "label": "WORK", "field": "ada@example.com" }],
            "IMs": [{ "label": "SKYPE", "field": { "userName": "ada.l", "IMService": "Skype" } }],
            "profiles": [{ "label": "TWITTER", "field": "https://x.com/ada", "user": "ada" }],
            "relatedNames": [{ "label": "_$!<Mother>!$_", "field": "Anne Isabella Milbanke" }],
            "dates": [{ "label": "_$!<Anniversary>!$_", "field": "1835-07-08" }],
            "urls": [{ "label": "HOMEPAGE", "field": "https://example.com/ada" }],
            "streetAddresses": [{ "label": "HOME", "field": { "street": "12 St James's Sq", "city": "London" } }],
            "photo": { "url": "https://p.example/ada.jpg", "crop": { "x": 1, "y": 2, "width": 300, "height": 300 }, "signature": "sig" },
            "X-ABShowAs": "PERSON",
            "X-SOCIALPROFILE": "x-apple:ada",
            "someFieldAppleAddsLater": { "nested": true }
        });
        let base = contact_from(&existing).expect("a person");
        let mut mine = base.clone();
        mine.first_name = Some("Augusta Ada".into());
        for written in [card_merged(&mine, Some(&existing), Some(&base)), {
            let card = parse_cards(&json!({ "contacts": [existing.clone()] }))
                .pop()
                .unwrap();
            card_written(
                &mine,
                Some(&card.details),
                Some(&existing),
                Some(&base),
                Some(&card.details),
            )
        }] {
            let (Some(written), Some(was)) = (written.as_object(), existing.as_object()) else {
                panic!("a card is an object");
            };
            for (key, value) in was {
                if key == "firstName" {
                    continue;
                }
                assert_eq!(
                    written.get(key),
                    Some(value),
                    "{key} changed or went missing"
                );
            }
            assert_eq!(written["firstName"], "Augusta Ada");
            assert_eq!(written.len(), was.len(), "nothing added: {written:?}");
        }
    }

    fn person() -> Contact {
        Contact {
            guid: "C1".into(),
            first_name: Some("Ada".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Lovelace".into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: Some("v1".into()),
            methods: vec![
                ContactMethod {
                    kind: "phone".into(),
                    label: Some("mobile".into()),
                    value: "+441234567890".into(),
                },
                ContactMethod {
                    kind: "email".into(),
                    label: None,
                    value: "ada@example.com".into(),
                },
            ],
        }
    }

    fn tokens() -> Ledger {
        Ledger::holding(Tokens {
            sync: "SYNC".into(),
            pref: "PREF".into(),
        })
    }

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "contacts": [] })))
            .mount(&server)
            .await;
        server
    }

    async fn sent(server: &MockServer) -> (Request, Value) {
        let request = server.received_requests().await.unwrap().remove(0);
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        (request, body["contacts"][0].clone())
    }

    /// The one that matters. Apple's card carries fields this app does not model — related
    /// names, social profiles, a photo. Writing only what we understand would drop all of
    /// it, so an edit starts from Apple's own card and changes what it means to change.
    #[tokio::test]
    async fn an_edit_keeps_the_fields_this_app_does_not_model() {
        let server = server().await;
        let apples = json!({
            "contactId": "C1",
            "firstName": "Ada",
            "photo": { "url": "https://example.invalid/ada.jpg" },
            "relatedNames": [{ "label": "sister", "field": "Anne" }],
            "etag": "v1"
        });
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            Some(&apples),
        )
        .await
        .unwrap();

        let (_, card) = sent(&server).await;
        assert_eq!(
            card["photo"]["url"], "https://example.invalid/ada.jpg",
            "a photo must survive an edit"
        );
        assert_eq!(card["relatedNames"][0]["field"], "Anne");
        assert_eq!(card["lastName"], "Lovelace", "and the change still lands");
    }

    /// Every write carries the tokens Apple handed out, or the service refuses it.
    #[tokio::test]
    async fn the_tokens_go_with_every_write() {
        let server = server().await;
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            None,
        )
        .await
        .unwrap();
        let (request, _) = sent(&server).await;
        let query: std::collections::HashMap<_, _> = request.url.query_pairs().collect();
        assert_eq!(query["syncToken"], "SYNC");
        assert_eq!(query["prefToken"], "PREF");
    }

    /// Apple groups methods by kind, each as `field` with an optional `label`.
    #[tokio::test]
    async fn methods_are_grouped_the_way_apple_groups_them() {
        let server = server().await;
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            None,
        )
        .await
        .unwrap();
        let (_, card) = sent(&server).await;
        assert_eq!(card["phones"][0]["field"], "+441234567890");
        assert_eq!(card["phones"][0]["label"], "mobile");
        assert_eq!(card["emailAddresses"][0]["field"], "ada@example.com");
        assert!(
            card["emailAddresses"][0].get("label").is_none(),
            "no label means no label, not an empty one"
        );
        assert_eq!(card["urls"], json!([]));
    }

    /// Clearing a field means sending it empty. Leaving the key out would leave whatever
    /// Apple already has, so the change would appear to do nothing.
    #[tokio::test]
    async fn clearing_a_field_sends_it_empty_rather_than_omitting_it() {
        let server = server().await;
        let mut cleared = person();
        cleared.company = None;
        cleared.notes = None;
        update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &cleared,
            Some(&json!({ "companyName": "Old Co", "notes": "old note" })),
        )
        .await
        .unwrap();
        let (_, card) = sent(&server).await;
        assert_eq!(card["companyName"], "");
        assert_eq!(card["notes"], "");
    }

    #[tokio::test]
    async fn a_contact_with_no_name_at_all_is_refused_before_anything_is_sent() {
        let server = server().await;
        let nameless = Contact {
            first_name: None,
            last_name: None,
            company: None,
            methods: vec![],
            ..person()
        };
        assert!(create(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &nameless
        )
        .await
        .is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// A create sent again under its id — the answer lost, a queue replayed after a crash —
    /// finds the card at Apple and writes nothing; one not there yet is written once.
    #[tokio::test]
    async fn a_named_create_is_made_once() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*/co/startup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "contacts": [{ "contactId": "G1", "firstName": "Ada", "etag": "e" }],
                "syncToken": "s", "prefToken": "p"
            })))
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "contacts": [] })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        create_named(&http, &server.uri(), vec![], &tokens(), "G1", &person())
            .await
            .unwrap();
        let writes = |rs: &[Request]| {
            rs.iter()
                .filter(|r| r.url.path().ends_with("/card"))
                .count()
        };
        assert_eq!(
            writes(&server.received_requests().await.unwrap()),
            0,
            "already there"
        );
        create_named(&http, &server.uri(), vec![], &tokens(), "G2", &person())
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(writes(&requests), 1, "made once");
        let card: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
        assert_eq!(
            card["contacts"][0]["contactId"], "G2",
            "under the id chosen beforehand"
        );
    }

    /// A create gets a fresh id and no etag: an etag would claim to be replacing something.
    #[tokio::test]
    async fn a_create_mints_its_own_id() {
        let server = server().await;
        let guid = create(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
        )
        .await
        .unwrap();
        assert_ne!(guid, "C1", "the id is minted, not carried over");

        let (request, card) = sent(&server).await;
        assert_eq!(request.method, reqwest::Method::POST);
        assert_eq!(card["contactId"], guid);
        assert!(card.get("etag").is_none(), "nothing to replace yet");
    }

    /// The etag sent is the one Apple holds now: a card edited on the phone since the last
    /// sync was refused as a conflict when the stored one went.
    #[tokio::test]
    async fn deleting_says_so_and_carries_apples_current_etag() {
        let server = server().await;
        Mock::given(path_regex(r".*/co/startup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "contacts": [{ "contactId": "C1", "etag": "v2" }],
                "syncToken": "SYNC-NOW", "prefToken": "PREF",
            })))
            .mount(&server)
            .await;
        delete(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
        )
        .await
        .unwrap();
        let request = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.url.path().ends_with("/co/contacts/card"))
            .expect("the delete was sent");
        let query: std::collections::HashMap<_, _> = request.url.query_pairs().collect();
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(query["method"], "DELETE");
        // The tokens of the read the etag came from, not older ones held before it.
        assert_eq!(query["syncToken"], "SYNC-NOW");
        assert_eq!(body["contacts"][0]["contactId"], "C1");
        assert_eq!(body["contacts"][0]["etag"], "v2");
    }

    /// A run of writes never sends a token twice: the next write carries the tokens Apple
    /// answered the last with, and where Apple's answer carried none the book is read again
    /// for fresh ones before the next write goes.
    #[tokio::test]
    async fn a_run_of_writes_never_sends_a_spent_token() {
        let server = MockServer::start().await;
        // The first write is answered with new tokens; the second without any.
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .and(wiremock::matchers::query_param("syncToken", "S1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "contacts": [], "syncToken": "S2", "prefToken": "P1",
            })))
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .and(wiremock::matchers::query_param("syncToken", "S2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "contacts": [] })))
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .and(wiremock::matchers::query_param("syncToken", "S3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "contacts": [] })))
            .mount(&server)
            .await;
        // A spent token is refused as Apple refuses one.
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .respond_with(ResponseTemplate::new(412))
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/startup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "contacts": [], "syncToken": "S3", "prefToken": "P1",
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let ledger = Ledger::holding(Tokens {
            sync: "S1".into(),
            pref: "P0".into(),
        });
        for _ in 0..3 {
            create(&http, &server.uri(), vec![], &ledger, &person())
                .await
                .expect("every write carries a live token");
        }
        let sent: Vec<(String, String)> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| {
                let q: std::collections::HashMap<_, _> = r.url.query_pairs().collect();
                (
                    r.url
                        .path()
                        .rsplit('/')
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    q.get("syncToken")
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                )
            })
            .collect();
        let pairs: Vec<(&str, &str)> = sent.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("card", "S1"),
                ("card", "S2"),
                ("startup", ""),
                ("card", "S3")
            ]
        );

        // A write that fails leaves no token behind either.
        let ledger = Ledger::holding(Tokens {
            sync: "OLD".into(),
            pref: "P0".into(),
        });
        assert!(matches!(
            create(&http, &server.uri(), vec![], &ledger, &person()).await,
            Err(Error::Conflict)
        ));
        assert!(ledger.held().is_none());
    }

    /// An import in batches: the second batch went with the first's token, which the first
    /// had spent. It now carries the token the first was answered with, or reads one.
    #[tokio::test]
    async fn an_import_in_batches_carries_a_live_token_for_each() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*/co/import/$"))
            .and(wiremock::matchers::query_param("syncToken", "S1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "importedContactIds": ["N1"], "syncToken": "S2", "prefToken": "P",
            })))
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/import/$"))
            .and(wiremock::matchers::query_param("syncToken", "S2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "importedContactIds": ["N2"] })),
            )
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/import/$"))
            .and(wiremock::matchers::query_param("syncToken", "S3"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "importedContactIds": ["N3"] })),
            )
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/import/$"))
            .respond_with(ResponseTemplate::new(412))
            .mount(&server)
            .await;
        Mock::given(path_regex(r".*/co/startup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "contacts": [], "syncToken": "S3", "prefToken": "P",
            })))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let ledger = Ledger::holding(Tokens {
            sync: "S1".into(),
            pref: "P".into(),
        });
        let vcard = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:A\r\nEND:VCARD\r\n";
        let mut ids = Vec::new();
        for _ in 0..3 {
            let (got, _) = import_vcards(&http, &server.uri(), vec![], &ledger, vcard)
                .await
                .expect("each batch carries a live token");
            ids.extend(got);
        }
        assert_eq!(ids, ["N1", "N2", "N3"]);
    }

    /// A card Apple no longer has is already deleted: nothing is sent and nothing fails.
    #[tokio::test]
    async fn deleting_a_card_already_gone_succeeds_quietly() {
        let server = server().await;
        Mock::given(path_regex(r".*/co/startup$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "contacts": [] })))
            .mount(&server)
            .await;
        delete(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
        )
        .await
        .unwrap();
        assert!(!server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/co/contacts/card")));
    }

    /// A number alone is a contact, as on the phone.
    #[tokio::test]
    async fn a_contact_with_only_a_number_is_created() {
        let server = server().await;
        let number_only = Contact {
            first_name: None,
            last_name: None,
            company: None,
            ..person()
        };
        assert!(
            number_only.methods.iter().any(|m| !m.value.is_empty()),
            "the fixture has a number"
        );
        create(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &number_only,
        )
        .await
        .unwrap();
    }

    /// A stale sync token is recoverable by re-reading, so it is named as a conflict rather
    /// than surfaced as an HTTP status nobody can act on.
    #[tokio::test]
    async fn a_stale_sync_token_reads_as_a_conflict() {
        let server = MockServer::start().await;
        Mock::given(path_regex(r".*/co/contacts/card$"))
            .and(method("PUT"))
            .respond_with(ResponseTemplate::new(412))
            .mount(&server)
            .await;
        let err = update(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            &tokens(),
            &person(),
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::Conflict), "got {err:?}");
    }
}

#[cfg(test)]
mod shape_tests {
    use super::*;
    use serde_json::json;

    /// A postal address nests its parts rather than holding a string, so reading it the way
    /// every other method array is read skipped it entirely — the one contact with an
    /// address showed none.
    #[test]
    fn a_postal_address_reads_as_the_lines_it_would_be_written_on() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "firstName": "Asha",
                "streetAddresses": [{
                    "label": "HOME",
                    "field": {
                        "street": "12 Example Street\nSpringfield",
                        "city": "Springfield",
                        "state": "ST",
                        "postalCode": "123456",
                        "country": "India"
                    }
                }]
            }]
        }));
        let address = contacts[0]
            .methods
            .iter()
            .find(|m| m.kind == "address")
            .expect("the address is read");
        assert_eq!(
            address.value,
            // India's order: town and PIN together, the state on its own line.
            "12 Example Street\nSpringfield\nSpringfield 123456\nST\nIndia"
        );
        assert_eq!(address.label.as_deref(), Some("HOME"));
    }

    /// Town, county and postcode belong on one line; a missing part must not leave a gap.
    #[test]
    fn a_partial_address_leaves_no_holes() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "streetAddresses": [{ "field": { "city": "Springfield", "country": "India" } }]
            }]
        }));
        assert_eq!(contacts[0].methods[0].value, "Springfield\nIndia");
    }

    /// A profile carries an encoded handle and the readable one a person typed. Showing
    /// `x-apple:%2B91...` answers nothing.
    #[test]
    fn a_profile_shows_what_a_person_typed_not_the_encoding() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "profiles": [{
                    "field": "x-apple:+91%2090000%2000003",
                    "user": "+91 90000 00003"
                }]
            }]
        }));
        assert_eq!(contacts[0].methods[0].kind, "profile");
        assert_eq!(contacts[0].methods[0].value, "+91 90000 00003");
    }

    /// Apple keeps a middle name, and a person with one is not shown under a name that is
    /// only nearly theirs.
    #[test]
    fn a_middle_name_is_part_of_the_name() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "firstName": "Asha",
                "middleName": "Devi",
                "lastName": "Rao"
            }]
        }));
        assert_eq!(contacts[0].display_name(), "Asha Devi Rao");
    }

    /// An address that really is a plain string still reads.
    #[test]
    fn an_address_given_as_a_string_still_reads() {
        let contacts = parse(&json!({
            "contacts": [{
                "contactId": "C1",
                "streetAddresses": [{ "field": "12 Somewhere Lane" }]
            }]
        }));
        assert_eq!(contacts[0].methods[0].value, "12 Somewhere Lane");
    }

    #[test]
    fn an_empty_address_is_left_out_rather_than_shown_blank() {
        let contacts = parse(&json!({
            "contacts": [{ "contactId": "C1", "streetAddresses": [{ "field": {} }] }]
        }));
        assert!(contacts[0].methods.is_empty());
    }
}

#[cfg(test)]
mod photo_tests {
    use super::*;

    /// Apple's own service sends these; four of seventeen pictures on a real account arrive
    /// with a vCard property welded onto the card id, and that URL answers 404.
    #[test]
    fn a_url_apple_mangled_is_trimmed_back_to_the_card_it_names() {
        let base = "https://gateway.icloud.com/contacts/1148/ck/card";
        assert_eq!(
            photo_url(&format!(
                "{base}/095356b02d69e2fbebb51c11fee2459dX-SHARED-PHOTO-DISPLAY-PREF:ALWAYS_ASK"
            )),
            format!("{base}/095356b02d69e2fbebb51c11fee2459d")
        );
        assert_eq!(
            photo_url(&format!(
                "{base}/396d3dd0fc24ceeb66305190d44840edX-SHARED-PHOTO-DISPLAY-PREF:AUTOUPDATE"
            )),
            format!("{base}/396d3dd0fc24ceeb66305190d44840ed")
        );
    }

    /// A URL that is already right must come back untouched, whatever shape it is.
    #[test]
    fn a_sound_url_is_left_exactly_as_it_came() {
        for url in [
            "https://gateway.icloud.com/contacts/1148/ck/card/5dcf9896c11839d3119214c8a000fe0f",
            "https://example.invalid/ada.jpg",
            "https://example.invalid/deadbeef",
            "no-slash-at-all",
        ] {
            assert_eq!(photo_url(url), url, "{url}");
        }
    }

    #[test]
    fn a_crop_is_flattened_and_a_useless_one_is_dropped() {
        let value = serde_json::json!({"x": 251, "y": 1249, "width": 1279, "height": 1279});
        assert_eq!(crop(&value).as_deref(), Some("251,1249,1279,1279"));
        assert_eq!(
            crop(&serde_json::json!({"x": 0, "y": 0, "width": 0, "height": 9})),
            None
        );
        assert_eq!(crop(&serde_json::json!({"x": 0, "y": 0})), None);
        assert_eq!(crop(&Value::Null), None);
    }

    /// The picture fields come off a real payload, not an invented one.
    #[test]
    fn a_shared_picture_parses_with_a_usable_url_and_nothing_else() {
        let body = serde_json::json!({"contacts": [{
            "contactId": "abc",
            "firstName": "Atta",
            "photo": { "url":
                "https://gateway.icloud.com/contacts/1148/ck/card/095356b02d69e2fbebb51c11fee2459dX-SHARED-PHOTO-DISPLAY-PREF:ALWAYS_ASK" }
        }]});
        let parsed = parse(&body);
        assert_eq!(
            parsed[0].photo_url.as_deref(),
            Some(
                "https://gateway.icloud.com/contacts/1148/ck/card/095356b02d69e2fbebb51c11fee2459d"
            )
        );
        assert_eq!(parsed[0].photo_signature, None);
        assert_eq!(parsed[0].photo_crop, None);
    }
}

#[cfg(test)]
mod odd_card_tests {
    use super::*;

    /// Apple's record is the starting point for a write, and nothing obliges it to be an
    /// object. A panic here would take the whole window down on a single odd reply.
    #[test]
    fn a_card_that_is_not_an_object_is_set_aside_rather_than_panicking() {
        let contact = Contact {
            guid: "C1".into(),
            first_name: Some("Ada".into()),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: Some("Lovelace".into()),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: vec![],
        };
        for odd in [
            Value::Null,
            serde_json::json!([1, 2]),
            serde_json::json!("text"),
            serde_json::json!(7),
        ] {
            let card = card_merged(&contact, Some(&odd), None);
            assert_eq!(card["firstName"], "Ada", "{odd} still produces the card");
        }
    }
}

#[cfg(test)]
mod atomic_apply_tests {
    use super::*;
    use quince_core::contacts::Contact;

    fn person(guid: &str) -> Contact {
        Contact {
            guid: guid.into(),
            first_name: Some(guid.into()),
            middle_name: None,
            last_name: None,
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            etag: None,
            methods: vec![],
        }
    }

    /// The book is stored (pruning whoever is not in it) and then its lists: a failure storing
    /// the lists — a crash part way — must not leave the people pruned and the rest unwritten.
    #[test]
    fn a_book_that_fails_part_way_stores_none_of_itself() {
        let mut library = quince_core::Library::in_memory().unwrap();
        library.put_contacts(&[person("A")]).unwrap();
        library
            .conn()
            .execute_batch(
                "CREATE TEMP TRIGGER crash BEFORE INSERT ON main.app_state
                   BEGIN SELECT RAISE(ABORT, 'crash mid-apply'); END;",
            )
            .unwrap();
        let failed = apply(
            &mut library,
            AddressBook {
                contacts: vec![person("B")],
                lists: vec![],
                me: None,
                addresses: vec![],
                details: vec![],
            },
        );
        assert!(failed.is_err());
        let guids: Vec<String> = library
            .contacts(None)
            .unwrap()
            .into_iter()
            .map(|c| c.guid)
            .collect();
        assert_eq!(guids, vec!["A".to_string()], "the old book, whole");
    }
}

#[cfg(test)]
mod field_tests {
    use super::*;
    use serde_json::json;

    fn apple() -> Value {
        json!({
            "contactId": "C1", "etag": "e1", "prefix": "Dr.", "firstName": "Asha",
            "middleName": "K", "lastName": "Rao", "suffix": "Jr.", "nickName": "Ash",
            "phoneticFirstName": "AA-sha", "phoneticLastName": "Rao",
            "phoneticCompanyName": "nor-th-field", "companyName": "Northfield",
            "jobTitle": "Architect", "department": "Design",
            "birthday": "1604-03-17",
            "phones": [{ "label": "MOBILE", "field": "+91 1" }, { "label": "Grandma's", "field": "+91 2" }],
            "emailAddresses": [{ "label": "_$!<Work>!$_", "field": "a@x.in" }],
            "urls": [{ "label": "HOMEPAGE", "field": "asha.in" }],
            "dates": [{ "label": "ANNIVERSARY", "field": "2015-02-14" }, { "field": "1604-08-01", "label": "first day" }],
            "relatedNames": [{ "label": "MOTHER", "field": "Amma" }],
            "profiles": [{ "label": "TWITTER", "user": "asha", "field": "https://twitter.com/asha", "userId": "42" }],
            "IMs": [{ "label": "HOME", "field": { "userName": "asha.k", "IMService": "SKYPE" } }],
            "photo": { "url": "https://p/C1", "signature": "s1", "crop": { "x": 1, "y": 2, "width": 30, "height": 30 } },
            "streetAddresses": [{ "label": "HOME", "field": { "city": "Pune" } }],
        })
    }

    fn read() -> Card {
        parse_cards(&json!({ "contacts": [apple()] })).remove(0)
    }

    #[test]
    fn every_field_apple_keeps_is_read() {
        let c = read();
        let d = &c.details;
        assert_eq!(d.prefix.as_deref(), Some("Dr."));
        assert_eq!(d.suffix.as_deref(), Some("Jr."));
        assert_eq!(d.nickname.as_deref(), Some("Ash"));
        assert_eq!(d.phonetic_first_name.as_deref(), Some("AA-sha"));
        assert_eq!(d.phonetic_company.as_deref(), Some("nor-th-field"));
        assert_eq!(d.job_title.as_deref(), Some("Architect"));
        assert_eq!(d.department.as_deref(), Some("Design"));
        assert_eq!(c.contact.middle_name.as_deref(), Some("K"));
        assert_eq!(c.contact.birthday.as_deref(), Some("1604-03-17"));
        let of = |kind: &str| -> Vec<(Option<String>, String)> {
            c.contact
                .methods
                .iter()
                .filter(|m| m.kind == kind)
                .map(|m| (m.label.clone(), m.value.clone()))
                .collect()
        };
        assert_eq!(of("date").len(), 2);
        assert_eq!(of("related"), [(Some("MOTHER".into()), "Amma".into())]);
        assert_eq!(of("im"), [(Some("SKYPE".into()), "asha.k".into())]);
        assert_eq!(of("profile"), [(Some("TWITTER".into()), "asha".into())]);
        assert_eq!(
            of("phone")[1].0.as_deref(),
            Some("Grandma's"),
            "a custom label as typed"
        );
    }

    /// Opened and saved with nothing changed, the card goes back exactly as Apple sent it.
    #[test]
    fn a_card_saved_unchanged_goes_back_byte_for_byte() {
        let c = read();
        let written = card_written(
            &c.contact,
            Some(&c.details),
            Some(&apple()),
            Some(&c.contact),
            Some(&c.details),
        );
        let mut want = apple();
        want["isCompany"] = json!(false);
        assert_eq!(written, want);
    }

    #[test]
    fn each_new_field_is_written_as_apple_keeps_it() {
        let base = read();
        let mut c = base.clone();
        c.details.nickname = None;
        c.details.job_title = Some("Principal Architect".into());
        c.contact.middle_name = None;
        // An instant message edited: its outer label stays, as icloud.com keeps it.
        let im = c
            .contact
            .methods
            .iter_mut()
            .find(|m| m.kind == "im")
            .unwrap();
        im.value = "asha.rao".into();
        let profile = c
            .contact
            .methods
            .iter_mut()
            .find(|m| m.kind == "profile")
            .unwrap();
        profile.value = "asharao".into();
        c.contact.methods.push(ContactMethod {
            kind: "related".into(),
            label: Some("SPOUSE".into()),
            value: "Ravi".into(),
        });
        c.contact.methods.push(ContactMethod {
            kind: "date".into(),
            label: Some("ANNIVERSARY".into()),
            value: "1604-11-02".into(),
        });
        let w = card_written(
            &c.contact,
            Some(&c.details),
            Some(&apple()),
            Some(&base.contact),
            Some(&base.details),
        );
        assert_eq!(w["nickName"], "", "a cleared field is sent empty");
        assert_eq!(w["jobTitle"], "Principal Architect");
        assert_eq!(w["prefix"], "Dr.", "untouched");
        assert_eq!(w["middleName"], "");
        assert_eq!(
            w["IMs"][0],
            json!({ "label": "HOME", "field": { "userName": "asha.rao", "IMService": "SKYPE" } })
        );
        assert_eq!(w["profiles"][0]["user"], "asharao");
        assert_eq!(w["profiles"][0]["label"], "TWITTER");
        assert_eq!(
            w["relatedNames"][1],
            json!({ "label": "SPOUSE", "field": "Ravi" })
        );
        assert_eq!(
            w["dates"][2],
            json!({ "label": "ANNIVERSARY", "field": "1604-11-02" })
        );
        assert_eq!(
            w["emailAddresses"][0]["label"], "_$!<Work>!$_",
            "Apple's own spelling kept"
        );
    }

    #[test]
    fn a_new_card_carries_only_what_holds_something() {
        let mut c = read();
        c.contact.guid = "NEW".into();
        c.contact.photo_url = None;
        c.details.department = None;
        let w = card_written(&c.contact, Some(&c.details), None, None, None);
        assert_eq!(w["nickName"], "Ash");
        assert!(w.get("department").is_none());
        assert!(w.get("photo").is_none());
        assert_eq!(w["IMs"][0]["field"]["IMService"], "SKYPE");
    }

    #[test]
    fn a_picture_is_sent_whole_cropped_again_or_taken_away() {
        let base = read();
        let mut c = base.clone();
        c.contact.photo_url = Some("data:image/jpeg;base64,AAAA".into());
        c.contact.photo_crop = Some("0,10,200,200".into());
        let w = card_written(&c.contact, None, Some(&apple()), Some(&base.contact), None);
        assert_eq!(
            w["photo"],
            json!({ "url": "data:image/jpeg;base64,AAAA", "crop": { "x": 0, "y": 10, "width": 200, "height": 200 } })
        );

        let mut c = base.clone();
        c.contact.photo_crop = Some("5,5,20,20".into());
        let w = card_written(&c.contact, None, Some(&apple()), Some(&base.contact), None);
        assert_eq!(w["photo"]["url"], "https://p/C1", "the same picture");
        assert_eq!(w["photo"]["signature"], "s1");
        assert_eq!(w["photo"]["crop"]["width"], 20);

        let mut c = base.clone();
        c.contact.photo_url = None;
        let w = card_written(&c.contact, None, Some(&apple()), Some(&base.contact), None);
        assert_eq!(w["photo"], Value::Null);

        let mut c = base;
        c.contact.guid = "NEW".into();
        c.contact.photo_url = Some("data:image/jpeg;base64,BBBB".into());
        c.contact.photo_crop = None;
        let w = card_written(&c.contact, None, None, None, None);
        assert_eq!(w["photo"], json!({ "url": "data:image/jpeg;base64,BBBB" }));
    }

    #[test]
    fn a_whole_card_round_trips_through_json_as_the_interface_sends_it() {
        let c = read();
        let text = serde_json::to_string(&c).unwrap();
        assert!(text.contains("\"jobTitle\":\"Architect\""));
        assert!(text.contains("\"firstName\":\"Asha\""));
        let back: Card = serde_json::from_str(&text).unwrap();
        assert_eq!(back, c);
        // A card from an older window, with none of the new fields, still reads.
        let old: Card = serde_json::from_value(json!({
            "guid": "X", "firstName": "A", "lastName": null, "company": null, "isCompany": false,
            "birthday": null, "notes": null, "etag": null, "methods": []
        }))
        .unwrap();
        assert!(old.details.is_empty());
    }
}
