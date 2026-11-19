//! A file downloaded whole from iCloud: checked to its last byte, and carried on from where it
//! stopped when the connection drops or iCloud sends it in parts.
//!
//! iCloud can answer with part of a file only, a large video especially (icloudpd #1311: "a
//! file that iCloud sent in parts (e.g. videos over 1 GB) was saved after the first part, cut
//! off and without an error, and never downloaded again"), and a connection can end anywhere in
//! a four-gigabyte film. Each download here knows the whole file's length from Apple's answer,
//! asks for the rest (`Range: bytes=<have>-`) for as long as each try brings more, and is whole
//! only when every byte is in. Before, a photo download took whatever ended its stream as the
//! file, and one that broke off started again from nothing.

use std::path::Path;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{CONTENT_RANGE, RANGE};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::{Error, Result};

/// Tries in a row that bring no new byte before a download is given up.
const STUCK: u32 = 3;

/// The pause before asking again, after a try that brought nothing.
const PAUSE: Duration = Duration::from_secs(2);

/// What asking for a file came to.
pub enum Opened {
    /// Apple would not send it: the link has expired, or was refused (its status). The caller
    /// may ask Apple for a fresh link.
    Refused(u16),
    /// It is on its way: [`Download::save`] writes it.
    Sending(Download),
}

/// A file Apple is sending.
pub struct Download {
    http: reqwest::Client,
    url: String,
    first: Option<reqwest::Response>,
    total: Option<u64>,
    /// Never started over: its bytes are read as they land (`only_onward`).
    onward: bool,
}

/// Asks for the file at `url`.
///
/// # Errors
/// When Apple cannot be reached.
pub async fn open(http: &reqwest::Client, url: &str) -> Result<Opened> {
    let mut url = url.to_string();
    let mut response = http.get(&url).send().await?;
    // iCloud answers some downloads with 330 and where to ask instead, which HTTP clients
    // don't follow by themselves (rclone's iCloud Drive follows it: "icloud has some weird http
    // codes"). Followed once; the address is checked as every other is (Apple's alone).
    if response.status().as_u16() == 330 {
        let moved = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .and_then(|l| response.url().join(l).ok());
        if let Some(moved) = moved {
            url = moved.to_string();
            response = http.get(&url).send().await?;
        }
    }
    let status = response.status();
    if !status.is_success() {
        return Ok(Opened::Refused(status.as_u16()));
    }
    let total = whole_length(&response);
    Ok(Opened::Sending(Download {
        http: http.clone(),
        url,
        first: Some(response),
        total,
        onward: false,
    }))
}

impl Download {
    /// Never written from the start again: for a file read while it lands, whose readers have
    /// the bytes already told of. A server that answers "the rest" with the whole file again
    /// (iCloud doesn't) ends such a download instead.
    #[must_use]
    pub fn only_onward(mut self) -> Self {
        self.onward = true;
        self
    }

    /// The whole file's length, where Apple said it.
    #[must_use]
    pub fn total(&self) -> Option<u64> {
        self.total
    }

