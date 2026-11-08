//! Contacts in the local library.

use crate::{Library, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// Apple's own names for the labels on a card's numbers, addresses, dates and people.
#[path = "contacts_labels.rs"]
pub mod labels;

/// Cards that are one person twice, and putting them back together.
#[path = "contacts_merge.rs"]
pub mod merge;

/// The rest of a person's name and work, as Apple's card holds them. Kept beside [`Contact`]
/// rather than in it, so code that builds a `Contact` by hand is not made to name them.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContactDetails {
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub phonetic_first_name: Option<String>,
    #[serde(default)]
    pub phonetic_last_name: Option<String>,
    #[serde(default)]
    pub phonetic_company: Option<String>,
    #[serde(default)]
    pub job_title: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
}

/// Each detail by the key Apple's card names it with (`/co/` contact JSON).
pub const DETAIL_KEYS: [&str; 8] = [
    "prefix",
    "suffix",
    "nickName",
    "phoneticFirstName",
    "phoneticLastName",
    "phoneticCompanyName",
    "jobTitle",
    "department",
];

impl ContactDetails {
    /// One detail by Apple's key for it; `None` for a key that is not one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Option<String>> {
        Some(match key {
            "prefix" => &self.prefix,
            "suffix" => &self.suffix,
            "nickName" => &self.nickname,
            "phoneticFirstName" => &self.phonetic_first_name,
            "phoneticLastName" => &self.phonetic_last_name,
            "phoneticCompanyName" => &self.phonetic_company,
            "jobTitle" => &self.job_title,
            "department" => &self.department,
            _ => return None,
        })
    }

    /// Set one detail by Apple's key, blank meaning cleared. False for a key that is not one.
    pub fn set(&mut self, key: &str, value: Option<&str>) -> bool {
        let slot = match key {
            "prefix" => &mut self.prefix,
            "suffix" => &mut self.suffix,
            "nickName" => &mut self.nickname,
            "phoneticFirstName" => &mut self.phonetic_first_name,
            "phoneticLastName" => &mut self.phonetic_last_name,
            "phoneticCompanyName" => &mut self.phonetic_company,
            "jobTitle" => &mut self.job_title,
            "department" => &mut self.department,
            _ => return false,
        };
        *slot = clean(value);
        true
    }

    /// Nothing in any of them.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        DETAIL_KEYS
            .iter()
            .all(|k| self.get(k).is_none_or(Option::is_none))
    }
}

/// A whole card as the interface reads and writes it: the person and the rest of their name
/// and work, side by side in one object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Card {
    #[serde(flatten)]
    pub contact: Contact,
    #[serde(flatten)]
    pub details: ContactDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContactMethod {
    /// `phone`, `email`, `url`, `address`.
    pub kind: String,
    pub label: Option<String>,
    pub value: String,
}

/// A postal address as Apple keeps it: each part on its own, so one can be changed without
/// retyping the rest. Field names match Apple's `streetAddresses[].field` keys when
/// serialised, which is also what an edit names a field by.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PostalAddress {
    pub label: Option<String>,
    /// Every line before the town, newlines and all.
    pub street: Option<String>,
    pub sub_locality: Option<String>,
    pub city: Option<String>,
    pub sub_administrative_area: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    /// Apple's two-letter code, lower case on the cards it writes.
    pub country_code: Option<String>,
}

/// The parts of an address, by the names Apple's service uses for them.
pub const ADDRESS_FIELDS: [&str; 8] = [
    "street",
    "subLocality",
    "city",
    "subAdministrativeArea",
    "state",
    "postalCode",
    "country",
    "countryCode",
];

impl PostalAddress {
    /// One part by Apple's name for it; `None` for a name that is not a part.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&Option<String>> {
        Some(match name {
            "street" => &self.street,
            "subLocality" => &self.sub_locality,
            "city" => &self.city,
            "subAdministrativeArea" => &self.sub_administrative_area,
            "state" => &self.state,
            "postalCode" => &self.postal_code,
            "country" => &self.country,
            "countryCode" => &self.country_code,
            _ => return None,
        })
    }

    /// Set one part by Apple's name for it, blank meaning cleared. False for a name that is
    /// not a part.
    pub fn set_field(&mut self, name: &str, value: Option<&str>) -> bool {
        let slot = match name {
            "street" => &mut self.street,
            "subLocality" => &mut self.sub_locality,
            "city" => &mut self.city,
            "subAdministrativeArea" => &mut self.sub_administrative_area,
            "state" => &mut self.state,
            "postalCode" => &mut self.postal_code,
            "country" => &mut self.country,
            "countryCode" => &mut self.country_code,
            _ => return false,
        };
        *slot = clean(value);
        true
    }

    /// No part holds anything (the label alone is not an address).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        ADDRESS_FIELDS
            .iter()
            .all(|f| self.field(f).is_none_or(Option::is_none))
    }

    /// The lines a person would write on an envelope, in the order the address's country
    /// writes them: a postcode before the town in Germany and France, the town and postcode
    /// on lines of their own in the UK, largest to smallest in Japan.
    #[must_use]
    pub fn formatted(&self) -> String {
        let template = match self.country_key() {
            Some("us") => US,
            Some("gb") => GB,
            Some("in") => IN,
            Some("de" | "fr") => DE_FR,
            Some("jp") => JP,
            _ => DEFAULT,
        };
        let mut lines: Vec<String> = Vec::new();
        for line in template {
            let mut text = String::new();
            for (name, separator) in *line {
                let Some(part) = self.field(name).and_then(Option::as_deref) else {
                    continue;
                };
                if !text.is_empty() {
                    text.push_str(separator);
                }
                text.push_str(part);
            }
            // A street keeps its own line breaks, each trimmed.
            lines.extend(
                text.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(ToString::to_string),
            );
        }
        lines.join("\n")
    }

    /// Which country's layout applies: Apple's code when the card has one, else the name.
    #[must_use]
    pub fn country_key(&self) -> Option<&'static str> {
        self.country_code
            .as_deref()
            .and_then(country_from_code)
            .or_else(|| self.country.as_deref().and_then(country_code_for))
    }
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

