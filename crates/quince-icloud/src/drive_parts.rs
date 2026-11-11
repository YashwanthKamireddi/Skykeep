//! A Drive file read in parts: a byte range of it fetched from Apple's download link, never
//! the whole file. One way for every front end that reads files on demand (the desktop's
//! folder, the phone's Files app), so each serves the same bytes the same way.
//!
//! Apple's download links answer HTTP ranges (proven live; desktop `mount.rs` since the
//! folder's first release). A link is signed and lasts a while, so it is asked for once per
//! file version and kept, and asked for again once when Apple stops honouring it.

use crate::drive::download_url_of_version;
use crate::{Error, Result};
use quince_core::drive::DriveItem;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a download link is used before a fresh one is asked for: well inside the time
/// Apple's signed links are honoured for (measured at more than half an hour).
pub const LINK_LIFE: Duration = Duration::from_secs(20 * 60);

/// Part of a file as Apple sent it, and how long Apple says the whole file is, where it said.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Served {
    pub bytes: Vec<u8>,
    /// The file's length as served (a range answer's `Content-Range` total, a whole answer's
    /// length): the truth where a listing's size is not.
    pub total: Option<u64>,
}

/// The download links in use, by file and version: each asked of Apple once while it lasts.
#[derive(Default)]
pub struct Links(Mutex<HashMap<String, (String, Instant)>>);

impl Links {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn known(&self, key: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .filter(|(_, at)| at.elapsed() < LINK_LIFE)
            .map(|(u, _)| u.clone())
    }

    fn keep(&self, key: String, url: String) {
        let mut links = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Spent links go as new ones come: the map stays the size of what is being read.
        links.retain(|_, (_, at)| at.elapsed() < LINK_LIFE);
        links.insert(key, (url, Instant::now()));
    }

    /// `len` bytes of `item` from `offset` (fewer only at its end), from the link for exactly
    /// the version `item` names: a file changed on another device since it was listed is
    /// refused ([`Error::Conflict`]), never read in parts of two versions.
    ///
    /// # Errors
    /// When Apple cannot be reached, will not give a link, or answers the range with an error.
    pub async fn read(
        &self,
        http: &reqwest::Client,
        docws_root: &str,
        params: Vec<(String, String)>,
        item: &DriveItem,
        offset: u64,
        len: u64,
    ) -> Result<Served> {
        // Nothing asked for is nothing to fetch, and `offset + len - 1` has no last byte to
        // name for it.
        if len == 0 {
            return Ok(Served::default());
        }
        let key = format!("{}~{}", item.drivewsid, item.etag.as_deref().unwrap_or(""));
        let known = self.known(&key);
        // A link Apple no longer honours is asked for again, once.
        for fresh in [known.is_none(), true] {
            let url = match (&known, fresh) {
                (Some(u), false) => u.clone(),
                _ => {
                    let u = download_url_of_version(http, docws_root, params.clone(), item).await?;
                    self.keep(key.clone(), u.clone());
                    u
                }
            };
            match read_range(http, &url, offset, len).await? {
                Ranged::Part(part) => return Ok(part),
                Ranged::Expired if !fresh => {}
                Ranged::Expired => break,
            }
        }
        Err(Error::Refused("Apple would not send the file.".into()))
    }
}

/// How one ranged request ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Ranged {
    Part(Served),
    /// The link is no longer honoured (401, 403, 404 or 410): to be asked for again.
    Expired,
}

/// `len` bytes from `offset` of what `url` serves (`len` > 0).
///
/// # Errors
/// When the request fails, or Apple answers anything but a part, the whole, or "past the end".
pub async fn read_range(
    http: &reqwest::Client,
    url: &str,
    offset: u64,
    len: u64,
) -> Result<Ranged> {
    let last = offset.saturating_add(len.max(1)) - 1;
    let r = http
        .get(url)
        .header("Range", format!("bytes={offset}-{last}"))
        .send()
        .await?;
    let status = r.status().as_u16();
    if matches!(status, 401 | 403 | 404 | 410) {
        return Ok(Ranged::Expired);
    }
    let whole = status == 200;
    if status != 206 && !whole && status != 416 {
        return Err(Error::Protocol(format!(
            "a file's part was answered {status}"
        )));
    }
    // The length as served: a range answer's total (`bytes 0-0/52311`), a 416's
    // (`bytes */52311`: asked past the end of a file shorter than listed).
    let total = r
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(served_total);
    if status == 416 {
        return match total {
            Some(total) => Ok(Ranged::Part(Served {
                bytes: Vec::new(),
                total: Some(total),
            })),
            None => Err(Error::Protocol(format!(
                "a file's part was answered {status}"
            ))),
        };
    }
    // A server that ignores the range sends the whole file: the part is read out of it as it
    // streams, from the offset, and the rest is never taken.
    let part = if whole {
        body_part(r, offset, len).await?
    } else {
        let part = body_part(r, 0, len).await?;
        Served {
            bytes: part.bytes,
            total,
        }
    };
    Ok(Ranged::Part(part))
}