    /// Written to `part` (made new: never one already there, nor a link left in its place),
    /// every byte, `landed` told of each piece once it is
    /// written and flushed, so a reader of `part` finds what it was told of. Answers the
    /// file's length.
    ///
    /// # Errors
    /// When the file can't be written, Apple stops sending it (three tries in a row bring
    /// nothing), the link stops working part way, or what arrives is longer than Apple said.
    pub async fn save(mut self, part: &Path, mut landed: impl FnMut(u64) + Send) -> Result<u64> {
        let mut out = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(part)
            .await?;
        let mut have = 0u64;
        let mut stuck = 0u32;
        let mut response = self.first.take();
        loop {
            let answer = match response.take() {
                Some(r) => Ok(r),
                None => {
                    let mut ask = self.http.get(&self.url);
                    if have > 0 {
                        ask = ask.header(RANGE, format!("bytes={have}-"));
                    }
                    ask.send().await
                }
            };
            let before = have;
            let broke = match answer {
                Err(e) => Some(e),
                Ok(r) => {
                    if have > 0 && !self.carries_on(&r, have)? {
                        if self.onward {
                            return Err(Error::Protocol(
                                "iCloud would not send the rest of the file".into(),
                            ));
                        }
                        // The rest was not sent as asked (the whole file again): from the start.
                        out.set_len(0).await?;
                        out.seek(std::io::SeekFrom::Start(0)).await?;
                        have = 0;
                        self.total = whole_length(&r);
                    }
                    let mut body = r.bytes_stream();
                    let mut broke = None;
                    while let Some(piece) = body.next().await {
                        match piece {
                            Ok(bytes) => {
                                out.write_all(&bytes).await?;
                                out.flush().await?;
                                have += bytes.len() as u64;
                                landed(bytes.len() as u64);
                            }
                            Err(e) => {
                                broke = Some(e);
                                break;
                            }
                        }
                    }
                    broke
                }
            };
            match self.total {
                Some(total) if have > total => {
                    return Err(Error::Protocol(format!(
                        "iCloud sent {have} bytes of a file it said was {total}"
                    )));
                }
                Some(total) if have == total => break,
                // No length said: whole once its stream ends cleanly.
                None if broke.is_none() => break,
                _ => {}
            }
            if have > before {
                stuck = 0;
            } else {
                stuck += 1;
            }
            if stuck >= STUCK {
                return Err(match broke {
                    Some(e) => Error::Network(e),
                    None => Error::Protocol(format!(
                        "iCloud stopped sending the file at {have} of {} bytes",
                        self.total.unwrap_or_default()
                    )),
                });
            }
            tokio::time::sleep(PAUSE).await;
        }
        out.flush().await?;
        out.sync_all().await?;
        Ok(have)
    }

    /// Whether `r`, the answer to asking for the rest from `have`, carries on from there: a
    /// range answer starting at `have`, of the same file. False when it is the whole file again
    /// (a server that does not answer ranges).
    fn carries_on(&self, r: &reqwest::Response, have: u64) -> Result<bool> {
        let status = r.status().as_u16();
        if status == 200 {
            return Ok(false);
        }
        if status != 206 {
            return Err(Error::Protocol(format!(
                "iCloud stopped sending the file part way ({status})"
            )));
        }
        match content_range(r) {
            Some((start, total)) if start == have && self.total.is_none_or(|t| t == total) => {
                Ok(true)
            }
            _ => Err(Error::Protocol(
                "iCloud sent the rest of a different file".into(),
            )),
        }
    }
}

/// The whole file's length an answer says: a range answer's `Content-Range` total, a whole
/// answer's `Content-Length`.
fn whole_length(r: &reqwest::Response) -> Option<u64> {
    if r.status().as_u16() == 206 {
        content_range(r).map(|(_, total)| total)
    } else {
        r.content_length()
    }
}

/// Where a range answer starts, and the whole file's length: `bytes 100-199/1000`.
fn content_range(r: &reqwest::Response) -> Option<(u64, u64)> {
    parse_content_range(r.headers().get(CONTENT_RANGE)?.to_str().ok()?)
}

