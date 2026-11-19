//! A file's bytes sent to Apple, watched rather than timed.
//!
//! An upload takes as long as the line and the file make it: half a gigabyte on a slow uplink
//! is many minutes, and nothing comes back over HTTP/1.1 until the last byte is in. A timeout
//! on the whole request or on reads cut such uploads off part way (reqwest's read timeout runs
//! while the body is sent: seanmonstar/reqwest#2784). What tells a dead upload from a slow one
//! is whether bytes still move, as rclone's `--timeout` is an idle time on a transfer and not
//! a limit on it: so an upload is given up only when Apple takes no bytes for [`STALL`], or
//! has them all and does not answer within [`ANSWER`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::{Error, Result};

/// Apple has taken no bytes for this long: the line has gone.
pub const STALL: Duration = Duration::from_secs(60);

/// Every byte is in, and Apple has not answered for this long. Apple checks a large file
/// before it answers; rclone allows five minutes of quiet on a transfer, and so does this.
pub const ANSWER: Duration = Duration::from_secs(300);

/// How often what has been sent is looked at.
const LOOK: Duration = Duration::from_secs(1);

/// Sends `request`, whose body counts what it has handed over in `sent` (`size` in all),
/// through [`quince_auth::Client::uploads`], and answers Apple's status and reply. Given up,
/// as [`Error::Stalled`], when no byte goes for [`STALL`], or Apple has had every byte for
/// [`ANSWER`] without answering, or its answer stops arriving for [`STALL`].
pub async fn send(
    request: reqwest::RequestBuilder,
    sent: &AtomicU64,
    size: u64,
) -> Result<(reqwest::StatusCode, String)> {
    send_within(request, sent, size, STALL, ANSWER).await
}

async fn send_within(
    request: reqwest::RequestBuilder,
    sent: &AtomicU64,
    size: u64,
    stall: Duration,
    answer: Duration,
) -> Result<(reqwest::StatusCode, String)> {
    let response = watched(request.send(), sent, size, stall, answer).await??;
    let status = response.status();
    let text = tokio::time::timeout(stall, response.text())
        .await
        .map_err(|_| Error::Stalled)??;
    Ok((status, text))
}

async fn watched<F: std::future::Future>(
    going: F,
    sent: &AtomicU64,
    size: u64,
    stall: Duration,
    answer: Duration,
) -> Result<F::Output> {
    tokio::pin!(going);
    let mut last = sent.load(Ordering::Relaxed);
    let mut moved = tokio::time::Instant::now();
    loop {
        tokio::select! {
            done = &mut going => return Ok(done),
            () = tokio::time::sleep(LOOK) => {
                let now = sent.load(Ordering::Relaxed);
                if now != last {
                    last = now;
                    moved = tokio::time::Instant::now();
                }
                let quiet = if now >= size { answer } else { stall };
                if moved.elapsed() > quiet {
                    return Err(Error::Stalled);
                }
            }
        }
    }
}

/// `bytes` as a body handed over in pieces, each counted in `sent` as it goes.
pub fn counted(bytes: Vec<u8>, sent: std::sync::Arc<AtomicU64>) -> reqwest::Body {
    const PIECE: usize = 256 * 1024;
    let pieces: Vec<Vec<u8>> = bytes.chunks(PIECE).map(<[u8]>::to_vec).collect();
    reqwest::Body::wrap_stream(futures_util::stream::iter(pieces.into_iter().map(
        move |p| {
            sent.fetch_add(p.len() as u64, Ordering::Relaxed);
            Ok::<_, std::io::Error>(p)
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A slow upload goes on for as long as its bytes keep moving, far past a minute; one
    /// whose bytes stop is given up a minute after the last; one with every byte in is given
    /// Apple's five minutes to answer, then given up.
    #[tokio::test(start_paused = true)]
    async fn an_upload_is_given_up_only_when_it_stops() {
        // Ten minutes, a piece every ten seconds: never given up.
        let sent = Arc::new(AtomicU64::new(0));
        let slow = {
            let sent = Arc::clone(&sent);
            async move {
                for _ in 0..60 {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    sent.fetch_add(1, Ordering::Relaxed);
                }
                "answered"
            }
        };
        assert_eq!(
            watched(slow, &sent, 60, STALL, ANSWER).await.unwrap(),
            "answered"
        );

        // Half sent, then nothing: given up a minute after the last byte, not before.
        let sent = AtomicU64::new(30);
        let started = tokio::time::Instant::now();
        let stuck = watched(std::future::pending::<()>(), &sent, 60, STALL, ANSWER).await;
        assert!(matches!(stuck, Err(Error::Stalled)));
        let after = started.elapsed();
        assert!(after > STALL && after <= STALL + LOOK * 2, "{after:?}");

        // Every byte in, no answer: five minutes, then given up.
        let sent = AtomicU64::new(60);
        let started = tokio::time::Instant::now();
        let unanswered = watched(std::future::pending::<()>(), &sent, 60, STALL, ANSWER).await;
        assert!(matches!(unanswered, Err(Error::Stalled)));
        assert!(started.elapsed() > ANSWER);
    }

    /// A local server taking an upload at `rate` bytes a second, or stopping after `stop`
    /// bytes and reading nothing more: answers 200 once it has read `size`.
    async fn server(size: usize, rate: usize, stop: Option<usize>) -> std::net::SocketAddr {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; rate / 10];
            let mut got = 0;
            while got < size {
                if stop.is_some_and(|s| got >= s) {
                    std::future::pending::<()>().await;
                }
                match socket.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => got += n,
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 4\r\n\r\ntook")
                .await;
        });
        addr
    }

    fn post(
        addr: std::net::SocketAddr,
        size: usize,
        sent: &Arc<AtomicU64>,
    ) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .post(format!("http://{addr}/upload"))
            .header("content-length", size)
            .body(counted(vec![7u8; size], Arc::clone(sent)))
    }

    /// On a real connection: an upload far slower than the stall limit goes through whole for
    /// as long as its bytes move (here three times the limit), and one whose receiver stops
    /// reading part way is given up as stalled once the connection's buffers are full.
    #[tokio::test]
    async fn on_a_real_connection_only_a_stalled_upload_is_given_up() {
        let stall = Duration::from_secs(2);
        // 12 MB at 2 MB a second: six seconds, past the limit three times over.
        let size = 12 << 20;
        let addr = server(size, 2 << 20, None).await;
        let sent = Arc::new(AtomicU64::new(0));
        let started = std::time::Instant::now();
        let (status, text) = send_within(post(addr, size, &sent), &sent, size as u64, stall, stall)
            .await
            .unwrap();
        assert_eq!((status.as_u16(), text.as_str()), (200, "took"));
        assert!(started.elapsed() > stall * 2, "{:?}", started.elapsed());

        // 64 MB, the receiver stopping after 4: given up, well short of the whole.
        let size = 64 << 20;
        let addr = server(size, 8 << 20, Some(4 << 20)).await;
        let sent = Arc::new(AtomicU64::new(0));
        let stuck = send_within(post(addr, size, &sent), &sent, size as u64, stall, stall).await;
        assert!(matches!(stuck, Err(Error::Stalled)), "{stuck:?}");
        assert!(sent.load(Ordering::Relaxed) < size as u64);
    }
}