/// One line of an address layout: each part with what goes before it when a part already
/// sits on the line. A missing part leaves no gap.
type Line = &'static [(&'static str, &'static str)];

const US: &[Line] = &[
    &[("street", "")],
    &[("subLocality", "")],
    &[("city", ""), ("state", ", "), ("postalCode", " ")],
    &[("country", "")],
];
const GB: &[Line] = &[
    &[("street", "")],
    &[("subLocality", "")],
    &[("city", "")],
    &[("subAdministrativeArea", "")],
    &[("postalCode", "")],
    &[("country", "")],
];
const IN: &[Line] = &[
    &[("street", "")],
    &[("subLocality", "")],
    &[("city", ""), ("postalCode", " ")],
    &[("state", "")],
    &[("country", "")],
];
const DE_FR: &[Line] = &[
    &[("street", "")],
    &[("postalCode", ""), ("city", " ")],
    &[("country", "")],
];
const JP: &[Line] = &[
    &[("postalCode", "")],
    &[("state", ""), ("city", " "), ("subLocality", " ")],
    &[("street", "")],
    &[("country", "")],
];
const DEFAULT: &[Line] = &[
    &[("street", "")],
    &[("subLocality", "")],
    &[("city", ""), ("state", " "), ("postalCode", " ")],
    &[("country", "")],
];

/// The countries with a layout of their own, by Apple's code.
fn country_from_code(code: &str) -> Option<&'static str> {
    match code.trim().to_ascii_lowercase().as_str() {
        "us" => Some("us"),
        "gb" | "uk" => Some("gb"),
        "in" => Some("in"),
        "de" => Some("de"),
        "fr" => Some("fr"),
        "jp" => Some("jp"),
        _ => None,
    }
}

/// Apple's two-letter code for a country typed by name, for the countries this app lays out
/// by their own rules. `None` for any other.
#[must_use]
pub fn country_code_for(name: &str) -> Option<&'static str> {
    let name = crate::fold::fold(name.trim());
    let name = name.trim_end_matches('.');
    Some(match name {
        "united states"
        | "united states of america"
        | "usa"
        | "us"
        | "u.s.a"
        | "u.s"
        | "america" => "us",
        "united kingdom" | "uk" | "u.k" | "great britain" | "britain" | "england" | "scotland"
        | "wales" | "northern ireland" | "gb" => "gb",
        "india" | "bharat" | "in" => "in",
        "germany" | "deutschland" | "de" => "de",
        "france" | "fr" => "fr",
        "japan" | "nippon" | "日本" | "jp" => "jp",
        _ => return None,
    })
}

/// Everything needed to put one person's picture on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactPhoto {
    pub url: String,
    /// Changes when the picture does, so a cached copy knows it has gone stale. Absent on a
    /// picture shared by the person it belongs to, which is then never cached to disk.
    pub signature: Option<String>,
    /// `x,y,width,height` in the stored picture's own pixels.
    pub crop: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub guid: String,
    pub first_name: Option<String>,
    /// Apple keeps one, and four people in a real address book have one.
    #[serde(default)]
    pub middle_name: Option<String>,
    /// Where Apple keeps this person's picture, and the signature that changes when the
    /// picture does. `None` for the great majority of an address book.
    #[serde(default)]
    pub photo_url: Option<String>,
    #[serde(default)]
    pub photo_signature: Option<String>,
    /// The square its owner chose, as `x,y,width,height` in the stored picture's own pixels.
    #[serde(default)]
    pub photo_crop: Option<String>,
    pub last_name: Option<String>,
    pub company: Option<String>,
    pub is_company: bool,
    pub birthday: Option<String>,
    pub notes: Option<String>,
    pub etag: Option<String>,
    #[serde(default)]
    pub methods: Vec<ContactMethod>,
}

