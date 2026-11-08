//! Downloads from Apple, a few at a time, each file once.
//!
//! A first look at a large library asks for every visible picture at once: a grid on two big
//! monitors, scrolled, is hundreds of requests through the `quince` scheme, and each used to
//! start its own download straight away. reqwest speaks HTTP/1.1 here, so each was a socket of
//! its own, and with the file written beside it the process ran out of file descriptors (a
//! desktop session's soft limit is 1024). Out of descriptors, GTK3's next frame on Wayland
//! could not make its shared-memory buffer and dereferenced the NULL pool it got back — a
//! segfault in `wl_proxy_get_version` under `gdk_window_begin_draw_frame` (issue #17).
//!
//! [`Gate`] bounds how many downloads run at once, and a second ask for a file already being
//! fetched waits for that one instead of starting another. Whatever ends a download — done,
//! failed, or the request dropped part way — its turn and its claim go back at once.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokio::sync::{Notify, Semaphore, SemaphorePermit};

/// Downloads at once, across the whole app: more than a browser's six to one host, few
/// enough that sockets and files stay a small fraction of any limit.
pub const AT_ONCE: usize = 12;

pub struct Gate {
    turns: Semaphore,
    going: Mutex<Option<HashSet<PathBuf>>>,
    ended: Notify,
}

/// A file's download, held: its claim and its turn go back when dropped.
pub struct Turn<'a> {
    gate: &'a Gate,
    path: PathBuf,
    _turn: SemaphorePermit<'a>,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        if let Some(going) = self
            .gate
            .going
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            going.remove(&self.path);
        }
        self.gate.ended.notify_waiters();
    }
}

impl Gate {
    pub const fn new(at_once: usize) -> Self {
        Self {
            turns: Semaphore::const_new(at_once),
            going: Mutex::new(None),
            ended: Notify::const_new(),
        }
    }

    /// A turn to download into `path`: once no other download of it is running and one of
    /// the turns is free. `None` when the file is there by then — another download made it.
    pub async fn turn(&self, path: &Path) -> Option<Turn<'_>> {
        // Claimed first, so two asks for one file never both download it.
        loop {
            // Asked to be told before looking, so an ending between the look and the wait is
            // never missed.
            let ended = self.ended.notified();
            tokio::pin!(ended);
            ended.as_mut().enable();
            {
                let mut going = self
                    .going
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let going = going.get_or_insert_with(HashSet::new);
                if !going.contains(path) {
                    going.insert(path.to_path_buf());
                    break;
                }
            }
            ended.await;
            if path.exists() {
                return None;
            }
        }
        // Claimed: a dropped wait from here gives the claim back (the guard owns it).
        let claim = Claim { gate: self, path };
        let permit = self.turns.acquire().await.ok()?;
        std::mem::forget(claim);
        if path.exists() {
            // Fetched while this waited its turn (by an ask that came and went first).
            drop(Turn {
                gate: self,
                path: path.to_path_buf(),
                _turn: permit,
            });
            return None;
        }
        Some(Turn {
            gate: self,
            path: path.to_path_buf(),
            _turn: permit,
        })
    }

    #[cfg(test)]
    fn claimed(&self) -> usize {
        self.going.lock().unwrap().as_ref().map_or(0, HashSet::len)
    }
}

/// A claim not yet given a turn: given back if the wait for one is dropped.
struct Claim<'a> {
    gate: &'a Gate,
    path: &'a Path,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if let Some(going) = self
            .gate
            .going
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            going.remove(self.path);
        }
        self.gate.ended.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn never_more_than_its_turns_at_once() {
        static GATE: Gate = Gate::new(3);
        let dir = tempfile::tempdir().unwrap();
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let mut all = Vec::new();
        for n in 0..40 {
            let path = dir.path().join(format!("{n}"));
            let (running, most) = (running.clone(), most.clone());
            all.push(tokio::spawn(async move {
                let _turn = GATE.turn(&path).await.expect("a turn");
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                most.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                running.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for t in all {
            t.await.unwrap();
        }
        assert!(most.load(Ordering::SeqCst) <= 3, "at most three at once");
        assert_eq!(GATE.claimed(), 0, "every claim given back");
    }

    #[tokio::test]
    async fn one_file_is_fetched_once_however_many_ask() {
        static GATE: Gate = Gate::new(4);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo");
        let fetched = Arc::new(AtomicUsize::new(0));
        let mut all = Vec::new();
        for _ in 0..10 {
            let (path, fetched) = (path.clone(), fetched.clone());
            all.push(tokio::spawn(async move {
                if let Some(_turn) = GATE.turn(&path).await {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    std::fs::write(&path, b"picture").unwrap();
                    fetched.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for t in all {
            t.await.unwrap();
        }
        assert_eq!(fetched.load(Ordering::SeqCst), 1, "downloaded once");
        assert_eq!(GATE.claimed(), 0);
    }

    #[tokio::test]
    async fn a_request_dropped_while_waiting_gives_its_claim_back() {
        static GATE: Gate = Gate::new(1);
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let held = GATE.turn(&a).await.expect("a turn");
        // `b` claims and waits for the one turn; dropped before it gets it.
        let waiting = tokio::time::timeout(Duration::from_millis(20), GATE.turn(&b)).await;
        assert!(waiting.is_err(), "it was still waiting");
        drop(held);
        assert_eq!(GATE.claimed(), 0, "the dropped wait's claim went back");
        assert!(GATE.turn(&b).await.is_some(), "and `b` can be fetched");
    }

    #[tokio::test]
    async fn a_failed_download_lets_the_next_ask_try() {
        static GATE: Gate = Gate::new(2);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo");
        {
            let _turn = GATE.turn(&path).await.expect("a turn");
            // Failed: nothing written.
        }
        assert!(GATE.turn(&path).await.is_some(), "tried again");
    }
}
