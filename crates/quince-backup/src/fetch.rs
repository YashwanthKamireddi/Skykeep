//! Files from iCloud streamed to disk whole or not at all. Moved from the desktop app
//! (photos.rs, main.rs) unchanged, so the backup engine can use them outside it.

/// Why a file from iCloud didn't arrive: Apple's answer when it gave one, or the disk's
/// trouble, so a caller can tell one photo's problem from an outage.
#[derive(Debug)]
pub struct Fetch {
    /// The HTTP status Apple answered with; `None` when the connection broke or never formed.
    pub status: Option<u16>,
    /// The disk refused the write (full, gone, not ours to write).
    pub disk: Option<std::io::ErrorKind>,
    pub message: String,
}

impl From<Fetch> for String {
    fn from(f: Fetch) -> String {
        f.message
    }
}

pub fn disk(e: &std::io::Error) -> Fetch {
    Fetch {
        status: None,
        disk: Some(e.kind()),
        message: e.to_string(),
    }
}

/// Where [`stream_to_new_file`] writes `path` until it is whole: `IMG_1.JPG.part`.
pub fn part_of(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension(format!(
        "{}.part",
        path.extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default()
    ))
}

/// A file from iCloud written to `path`, which must not exist: streamed into a `.part` beside
/// it (a large video never sits in memory) and named only once whole, so a broken download
/// leaves no half file under the real name. Answers how many bytes it holds.
pub async fn stream_to_new_file(
    http: &reqwest::Client,
    url: &str,
    path: &std::path::Path,
) -> Result<u64, Fetch> {
    use quince_icloud::download::{open, Opened};
    let part = part_of(path);
    let failed = |e: quince_icloud::Error| match e {
        quince_icloud::Error::Io(io) => disk(&io),
        quince_icloud::Error::Network(n) => Fetch {
            status: n.status().map(|s| s.as_u16()),
            disk: None,
            message: format!("The download broke off: {n}"),
        },
        other => Fetch {
            status: None,
            disk: None,
            message: other.to_string(),
        },
    };
    // Every byte Apple said it has, carried on from where it stopped if the connection drops
    // or iCloud sends it in parts (quince_icloud::download).
    let download = match open(http, url).await.map_err(failed)? {
        Opened::Sending(d) => d,
        Opened::Refused(status) => {
            return Err(Fetch {
                status: Some(status),
                disk: None,
                message: format!("iCloud did not send the file ({status})"),
            });
        }
    };
    let len = match download.save(&part, |_| {}).await {
        Ok(len) => len,
        Err(e) => {
            tokio::fs::remove_file(&part).await.ok();
            return Err(failed(e));
        }
    };
    move_into_place(&part, path).await.map_err(|e| disk(&e))?;
    Ok(len)
}

/// A file just written renamed to its own name, waiting while Windows holds it — the virus
/// scanner looks at each new file as it is closed, and a rename meanwhile is refused
/// (quince_core::files). Off the async threads: the wait is a sleep.
pub async fn move_into_place(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let (from, to) = (from.to_path_buf(), to.to_path_buf());
    tokio::task::spawn_blocking(move || quince_core::files::rename(&from, &to))
        .await
        .map_err(std::io::Error::other)?
}