impl Contact {
    /// What to show, whichever name fields this contact happens to have.
    ///
    /// A contact with only a phone number still needs a label, so the fallback chain ends at
    /// the first method rather than at an empty string.
    #[must_use]
    pub fn display_name(&self) -> String {
        // A company's card goes by the company, whoever is named on it, as Contacts shows it.
        if let Some(company) = self
            .company
            .as_deref()
            .map(str::trim)
            .filter(|c| self.is_company && !c.is_empty())
        {
            return company.to_string();
        }
        let full = [
            self.first_name.as_deref(),
            self.middle_name.as_deref(),
            self.last_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        if !full.trim().is_empty() {
            return full;
        }
        if let Some(company) = self.company.as_deref().filter(|c| !c.trim().is_empty()) {
            return company.to_string();
        }
        self.methods
            .first()
            .map_or_else(|| "No Name".to_string(), |m| m.value.clone())
    }

    /// Sorted on last name where there is one, which is how Apple orders the list.
    pub(crate) fn sort_key(&self) -> String {
        let key = [self.last_name.as_deref(), self.first_name.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let key = if key.trim().is_empty() {
            self.display_name()
        } else {
            key
        };
        crate::fold::fold(&key)
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Library {
    /// Replace the address book, dropping anyone Apple no longer reports.
    pub fn put_contacts(&mut self, contacts: &[Contact]) -> Result<()> {
        self.store_contacts(contacts, true)
    }

    /// Store one card as it now stands — made or edited here, before Apple has it — leaving
    /// everyone else as they are.
    pub fn put_contact(&mut self, contact: &Contact) -> Result<()> {
        self.store_contacts(std::slice::from_ref(contact), false)
    }

    /// One card gone — deleted here, before Apple has it — with its numbers and addresses.
    pub fn forget_contact(&mut self, guid: &str) -> Result<()> {
        let tx = self.conn.savepoint()?;
        tx.execute(
            "DELETE FROM contact_methods WHERE contact_guid = ?1",
            params![guid],
        )?;
        tx.execute(
            "DELETE FROM contact_addresses WHERE contact_guid = ?1",
            params![guid],
        )?;
        tx.execute("DELETE FROM contacts WHERE guid = ?1", params![guid])?;
        tx.commit()?;
        Ok(())
    }

    /// Store the rest of each card's name and work, as a read of the book found them. A whole
    /// book clears every card's first, so a nickname taken off on the phone goes here too.
    pub fn put_contact_details(
        &mut self,
        book: &[(String, ContactDetails)],
        whole_book: bool,
    ) -> Result<()> {
        let tx = self.conn.savepoint()?;
        if whole_book {
            tx.execute(
                "UPDATE contacts SET prefix = NULL, suffix = NULL, nickname = NULL,
                    phonetic_first_name = NULL, phonetic_last_name = NULL,
                    phonetic_company = NULL, job_title = NULL, department = NULL",
                [],
            )?;
        }
        {
            let mut set = tx.prepare(
                "UPDATE contacts SET prefix = ?2, suffix = ?3, nickname = ?4,
                    phonetic_first_name = ?5, phonetic_last_name = ?6, phonetic_company = ?7,
                    job_title = ?8, department = ?9
                 WHERE guid = ?1",
            )?;
            for (guid, d) in book {
                set.execute(params![
                    guid,
                    d.prefix,
                    d.suffix,
                    d.nickname,
                    d.phonetic_first_name,
                    d.phonetic_last_name,
                    d.phonetic_company,
                    d.job_title,
                    d.department
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// One card, whole, as it now stands — made or edited here — leaving everyone else as
    /// they are.
    pub fn put_card(&mut self, card: &Card) -> Result<()> {
        self.atomically(|l| {
            l.put_contact(&card.contact)?;
            l.put_contact_details(&[(card.contact.guid.clone(), card.details.clone())], false)
        })
    }

    /// The rest of one card's name and work.
    pub fn contact_details(&self, guid: &str) -> Result<ContactDetails> {
        Ok(self
            .details_by_guid(Some(guid))?
            .remove(guid)
            .unwrap_or_default())
    }

    fn details_by_guid(
        &self,
        guid: Option<&str>,
    ) -> Result<std::collections::HashMap<String, ContactDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, prefix, suffix, nickname, phonetic_first_name, phonetic_last_name,
                    phonetic_company, job_title, department
             FROM contacts WHERE ?1 IS NULL OR guid = ?1",
        )?;
        let rows = stmt.query_map(params![guid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ContactDetails {
                    prefix: row.get(1)?,
                    suffix: row.get(2)?,
                    nickname: row.get(3)?,
                    phonetic_first_name: row.get(4)?,
                    phonetic_last_name: row.get(5)?,
                    phonetic_company: row.get(6)?,
                    job_title: row.get(7)?,
                    department: row.get(8)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn store_contacts(&mut self, contacts: &[Contact], whole_book: bool) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut upsert = tx.prepare(
                "INSERT INTO contacts (guid, first_name, middle_name, last_name, company, is_company,
                                       sort_name, birthday, notes, photo_url, photo_signature,
                                       photo_crop, etag, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(guid) DO UPDATE SET
                   first_name = excluded.first_name, middle_name = excluded.middle_name,
                   last_name = excluded.last_name,
                   photo_url = excluded.photo_url,
                   photo_signature = excluded.photo_signature,
                   photo_crop = excluded.photo_crop,
                   company = excluded.company, is_company = excluded.is_company,
                   sort_name = excluded.sort_name, birthday = excluded.birthday,
                   notes = excluded.notes, etag = excluded.etag,
                   updated_at = excluded.updated_at",
            )?;
            let mut clear = tx.prepare("DELETE FROM contact_methods WHERE contact_guid = ?1")?;
            let mut add = tx.prepare(
                "INSERT INTO contact_methods (contact_guid, kind, label, value, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            let stamp = now();
            for c in contacts {
                upsert.execute(params![
                    c.guid,
                    c.first_name,
                    c.middle_name,
                    c.last_name,
                    c.company,
                    i64::from(c.is_company),
                    c.sort_key(),
                    c.birthday,
                    c.notes,
                    c.photo_url,
                    c.photo_signature,
                    c.photo_crop,
                    c.etag,
                    stamp
                ])?;
                // Methods are rewritten wholesale: a number removed on the phone must not
                // linger here.
                clear.execute(params![c.guid])?;
                for (i, m) in c.methods.iter().enumerate() {
                    add.execute(params![
                        c.guid,
                        m.kind,
                        m.label,
                        m.value,
                        i64::try_from(i).unwrap_or(0)
                    ])?;
                }
            }

            let keep: Vec<&str> = contacts.iter().map(|c| c.guid.as_str()).collect();
            if !whole_book {
                // One card: no one else is touched.
            } else if keep.is_empty() {
                tx.execute("DELETE FROM contacts", [])?;
            } else {
                tx.execute(
                    "DELETE FROM contacts WHERE guid NOT IN (SELECT value FROM json_each(?1))",
                    params![crate::id_list(&keep)],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Store the structured addresses a read of the address book found, by card. A whole
    /// book replaces every stored address; otherwise only the cards named are touched.
    ///
    /// Every card named must already be stored (its addresses hang off it), which holds when
    /// they come from the same read as [`Library::put_contacts`].
    pub fn put_addresses(
        &mut self,
        book: &[(String, Vec<PostalAddress>)],
        whole_book: bool,
    ) -> Result<()> {
        let tx = self.conn.savepoint()?;
        if whole_book {
            tx.execute("DELETE FROM contact_addresses", [])?;
        }
        {
            let mut clear = tx.prepare("DELETE FROM contact_addresses WHERE contact_guid = ?1")?;
            let mut add = tx.prepare(ADD_ADDRESS)?;
            for (guid, addresses) in book {
                clear.execute(params![guid])?;
                insert_addresses(&mut add, guid, addresses)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// One card's addresses as just edited here: the structured parts, and the flattened
    /// lines kept with its other methods (which search and older views read), in the place
    /// its addresses already held among them.
    pub fn set_contact_addresses(&mut self, guid: &str, addresses: &[PostalAddress]) -> Result<()> {
        let tx = self.conn.savepoint()?;
        {
            let mut methods: Vec<ContactMethod> = {
                let mut stmt = tx.prepare(
                    "SELECT kind, label, value FROM contact_methods
                     WHERE contact_guid = ?1 ORDER BY position",
                )?;
                let rows = stmt.query_map(params![guid], |row| {
                    Ok(ContactMethod {
                        kind: row.get(0)?,
                        label: row.get(1)?,
                        value: row.get(2)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            let at = methods
                .iter()
                .position(|m| m.kind == "address")
                .unwrap_or_else(|| {
                    // Where a read puts them: after phones, emails and links.
                    methods
                        .iter()
                        .rposition(|m| matches!(m.kind.as_str(), "phone" | "email" | "url"))
                        .map_or(0, |i| i + 1)
                });
            methods.retain(|m| m.kind != "address");
            let lines = addresses
                .iter()
                .filter(|a| !a.is_empty())
                .map(|a| ContactMethod {
                    kind: "address".into(),
                    label: a.label.clone(),
                    value: a.formatted(),
                });
            let at = at.min(methods.len());
            methods.splice(at..at, lines);

            tx.execute(
                "DELETE FROM contact_methods WHERE contact_guid = ?1",
                params![guid],
            )?;
            let mut add = tx.prepare(
                "INSERT INTO contact_methods (contact_guid, kind, label, value, position)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (i, m) in methods.iter().enumerate() {
                add.execute(params![
                    guid,
                    m.kind,
                    m.label,
                    m.value,
                    i64::try_from(i).unwrap_or(0)
                ])?;
            }
            tx.execute(
                "DELETE FROM contact_addresses WHERE contact_guid = ?1",
                params![guid],
            )?;
            let mut add = tx.prepare(ADD_ADDRESS)?;
            insert_addresses(&mut add, guid, addresses)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// One card's addresses, part by part, in Apple's order.
    pub fn contact_addresses(&self, guid: &str) -> Result<Vec<PostalAddress>> {
        let mut stmt = self.conn.prepare(
            "SELECT label, street, sub_locality, city, sub_administrative_area, state,
                    postal_code, country, country_code
             FROM contact_addresses WHERE contact_guid = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![guid], |row| {
            Ok(PostalAddress {
                label: row.get(0)?,
                street: row.get(1)?,
                sub_locality: row.get(2)?,
                city: row.get(3)?,
                sub_administrative_area: row.get(4)?,
                state: row.get(5)?,
                postal_code: row.get(6)?,
                country: row.get(7)?,
                country_code: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Where one person's picture lives, without reading the rest of the address book.
    ///
    /// The picture is served per row as a list scrolls, so this stays a single-row lookup
    /// rather than loading everyone and their phone numbers to answer it.
    /// Every contact with a picture at Apple and the signature naming it, `(guid, signature)`:
    /// what warming the picture cache needs to know, without building each card whole.
    pub fn contact_photo_signatures(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, photo_signature FROM contacts
             WHERE photo_url IS NOT NULL AND photo_signature IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn contact_photo_source(&self, guid: &str) -> Result<Option<ContactPhoto>> {
        let mut stmt = self.conn.prepare(
            "SELECT photo_url, photo_signature, photo_crop FROM contacts WHERE guid = ?1",
        )?;
        let mut rows = stmt.query(params![guid])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        // The signature and the crop are each absent on real contacts — a picture shared by
        // the person it belongs to arrives with neither. Only the URL decides whether there
        // is anything to fetch.
        let Some(url) = row.get::<_, Option<String>>(0)? else {
            return Ok(None);
        };
        Ok(Some(ContactPhoto {
            url,
            signature: row.get(1)?,
            crop: row.get(2)?,
        }))
    }

    /// How many contacts there are, counted rather than read: the Skykeep window's launcher
    /// puts the number under Contacts.
    pub fn contact_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM contacts", [], |r| r.get(0))?)
    }

    /// The address book in display order, optionally filtered.
    ///
    /// The filter matches names and any stored method, so searching a phone number finds the
    /// person it belongs to. Every word must match, in any order, without regard to case or
    /// accents, and a number matches however it was typed: "98765 43210" finds
    /// "+91 98765-43210". That is done here rather than in SQL, which folds ASCII alone and
    /// knows nothing about phone formatting.
    pub fn contacts(&self, search: Option<&str>) -> Result<Vec<Contact>> {
        Ok(self.cards(search)?.into_iter().map(|c| c.contact).collect())
    }

    /// Everyone with a birthday, in the order [`Library::contacts`] gives them, with only their
    /// names and birthday filled in: what the Birthdays calendar draws. Calendar built every card
    /// whole for this — each method, detail and address — on every read of a month.
    pub fn contacts_with_birthdays(&self) -> Result<Vec<Contact>> {
        let mut stmt = self.conn.prepare(
            "SELECT guid, first_name, middle_name, last_name, company, is_company, birthday
             FROM contacts WHERE birthday IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Contact {
                guid: row.get(0)?,
                first_name: row.get(1)?,
                middle_name: row.get(2)?,
                photo_url: None,
                photo_signature: None,
                photo_crop: None,
                last_name: row.get(3)?,
                company: row.get(4)?,
                is_company: row.get::<_, i64>(5)? != 0,
                birthday: row.get(6)?,
                notes: None,
                etag: None,
                methods: Vec::new(),
            })
        })?;
        let mut keyed: Vec<(String, Contact)> = rows
            .map(|c| c.map(|c| (c.sort_key(), c)))
            .collect::<rusqlite::Result<_>>()?;
        keyed.sort_by(|(a, _), (b, _)| crate::fold::natural(a, b));
        Ok(keyed.into_iter().map(|(_, c)| c).collect())
    }

    /// Whether anyone in the address book has a birthday: whether there is a Birthdays calendar.
    pub fn any_birthday(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM contacts WHERE birthday IS NOT NULL)",
            [],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    /// [`Library::contacts`], each card whole: with the rest of its name and work, which a
    /// search matches too — a nickname or a job title finds the person.
    pub fn cards(&self, search: Option<&str>) -> Result<Vec<Card>> {
        self.cards_among(search, None)
    }

    /// The cards with these ids, whole, in the same order — those still here. What a screen
    /// reads when a pass names the cards it changed, rather than the whole book again.
    pub fn cards_by_guid(&self, guids: &[String]) -> Result<Vec<Card>> {
        self.cards_among(None, Some(guids))
    }

    fn cards_among(&self, search: Option<&str>, only: Option<&[String]>) -> Result<Vec<Card>> {
        let words = crate::fold::words(search.unwrap_or(""));
        // The ids as one JSON array, matched in SQL: no statement built per count of them.
        let only = only.map(|ids| serde_json::to_string(ids).unwrap_or_else(|_| "[]".into()));

        let mut stmt = self.conn.prepare(
            "SELECT c.guid, c.first_name, c.middle_name, c.last_name, c.company, c.is_company,
                    c.birthday, c.notes, c.photo_url, c.photo_signature, c.photo_crop, c.etag
             FROM contacts c
             WHERE ?1 IS NULL OR c.guid IN (SELECT value FROM json_each(?1))",
        )?;
        let rows = stmt.query_map(params![only], |row| {
            Ok(Contact {
                guid: row.get(0)?,
                first_name: row.get(1)?,
                middle_name: row.get(2)?,
                last_name: row.get(3)?,
                company: row.get(4)?,
                is_company: row.get::<_, i64>(5)? != 0,
                birthday: row.get(6)?,
                notes: row.get(7)?,
                photo_url: row.get(8)?,
                photo_signature: row.get(9)?,
                photo_crop: row.get(10)?,
                etag: row.get(11)?,
                methods: Vec::new(),
            })
        })?;
        let mut contacts = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        // One pass over every method rather than a query per person: an address book of a
        // few thousand would otherwise cost a few thousand queries on every keystroke.
        let mut methods: std::collections::HashMap<String, Vec<ContactMethod>> =
            std::collections::HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT contact_guid, kind, label, value FROM contact_methods
             WHERE ?1 IS NULL OR contact_guid IN (SELECT value FROM json_each(?1))
             ORDER BY contact_guid, position",
        )?;
        let rows = stmt.query_map(params![only], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ContactMethod {
                    kind: row.get(1)?,
                    label: row.get(2)?,
                    value: row.get(3)?,
                },
            ))
        })?;
        for row in rows {
            let (guid, method) = row?;
            methods.entry(guid).or_default().push(method);
        }
        for contact in &mut contacts {
            contact.methods = methods.remove(&contact.guid).unwrap_or_default();
        }
        let mut details = if only.is_some() {
            let mut some = std::collections::HashMap::new();
            for c in &contacts {
                some.extend(self.details_by_guid(Some(&c.guid))?);
            }
            some
        } else {
            self.details_by_guid(None)?
        };
        let mut cards: Vec<Card> = contacts
            .into_iter()
            .map(|contact| Card {
                details: details.remove(&contact.guid).unwrap_or_default(),
                contact,
            })
            .collect();

        if !words.is_empty() {
            cards.retain(|c| c.contact.matches(&words, &c.details));
        }
        let mut keyed: Vec<(String, Card)> = cards
            .into_iter()
            .map(|c| (c.contact.sort_key(), c))
            .collect();
        keyed.sort_by(|(a, _), (b, _)| crate::fold::natural(a, b));
        Ok(keyed.into_iter().map(|(_, c)| c).collect())
    }
}

const ADD_ADDRESS: &str = "INSERT INTO contact_addresses
    (contact_guid, position, label, street, sub_locality, city, sub_administrative_area, state,
     postal_code, country, country_code)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

fn insert_addresses(
    add: &mut rusqlite::Statement<'_>,
    guid: &str,
    addresses: &[PostalAddress],
) -> Result<()> {
    for (i, a) in addresses.iter().filter(|a| !a.is_empty()).enumerate() {
        add.execute(params![
            guid,
            i64::try_from(i).unwrap_or(0),
            a.label,
            a.street,
            a.sub_locality,
            a.city,
            a.sub_administrative_area,
            a.state,
            a.postal_code,
            a.country,
            a.country_code
        ])?;
    }
    Ok(())
}

/// Only the digits of a number, so its formatting stops mattering.
fn digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// True for a query word that reads as (part of) a phone number rather than a name.
fn looks_like_number(word: &str) -> bool {
    word.chars().any(|c| c.is_ascii_digit())
        && word
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '(' | ')' | '.' | '/'))
}

impl Contact {
    fn matches(&self, words: &[String], details: &ContactDetails) -> bool {
        let mut text = [
            self.first_name.as_deref(),
            self.middle_name.as_deref(),
            self.last_name.as_deref(),
            self.company.as_deref(),
            details.nickname.as_deref(),
            details.job_title.as_deref(),
            details.department.as_deref(),
            details.phonetic_first_name.as_deref(),
            details.phonetic_last_name.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        // A date is not something a person searches for by its digits.
        for m in self.methods.iter().filter(|m| m.kind != "date") {
            text.push('\n');
            text.push_str(&m.value);
        }
        let text = crate::fold::fold(&text);
        let numbers: Vec<String> = self
            .methods
            .iter()
            .filter(|m| m.kind == "phone")
            .map(|m| digits(&m.value))
            .collect();
        // A whole number typed or pasted (its words together: "+91 98765 43210") is found
        // however the card writes it — with or without the country code or a trunk 0 — by its
        // last digits, as numbers are told apart when merging.
        let typed: String = words
            .iter()
            .filter(|w| looks_like_number(w))
            .map(|w| digits(w))
            .collect();
        let whole_number = typed.len() >= 7
            && numbers.iter().any(|n| {
                let tail = |d: &str| d[d.len().saturating_sub(10)..].to_string();
                n.contains(&typed)
                    || (n.len() >= 7 && {
                        let (a, b) = (tail(n), tail(&typed));
                        a.ends_with(&b) || b.ends_with(&a)
                    })
            });
        words.iter().all(|w| {
            text.contains(w.as_str())
                || (looks_like_number(w) && {
                    let d = digits(w);
                    whole_number || numbers.iter().any(|n| n.contains(&d))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(guid: &str, first: Option<&str>, last: Option<&str>) -> Contact {
        Contact {
            guid: guid.into(),
            first_name: first.map(ToString::to_string),
            middle_name: None,
            photo_url: None,
            photo_signature: None,
            photo_crop: None,
            last_name: last.map(ToString::to_string),
            company: None,
            is_company: false,
            birthday: None,
            notes: None,
            etag: None,
            methods: vec![ContactMethod {
                kind: "phone".into(),
                label: Some("mobile".into()),
                value: format!("+91{guid}"),
            }],
        }
    }

    /// Cards asked for by id come whole — numbers and the rest of the name — and only those
    /// still here, the same as the whole book has them.
    #[test]
    fn cards_by_id_are_the_cards_the_book_has() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("1", Some("Sam"), Some("Rivera")),
            contact("2", Some("Maya"), Some("Chen")),
            contact("3", Some("Jamie"), None),
        ])
        .unwrap();
        lib.put_contact_details(
            &[(
                "2".into(),
                ContactDetails {
                    nickname: Some("May".into()),
                    ..ContactDetails::default()
                },
            )],
            false,
        )
        .unwrap();
        let some = lib
            .cards_by_guid(&["2".into(), "3".into(), "gone".into()])
            .unwrap();
        let whole = lib.cards(None).unwrap();
        assert_eq!(some.len(), 2);
        for card in &some {
            let same = whole.iter().find(|c| c.contact.guid == card.contact.guid);
            assert_eq!(Some(card), same);
        }
        assert_eq!(some[0].details.nickname.as_deref(), Some("May"));
        assert!(lib.cards_by_guid(&[]).unwrap().is_empty());
    }

    /// A company's card goes by the company, whoever is named on it — as Contacts lists it,
    /// and as an export names it.
    #[test]
    fn a_company_card_goes_by_the_company() {
        let mut card = contact("1", Some("Ravi"), Some("Menon"));
        card.company = Some("Bluepine Logistics".into());
        assert_eq!(card.display_name(), "Ravi Menon");
        card.is_company = true;
        assert_eq!(card.display_name(), "Bluepine Logistics");
        card.company = Some("  ".into());
        assert_eq!(card.display_name(), "Ravi Menon");
    }

    /// A number searched as it is pasted from elsewhere — with its country code, or a trunk
    /// 0 — finds the card that holds it written the other way.
    #[test]
    fn a_number_is_found_with_or_without_its_country_code() {
        let mut lib = Library::in_memory().unwrap();
        let mut local = contact("1", Some("Asha"), None);
        local.methods[0].value = "98765 43210".into();
        let mut intl = contact("2", Some("Bela"), None);
        intl.methods[0].value = "+44 7700 900123".into();
        lib.put_contacts(&[local, intl]).unwrap();
        let found = |q: &str| -> Vec<String> {
            lib.contacts(Some(q))
                .unwrap()
                .into_iter()
                .map(|c| c.guid)
                .collect()
        };
        assert_eq!(found("+91 98765 43210"), ["1"]);
        assert_eq!(found("+919876543210"), ["1"]);
        assert_eq!(found("098765 43210"), ["1"]);
        assert_eq!(found("07700 900123"), ["2"]);
        assert_eq!(found("7700"), ["2"]);
        // Part of a number still finds only what holds that part.
        assert_eq!(found("98765"), ["1"]);
        assert!(found("+91 12345 67890").is_empty());
    }

    /// Warming the picture cache asks only who has a picture and which: the same people the
    /// whole cards say have one, with the same signatures.
    #[test]
    fn picture_signatures_are_read_without_building_every_card() {
        let mut lib = Library::in_memory().unwrap();
        let mut alex = contact("1", Some("Alex"), Some("Rivera"));
        alex.photo_url = Some("https://example.invalid/a".into());
        alex.photo_signature = Some("sig-a".into());
        // A picture with no signature is fetched as it is drawn, never warmed.
        let mut bea = contact("2", Some("Bea"), Some("Moreno"));
        bea.photo_url = Some("https://example.invalid/b".into());
        let mut cy = contact("3", Some("Cy"), Some("Lane"));
        cy.photo_signature = Some("stale".into());
        lib.put_contacts(&[alex, bea, cy]).unwrap();
        let mut whole: Vec<(String, String)> = lib
            .contacts(None)
            .unwrap()
            .into_iter()
            .filter(|c| c.photo_url.is_some())
            .filter_map(|c| c.photo_signature.map(|s| (c.guid, s)))
            .collect();
        let mut quick = lib.contact_photo_signatures().unwrap();
        whole.sort();
        quick.sort();
        assert_eq!(quick, whole);
        assert_eq!(quick, [("1".to_string(), "sig-a".to_string())]);
    }

    /// Calendar's read: the people with a birthday, in the address book's order, with what
    /// a birthday's title is made from — and whether there are any at all.
    #[test]
    fn birthdays_are_read_without_building_every_card() {
        let mut lib = Library::in_memory().unwrap();
        assert!(!lib.any_birthday().unwrap());
        let mut alex = contact("1", Some("Alex"), Some("Rivera"));
        alex.birthday = Some("1990-04-12".into());
        let mut bea = contact("2", Some("Bea"), Some("Moreno"));
        bea.birthday = Some("1604-09-30".into());
        lib.put_contacts(&[alex, contact("3", Some("Cy"), Some("Lane")), bea])
            .unwrap();
        assert!(lib.any_birthday().unwrap());
        let born = lib.contacts_with_birthdays().unwrap();
        let whole: Vec<Contact> = lib
            .contacts(None)
            .unwrap()
            .into_iter()
            .filter(|c| c.birthday.is_some())
            .collect();
        let names = |cs: &[Contact]| -> Vec<String> {
            cs.iter()
                .map(|c| {
                    format!(
                        "{} {:?} {:?} {:?}",
                        c.guid, c.first_name, c.last_name, c.birthday
                    )
                })
                .collect()
        };
        assert_eq!(names(&born), names(&whole));
        assert_eq!(born.len(), 2);
        assert!(born.iter().all(|c| c.methods.is_empty()), "names only");
    }

    #[test]
    fn search_ignores_case_accents_and_word_order() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("1", Some("Émile"), Some("Zola")),
            contact("2", Some("Zoë"), Some("Ahuja")),
        ])
        .unwrap();
        let found = |q: &str| -> Vec<String> {
            lib.contacts(Some(q))
                .unwrap()
                .iter()
                .map(|c| c.guid.clone())
                .collect()
        };
        assert_eq!(found("emile"), ["1"]);
        assert_eq!(found("ÉMILE"), ["1"]);
        assert_eq!(found("zola emile"), ["1"]);
        assert_eq!(found("zoe"), ["2"]);
        // A wildcard character is a character, not a pattern.
        assert!(found("%").is_empty());
        assert!(found("_").is_empty());
    }

    #[test]
    fn a_number_matches_however_it_was_typed() {
        let mut lib = Library::in_memory().unwrap();
        let mut c = contact("1", Some("Meera"), None);
        c.methods[0].value = "+91 98765-43210".into();
        lib.put_contacts(&[c]).unwrap();
        for typed in ["9876543210", "98765 43210", "(987) 65", "+91-98765"] {
            assert_eq!(lib.contacts(Some(typed)).unwrap().len(), 1, "{typed}");
        }
        assert!(lib.contacts(Some("12345")).unwrap().is_empty());
    }

    #[test]
    fn accented_names_sort_with_their_letter() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("1", Some("Zoya"), Some("Zutshi")),
            contact("2", Some("Émile"), Some("Écrivain")),
            contact("3", Some("Aarav"), Some("Ahuja")),
        ])
        .unwrap();
        let order: Vec<String> = lib
            .contacts(None)
            .unwrap()
            .iter()
            .map(|c| c.guid.clone())
            .collect();
        assert_eq!(order, ["3", "2", "1"]);
    }

    #[test]
    fn contacts_sort_by_last_name_the_way_apple_does() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("1", Some("Aarav"), Some("Zutshi")),
            contact("2", Some("Zoya"), Some("Ahuja")),
        ])
        .unwrap();
        let names: Vec<String> = lib
            .contacts(None)
            .unwrap()
            .iter()
            .map(Contact::display_name)
            .collect();
        assert_eq!(names, vec!["Zoya Ahuja", "Aarav Zutshi"]);
    }

    /// A contact with only a phone number still needs something to show.
    #[test]
    fn a_nameless_contact_falls_back_to_its_first_method() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[contact("9", None, None)]).unwrap();
        assert_eq!(lib.contacts(None).unwrap()[0].display_name(), "+919");
    }

    #[test]
    fn searching_matches_a_phone_number_as_well_as_a_name() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[
            contact("555", Some("Meera"), Some("Iyer")),
            contact("777", Some("Rahul"), Some("Nair")),
        ])
        .unwrap();
        assert_eq!(lib.contacts(Some("meera")).unwrap().len(), 1);
        assert_eq!(
            lib.contacts(Some("+91777")).unwrap()[0].display_name(),
            "Rahul Nair"
        );
        assert_eq!(
            lib.contacts(Some("   ")).unwrap().len(),
            2,
            "a blank search is no filter"
        );
    }

    /// A number removed on the phone must not linger here.
    #[test]
    fn methods_are_replaced_not_accumulated() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[contact("1", Some("Ada"), Some("Lovelace"))])
            .unwrap();

        let mut changed = contact("1", Some("Ada"), Some("Lovelace"));
        changed.methods = vec![ContactMethod {
            kind: "email".into(),
            label: None,
            value: "ada@example.com".into(),
        }];
        lib.put_contacts(&[changed]).unwrap();

        let stored = &lib.contacts(None).unwrap()[0];
        assert_eq!(stored.methods.len(), 1);
        assert_eq!(stored.methods[0].kind, "email");
    }

    #[test]
    fn contacts_apple_stops_reporting_are_removed() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[contact("1", Some("A"), None), contact("2", Some("B"), None)])
            .unwrap();
        lib.put_contacts(&[contact("1", Some("A"), None)]).unwrap();
        assert_eq!(lib.contacts(None).unwrap().len(), 1);
    }

    fn address(
        street: &str,
        city: &str,
        state: &str,
        postcode: &str,
        country: &str,
    ) -> PostalAddress {
        let part = |v: &str| (!v.is_empty()).then(|| v.to_string());
        PostalAddress {
            label: Some("HOME".into()),
            street: part(street),
            city: part(city),
            state: part(state),
            postal_code: part(postcode),
            country: part(country),
            ..PostalAddress::default()
        }
    }

    #[test]
    fn an_address_is_laid_out_the_way_its_country_writes_it() {
        let us = address("1 Main St", "Springfield", "IL", "62701", "United States");
        assert_eq!(
            us.formatted(),
            "1 Main St\nSpringfield, IL 62701\nUnited States"
        );
        let de = address("Hauptstraße 5", "Berlin", "", "10115", "Germany");
        assert_eq!(de.formatted(), "Hauptstraße 5\n10115 Berlin\nGermany");
        let mut fr = address("5 Rue de la Paix", "Paris", "", "75002", "");
        fr.country_code = Some("FR".into());
        assert_eq!(fr.formatted(), "5 Rue de la Paix\n75002 Paris");
        let uk = address("10 Downing St", "London", "", "SW1A 2AA", "UK");
        assert_eq!(uk.formatted(), "10 Downing St\nLondon\nSW1A 2AA\nUK");
        let india = address(
            "12 MG Road\nIndiranagar",
            "Bengaluru",
            "Karnataka",
            "560038",
            "India",
        );
        assert_eq!(
            india.formatted(),
            "12 MG Road\nIndiranagar\nBengaluru 560038\nKarnataka\nIndia"
        );
        let jp = address("1-1 Chiyoda", "Chiyoda-ku", "Tokyo", "100-0001", "Japan");
        assert_eq!(
            jp.formatted(),
            "100-0001\nTokyo Chiyoda-ku\n1-1 Chiyoda\nJapan"
        );
        // Anywhere else: town, region and postcode on one line, and no gaps.
        let other = address("", "Lagos", "", "100001", "Nigeria");
        assert_eq!(other.formatted(), "Lagos 100001\nNigeria");
    }

    #[test]
    fn a_country_typed_by_name_finds_its_code() {
        assert_eq!(country_code_for("United States of America"), Some("us"));
        assert_eq!(country_code_for(" Deutschland "), Some("de"));
        assert_eq!(country_code_for("U.K."), Some("gb"));
        assert_eq!(country_code_for("Narnia"), None);
        // Apple's code outranks a name that disagrees with it.
        let mut a = address("", "Paris", "", "75002", "Germany");
        a.country_code = Some("fr".into());
        assert_eq!(a.country_key(), Some("fr"));
    }

    #[test]
    fn a_part_is_set_by_apples_name_for_it() {
        let mut a = PostalAddress::default();
        assert!(a.is_empty());
        assert!(a.set_field("postalCode", Some(" 560038 ")));
        assert_eq!(a.postal_code.as_deref(), Some("560038"));
        assert!(a.set_field("postalCode", Some("  ")));
        assert_eq!(a.postal_code, None, "blank clears");
        assert!(!a.set_field("planet", Some("Earth")));
    }

    #[test]
    fn addresses_are_kept_part_by_part_and_go_with_their_card() {
        let mut lib = Library::in_memory().unwrap();
        lib.put_contacts(&[contact("1", Some("A"), None), contact("2", Some("B"), None)])
            .unwrap();
        let home = address("1 Main St", "Springfield", "IL", "62701", "USA");
        lib.put_addresses(
            &[
                ("1".into(), vec![home.clone(), PostalAddress::default()]),
                ("2".into(), vec![home.clone()]),
            ],
            true,
        )
        .unwrap();
        assert_eq!(
            lib.contact_addresses("1").unwrap(),
            std::slice::from_ref(&home),
            "blanks dropped"
        );

        // A whole book replaces every address; a card Apple stopped reporting takes its own.
        lib.put_contacts(&[contact("1", Some("A"), None)]).unwrap();
        assert!(lib.contact_addresses("2").unwrap().is_empty());
        lib.put_addresses(&[], true).unwrap();
        assert!(lib.contact_addresses("1").unwrap().is_empty());
    }

    #[test]
    fn an_edited_address_replaces_its_flattened_lines_in_place() {
        let mut lib = Library::in_memory().unwrap();
        let mut c = contact("1", Some("A"), None);
        c.methods.push(ContactMethod {
            kind: "address".into(),
            label: Some("HOME".into()),
            value: "old".into(),
        });
        c.methods.push(ContactMethod {
            kind: "profile".into(),
            label: None,
            value: "@a".into(),
        });
        lib.put_contacts(&[c]).unwrap();
        let moved = address("2 New Rd", "Pune", "MH", "411001", "India");
        lib.set_contact_addresses("1", std::slice::from_ref(&moved))
            .unwrap();

        assert_eq!(lib.contact_addresses("1").unwrap(), [moved]);
        let kinds: Vec<(String, String)> = lib.contacts(None).unwrap()[0]
            .methods
            .iter()
            .map(|m| (m.kind.clone(), m.value.clone()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("phone".into(), "+911".into()),
                ("address".into(), "2 New Rd\nPune 411001\nMH\nIndia".into()),
                ("profile".into(), "@a".into()),
            ]
        );
        assert_eq!(
            lib.contacts(Some("pune")).unwrap().len(),
            1,
            "searchable at once"
        );
    }
}
