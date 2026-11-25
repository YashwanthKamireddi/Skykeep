//! Decompressing someone else's bytes, with a ceiling. A shared note's table, a feed or a
//! document arrives compressed, and ten megabytes of zeros inflate to gigabytes: read whole,
//! that was enough to take the computer's memory (and on Linux, the whole desktop with it).
//! Every inflate of remote data reads through here, and anything past its ceiling is refused —
//! never cut short and read as if whole.

use std::io::Read;

/// The most any one remote payload inflates to: far beyond a real note, table or calendar.
pub const MOST_INFLATED: u64 = 64 * 1024 * 1024;

/// All of `reader`, or `None` when it fails or holds more than `most` bytes.
pub fn read_capped(reader: impl Read, most: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    reader
        .take(most.saturating_add(1))
        .read_to_end(&mut out)
        .ok()?;
    (out.len() as u64 <= most).then_some(out)
}

/// The most one answer from a server is read to: hundreds of thousands of Drive items in one
/// folder listing, far beyond any real answer, and still short of taking the memory with it.
pub const MOST_REPLY: u64 = 256 * 1024 * 1024;

/// A server's whole answer as text, within [`MOST_REPLY`]: the one way an answer is read.
///
/// Read as `text().unwrap_or_default()` before, an answer had no ceiling, and a connection
/// dropped part way through came back as an empty answer and was said as one not understood
/// rather than as the connection it was.
pub async fn reply(response: reqwest::Response) -> crate::Result<String> {
    reply_within(response, MOST_REPLY).await
}

/// [`reply`] within `most` bytes: anything longer is refused, never cut short and read as if
/// whole.
pub async fn reply_within(mut response: reqwest::Response, most: u64) -> crate::Result<String> {
    let too_long = || {
        crate::Error::Protocol(format!(
            "the answer was larger than {} MB",
            most / (1024 * 1024)
        ))
    };
    if response.content_length().is_some_and(|n| n > most) {
        return Err(too_long());
    }
    let mut body = Vec::new();
    while let Some(piece) = response.chunk().await? {
        if (body.len() + piece.len()) as u64 > most {
            return Err(too_long());
        }
        quince_record::landed(piece.len() as u64);
        body.extend_from_slice(&piece);
    }
    Ok(match String::from_utf8(body) {
        Ok(text) => text,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    })
}

/// A gzip stream inflated, within `most`.
pub fn gunzip(data: &[u8], most: u64) -> Option<Vec<u8>> {
    read_capped(flate2::read::GzDecoder::new(data), most)
}

/// A zlib stream inflated, within `most`.
pub fn unzlib(data: &[u8], most: u64) -> Option<Vec<u8>> {
    read_capped(flate2::read::ZlibDecoder::new(data), most)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    /// A server that writes `head` and `body` raw to the first connection, then closes it:
    /// an answer streamed in pieces with no length said, or one cut off part way.
    pub(crate) async fn raw_server(head: &'static str, body: Vec<u8>) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut asked = [0u8; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut asked).await;
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(&body).await;
            let _ = socket.shutdown().await;
        });
        at
    }

    fn chunked(pieces: usize, size: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for _ in 0..pieces {
            out.extend_from_slice(format!("{size:x}\r\n").as_bytes());
            out.extend(std::iter::repeat_n(b'a', size));
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"0\r\n\r\n");
        out
    }

    const CHUNKED: &str =
        "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";

    /// Every byte of an answer is counted in the download record, under the reason its work
    /// was given (quince_record): 4,096 bytes read for an update land as 4,096 more there.
    #[tokio::test]
    async fn an_answer_is_counted_under_the_reason_its_work_was_given() {
        use quince_record::{because, Why, RECORD};
        let updates = || {
            RECORD
                .days()
                .first()
                .map_or(0, |d| d.bytes[Why::Updates.code()])
        };
        let before = updates();
        let at = raw_server(CHUNKED, chunked(4, 1024)).await;
        let got = because(Why::Updates, async {
            reply_within(reqwest::get(&at).await.unwrap(), 4096).await
        })
        .await
        .unwrap();
        assert_eq!(got.len(), 4096);
        assert_eq!(updates() - before, 4096);
    }

    /// An answer with no length said is read up to the ceiling and refused past it, never
    /// cut short and read as if whole; one within it reads whole.
    #[tokio::test]
    async fn an_answer_past_the_ceiling_is_refused_and_one_within_reads_whole() {
        let at = raw_server(CHUNKED, chunked(8, 1024)).await;
        let got = reply_within(reqwest::get(&at).await.unwrap(), 4096).await;
        assert!(matches!(got, Err(crate::Error::Protocol(_))), "{got:?}");

        let at = raw_server(CHUNKED, chunked(4, 1024)).await;
        let got = reply_within(reqwest::get(&at).await.unwrap(), 4096)
            .await
            .unwrap();
        assert_eq!(got.len(), 4096);
    }

    /// A length said up front past the ceiling is refused before a byte of it is read.
    #[tokio::test]
    async fn a_length_said_past_the_ceiling_is_refused_unread() {
        let at = raw_server(
            "HTTP/1.1 200 OK\r\ncontent-length: 1000000000\r\nconnection: close\r\n\r\n",
            Vec::new(),
        )
        .await;
        let got = reply_within(reqwest::get(&at).await.unwrap(), 4096).await;
        assert!(matches!(got, Err(crate::Error::Protocol(_))), "{got:?}");
    }

    #[test]
    fn a_bomb_is_refused_and_a_real_payload_reads_whole() {
        let bomb = gz(&vec![0u8; 4 * 1024 * 1024]);
        assert!(bomb.len() < 64 * 1024, "{}", bomb.len());
        assert_eq!(gunzip(&bomb, 1024 * 1024), None);
        assert_eq!(
            gunzip(&bomb, 4 * 1024 * 1024).map(|v| v.len()),
            Some(4 * 1024 * 1024)
        );
        assert_eq!(gunzip(&gz(b"hello"), 5).as_deref(), Some(&b"hello"[..]));
        assert_eq!(gunzip(b"not gzip", 100), None);
    }
}