fn parse_content_range(v: &str) -> Option<(u64, u64)> {
    let (range, total) = v.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A local server answering each request in turn as `answer` says, from what it was
    /// asked (the `Range` start, or 0): the status line and headers, the bytes, and whether to
    /// cut the connection after them.
    async fn server(
        answer: impl Fn(usize, u64) -> (String, Vec<u8>) + Send + Sync + 'static,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut n = 0;
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let got = socket.read(&mut buf).await.unwrap_or(0);
                let asked = String::from_utf8_lossy(&buf[..got]).to_lowercase();
                let from = asked
                    .split("range: bytes=")
                    .nth(1)
                    .and_then(|r| r.split('-').next())
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
                let (head, body) = answer(n, from);
                n += 1;
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://{addr}/film")
    }

    /// A folder of the test's own, under the system's temporary folder.
    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("quince-download-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    async fn fetch(url: &str, part: &Path) -> Result<u64> {
        let http = reqwest::Client::new();
        match open(&http, url).await? {
            Opened::Sending(d) => d.save(part, |_| {}).await,
            Opened::Refused(s) => panic!("refused {s}"),
        }
    }

    /// iCloud sending part of a file (a 206 whose range stops short): the rest is asked for
    /// from where it stopped, and the file is whole (icloudpd #1311).
    #[tokio::test]
    async fn a_file_sent_in_parts_is_asked_for_the_rest() {
        let whole = file(10_000);
        let w = whole.clone();
        let url = server(move |_, from| {
            let from = from as usize;
            let to = (from + 4000).min(w.len());
            (
                format!(
                    "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {from}-{}/{}\r\ncontent-length: {}\r\n\r\n",
                    to - 1,
                    w.len(),
                    to - from
                ),
                w[from..to].to_vec(),
            )
        })
        .await;
        let dir = scratch();
        let part = dir.join("f.part");
        assert_eq!(fetch(&url, &part).await.unwrap(), 10_000);
        assert_eq!(std::fs::read(&part).unwrap(), whole);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A connection that ends part way (fewer bytes than its length said) is carried on from
    /// where it stopped, not started again.
    #[tokio::test]
    async fn a_dropped_connection_carries_on_where_it_stopped() {
        let whole = file(50_000);
        let w = whole.clone();
        let url = server(move |n, from| {
            let from = from as usize;
            if n == 0 {
                // Says 50,000, sends 20,000, and the connection ends.
                (
                    format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", w.len()),
                    w[..20_000].to_vec(),
                )
            } else {
                (
                    format!(
                        "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {from}-{}/{}\r\ncontent-length: {}\r\n\r\n",
                        w.len() - 1,
                        w.len(),
                        w.len() - from
                    ),
                    w[from..].to_vec(),
                )
            }
        })
        .await;
        let dir = scratch();
        let part = dir.join("f.part");
        assert_eq!(fetch(&url, &part).await.unwrap(), 50_000);
        assert_eq!(std::fs::read(&part).unwrap(), whole);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that answers "the rest" with the whole file again is read from the start, never
    /// appended twice.
    #[tokio::test]
    async fn the_whole_file_again_is_started_over() {
        let whole = file(30_000);
        let w = whole.clone();
        let url = server(move |n, _| {
            let body = if n == 0 {
                w[..10_000].to_vec()
            } else {
                w.clone()
            };
            (
                format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", w.len()),
                body,
            )
        })
        .await;
        let dir = scratch();
        let part = dir.join("f.part");
        assert_eq!(fetch(&url, &part).await.unwrap(), 30_000);
        assert_eq!(std::fs::read(&part).unwrap(), whole);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file iCloud keeps stopping short of is given up after three tries that bring nothing,
    /// never taken as whole.
    #[tokio::test]
    async fn a_file_that_never_arrives_whole_is_an_error() {
        let w = file(10_000);
        let url = server(move |n, _| {
            if n == 0 {
                (
                    format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", w.len()),
                    w[..1000].to_vec(),
                )
            } else {
                (
                    "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes 1000-9999/10000\r\ncontent-length: 9000\r\n\r\n".into(),
                    Vec::new(),
                )
            }
        })
        .await;
        let dir = scratch();
        assert!(fetch(&url, &dir.join("f.part")).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// iCloud's 330, with where to ask instead, is followed to the file.
    #[tokio::test]
    async fn a_330_is_followed_to_the_file() {
        let whole = file(5_000);
        let w = whole.clone();
        let url = server(move |n, _| {
            if n == 0 {
                (
                    "HTTP/1.1 330 Moved\r\nlocation: /elsewhere\r\ncontent-length: 0\r\n\r\n"
                        .into(),
                    Vec::new(),
                )
            } else {
                (
                    format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", w.len()),
                    w.clone(),
                )
            }
        })
        .await;
        let dir = scratch();
        let part = dir.join("f.part");
        assert_eq!(fetch(&url, &part).await.unwrap(), 5_000);
        assert_eq!(std::fs::read(&part).unwrap(), whole);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_content_range_is_read() {
        assert_eq!(parse_content_range("bytes 100-199/1000"), Some((100, 1000)));
        assert_eq!(parse_content_range("bytes */1000"), None);
        assert_eq!(parse_content_range("items 1-2/3"), None);
    }
}