/// `len` bytes from `skip` on of a body, read as it arrives: never more of it is held than the
/// part, so a server that answers a range with the whole file (200) costs the bytes up to the
/// part and not the file in memory on every read. Its length is the file's when the server
/// gives one, or when the body ended before the part was whole; otherwise not known.
async fn body_part(mut r: reqwest::Response, skip: u64, len: u64) -> Result<Served> {
    let declared = r.content_length();
    let mut seen: u64 = 0;
    let mut bytes = Vec::with_capacity(usize::try_from(len.min(1 << 20)).unwrap_or(0));
    let end = skip.saturating_add(len);
    while seen < end {
        let Some(chunk) = r.chunk().await? else {
            // The whole body read: its length is the file's, whatever was declared.
            return Ok(Served {
                bytes,
                total: Some(seen),
            });
        };
        let at = seen;
        seen += chunk.len() as u64;
        // The piece of this chunk inside [skip, end).
        let from = usize::try_from(skip.saturating_sub(at).min(chunk.len() as u64)).unwrap_or(0);
        let to = usize::try_from(end.saturating_sub(at).min(chunk.len() as u64)).unwrap_or(0);
        if from < to {
            bytes.extend_from_slice(&chunk[from..to]);
        }
    }
    Ok(Served {
        bytes,
        total: declared,
    })
}

/// The whole length a `Content-Range` gives: `bytes 0-9/52311` or `bytes */52311`. `None` for
/// one that says it is not known (`/*`) or cannot be read.
fn served_total(range: &str) -> Option<u64> {
    range
        .trim()
        .strip_prefix("bytes")?
        .rsplit_once('/')?
        .1
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The length a file is served at, from a range answer or a 416 past its end.
    #[test]
    fn a_served_length_is_read_from_content_range() {
        assert_eq!(served_total("bytes 0-0/52311"), Some(52_311));
        assert_eq!(served_total("bytes */52311"), Some(52_311));
        assert_eq!(served_total("bytes 0-9/*"), None);
        assert_eq!(served_total("items 0-9/10"), None);
    }

    /// A range answered with the whole file (200) gives just the part, read as it streams,
    /// with the file's length; a part past the end is empty, never a panic.
    #[tokio::test]
    async fn a_whole_answer_gives_only_the_part() {
        let server = MockServer::start().await;
        let file: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(file.clone()))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let url = server.uri();
        let part = |offset, len| {
            let (http, url) = (http.clone(), url.clone());
            async move {
                match read_range(&http, &url, offset, len).await.unwrap() {
                    Ranged::Part(p) => p,
                    Ranged::Expired => panic!("expired"),
                }
            }
        };
        let p = part(150_000, 10).await;
        assert_eq!(p.bytes, file[150_000..150_010]);
        assert_eq!(p.total, Some(200_000));
        let tail = part(199_995, 10).await;
        assert_eq!(tail.bytes, file[199_995..]);
        assert_eq!(tail.total, Some(200_000));
        let past = part(300_000, 10).await;
        assert!(past.bytes.is_empty());
        assert_eq!(past.total, Some(200_000));
    }

    /// A range answered as a range: its bytes and the total from `Content-Range`; a link no
    /// longer honoured is said as that, to be asked for again.
    #[tokio::test]
    async fn a_range_answer_and_a_spent_link() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("Range", "bytes=10-13"))
            .respond_with(
                ResponseTemplate::new(206)
                    .insert_header("Content-Range", "bytes 10-13/900")
                    .set_body_bytes(b"abcd".to_vec()),
            )
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let got = read_range(&http, &server.uri(), 10, 4).await.unwrap();
        assert_eq!(
            got,
            Ranged::Part(Served {
                bytes: b"abcd".to_vec(),
                total: Some(900)
            })
        );
        let gone = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&gone)
            .await;
        assert_eq!(
            read_range(&http, &gone.uri(), 0, 4).await.unwrap(),
            Ranged::Expired
        );
    }
}
