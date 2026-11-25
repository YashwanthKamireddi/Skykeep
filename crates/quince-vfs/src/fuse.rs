//! The engine as a Linux folder, through FUSE (fuser).
//!
//! Names, sizes and dates come from the library, never the network, so a file manager
//! browsing is never kept waiting; a folder never opened before is fetched once. A file's
//! bytes come from the cache, each part fetched from Apple the first time it is read.
//! Folders, renames, moves and deletes are made at Apple as they happen. A file's own bytes
//! are written here first and sent behind the program writing them (see writes.rs).

use crate::writes::{self, Pending, Writes};
use crate::{is_thumbnailer, named, Cache, Inodes, Remote, Store};
use fuser::{
    BsdFileFlags, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenAccMode, OpenFlags, RenameFlags, ReplyAttr,
    ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs,
    ReplyWrite, ReplyXattr, Request, TimeOrNow, WriteFlags,
};
use quince_core::drive::DriveItem;
use std::ffi::OsStr;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long the kernel may keep what it was told about a name or a file. Short, as rclone's
/// is: a file changed on another device must not be read with its old length.
/// This computer's own disk refusing, told to the app as the disk told it — "No space left on
/// device" when it is full, never a vague input/output error that reads as a broken drive.
fn disk(e: std::io::Error) -> Errno {
    e.raw_os_error().map_or(Errno::EIO, Errno::from_i32)
}

const TTL: Duration = Duration::from_secs(1);

/// How long a folder never opened before may take to come from Apple before the file manager
/// is told it could not be read (and can ask again).
const LIST_WAIT: Duration = Duration::from_secs(30);
/// How long a folder's listing is taken as current before a look at it asks Apple again (in
/// the background: what is stored is shown at once). Apple pushes nothing for iCloud Drive.
const FRESH: Duration = Duration::from_secs(10);

/// How long a change (a new folder, a rename, a delete) may take at Apple.
const CHANGE_WAIT: Duration = Duration::from_secs(60);

/// The longest wait between tries of a send Apple would not take.
const LONGEST_RETRY: Duration = Duration::from_secs(300);

/// What the folder's threads and its sends share.
struct Inner {
    remote: Arc<dyn Remote>,
    store: Arc<dyn Store>,
    cache: Arc<Cache>,
    inodes: Mutex<Inodes>,
    rt: tokio::runtime::Handle,
    uid: u32,
    gid: u32,
    mounted: SystemTime,
    /// Where the folder is mounted and the thumbnail cache's root, when thumbnails are wanted.
    thumbs: Option<(std::path::PathBuf, std::path::PathBuf)>,
    /// Folders whose thumbnails were written, by their items' versions: written once.
    filled: Mutex<std::collections::HashSet<String>>,
    /// Files written here and not yet at Apple as written. `None`: the folder is read-only.
    writes: Option<Arc<Writes>>,
    /// A walk of what is kept is under way; asked again meanwhile, it walks once more after.
    keeping: std::sync::atomic::AtomicBool,
    again: std::sync::atomic::AtomicBool,
    /// When each folder was last asked of Apple, by id.
    looked: Mutex<std::collections::HashMap<String, std::time::Instant>>,
    /// The account's storage as last asked, `(used, total)`.
    space: Mutex<Option<((u64, u64), std::time::Instant)>>,
    /// Each folder's listing as last worked out, by the folder's inode, kept while nothing it
    /// was worked out from has moved (see `Listing`).
    listings: Mutex<std::collections::HashMap<u64, Arc<Listing>>>,
    /// Nothing in it can be changed.
    read_only: bool,
    /// New files can be added and anything deleted, nothing else (the Photos folder: a photo is
    /// added or deleted, never rewritten in place, renamed or given a folder).
    add_only: bool,
}

pub struct DriveFs(Arc<Inner>);

/// What a pending file in a folder brings to its listing: its inode, its name, and the file of
/// Apple's it takes the place of.
type LocalKey = (u64, String, Option<DriveItem>);

/// A folder's listing — names, inodes and items — and everything it was worked out from: the
/// store's version, the inode table's generation and the files being written into it. While all
/// three stand, working it out again gives the very same listing, so it is used as it is.
///
/// A file manager listing a folder looks every name in it up after, and each look-up worked the
/// whole folder out again — read from the library, named, placed — to find one name: a folder of
/// two thousand cost four million rows, every second the kernel's answer lasted.
struct Listing {
    version: u64,
    generation: u64,
    local: Vec<LocalKey>,
    entries: Vec<(String, u64, DriveItem)>,
    /// Where in `entries` each name is first.
    by_name: std::collections::HashMap<String, usize>,
}

impl Listing {
    fn of(entries: Vec<(String, u64, DriveItem)>) -> Self {
        let mut by_name = std::collections::HashMap::with_capacity(entries.len());
        for (at, (name, ..)) in entries.iter().enumerate() {
            by_name.entry(name.clone()).or_insert(at);
        }
        Self {
            version: 0,
            generation: 0,
            local: Vec::new(),
            entries,
            by_name,
        }
    }
}

/// The most folders whose listings are kept at once.
const KEPT_LISTINGS: usize = 256;

/// The attribute that keeps a file or folder on this computer: `setfattr -n user.quince.keep
/// -v 1 <path>` keeps it, `-x` lets it go — what the window's "Keep on This Computer" does.
const KEEP_ATTR: &str = "user.quince.keep";

/// What is kept on this computer, for the app (the window's "Keep on This Computer").
#[derive(Clone)]
pub struct Keeper(Arc<Inner>);

impl Keeper {
    /// A file or folder kept on this computer, or let go of: kept, its bytes (every file in it,
    /// for a folder, and any added later) are fetched behind and never trimmed.
    pub fn set(&self, id: &str, on: bool) {
        Inner::keep(&self.0, id, on);
    }

    /// The files and folders chosen to be kept.
    #[must_use]
    pub fn chosen(&self) -> Vec<String> {
        self.0.cache.kept().chosen.into_iter().collect()
    }

    /// Everything kept: what was chosen, and every file inside a chosen folder.
    #[must_use]
    pub fn kept(&self) -> Vec<String> {
        let k = self.0.cache.kept();
        k.chosen.into_iter().chain(k.within).collect()
    }
}

impl DriveFs {
    #[must_use]
    pub fn new(
        remote: Arc<dyn Remote>,
        store: Arc<dyn Store>,
        cache: Arc<Cache>,
        rt: tokio::runtime::Handle,
    ) -> Self {
        // SAFETY: getuid and getgid cannot fail and touch no memory.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        Self(Arc::new(Inner {
            remote,
            store,
            cache,
            inodes: Mutex::new(Inodes::new()),
            rt,
            uid,
            gid,
            mounted: SystemTime::now(),
            thumbs: None,
            filled: Mutex::new(std::collections::HashSet::new()),
            writes: None,
            space: Mutex::new(None),
            looked: Mutex::new(std::collections::HashMap::new()),
            listings: Mutex::new(std::collections::HashMap::new()),
            keeping: std::sync::atomic::AtomicBool::new(false),
            again: std::sync::atomic::AtomicBool::new(false),
            read_only: false,
            add_only: false,
        }))
    }

    fn inner_mut(&mut self) -> &mut Inner {
        Arc::get_mut(&mut self.0).expect("configured before it is mounted")
    }

    /// Apple's pictures of each folder listed put in the thumbnail cache (the crate's
    /// write_thumbnails), for a folder mounted at `mount`.
    #[must_use]
    pub fn with_thumbnails(mut self, mount: std::path::PathBuf, cache: std::path::PathBuf) -> Self {
        self.inner_mut().thumbs = Some((mount, cache));
        self
    }

    /// What is kept on this computer, for the app to change and show.
    #[must_use]
    pub fn keeper(&self) -> Keeper {
        Keeper(Arc::clone(&self.0))
    }

    /// What is written here and not yet at Apple, for the app to show (before mounting).
    #[must_use]
    pub fn writes(&self) -> Option<Arc<Writes>> {
        self.0.writes.clone()
    }

    /// Nothing in it can be changed: every change answered "read-only file system".
    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.inner_mut().read_only = true;
        self
    }

    /// New files can be added (into folders the remote `accepts`) and files deleted; nothing
    /// is rewritten, renamed, moved or made a folder — "read-only file system" to all of that.
    #[must_use]
    pub fn add_only(mut self) -> Self {
        self.inner_mut().add_only = true;
        self
    }

    /// Files can be written, their bytes kept in `dir` until Apple has them. What was waiting
    /// when the folder last went away is picked up again, and sent once the folder is mounted
    /// (see `mount`): a sender started here would hold the folder's state, and every setting
    /// made after this one would find it shared (#253).
    ///
    /// # Errors
    /// When the folder for the bytes cannot be made.
    pub fn with_writes(mut self, dir: &Path) -> std::io::Result<Self> {
        let writes = Arc::new(Writes::new(dir)?);
        self.inner_mut().writes = Some(Arc::clone(&writes));
        // Each to the inode it has now (see `Writes::take_back`: never moved straight across).
        let waiting = writes.take_back(|pending| {
            self.0
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .local(&pending.folder, &pending.name)
        })?;
        for (ino, pending) in waiting {
            writes
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    ino,
                    Pending {
                        dirty: true,
                        ..pending
                    },
                );
        }
        writes.save();
        Ok(self)
    }
}

impl Inner {
    /// What was waiting when the folder last went away (taken back by `with_writes`), each
    /// sent a moment after the folder comes up.
    fn send_waiting(me: &Arc<Self>) {
        let Some(writes) = me.writes.as_ref() else {
            return;
        };
        let waiting: Vec<u64> = writes
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|(_, p)| p.dirty)
            .map(|(ino, _)| *ino)
            .collect();
        for ino in waiting {
            Self::schedule(me, ino, Duration::from_millis(500));
        }
    }

    /// In the background, once per folder version: the folder's thumbnails written.
    fn fill_thumbnails(&self, folder: &DriveItem, files: &[(String, u64, DriveItem)]) {
        let Some((mount, cache)) = self.thumbs.clone() else {
            return;
        };
        let mut paths = Vec::new();
        let mut fingerprint = folder.drivewsid.clone();
        {
            let inodes = self
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (_, ino, item) in files
                .iter()
                .filter(|(_, _, i)| !i.is_container() && !i.drivewsid.starts_with("LOCAL::"))
            {
                if let Some(p) = inodes.path(*ino) {
                    paths.push((p.to_string(), item.clone()));
                    fingerprint.push_str(item.etag.as_deref().unwrap_or(""));
                }
            }
        }
        if paths.is_empty()
            || !self
                .filled
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(fingerprint)
        {
            return;
        }
        let (remote, id) = (Arc::clone(&self.remote), folder.drivewsid.clone());
        self.rt.spawn(async move {
            let Ok(pictures) = remote.thumbnails(&id).await else {
                return;
            };
            if !pictures.is_empty() {
                let _ = tokio::task::spawn_blocking(move || {
                    crate::write_thumbnails(&mount, &cache, &paths, &pictures)
                })
                .await;
            }
        });
    }

    /// A change made at Apple, waited for (the file manager waits on it too), then the folders
    /// it touched read again so the library — and so the next listing — is Apple's.
    fn change<T: Send + 'static>(
        &self,
        work: crate::Fut<T>,
        touched: &[&DriveItem],
    ) -> Result<T, Errno> {
        let done = self
            .rt
            .block_on(async { tokio::time::timeout(CHANGE_WAIT, work).await })
            .map_err(|_| Errno::ETIMEDOUT)?
            .map_err(|e| {
                quiet_note(&e);
                Errno::EIO
            })?;
        for folder in touched {
            self.relist(&folder.drivewsid);
        }
        Ok(done)
    }

    /// Whether an item is kept on this computer: chosen, inside a chosen folder (as far up as
    /// its folders are known), or noted as inside one by the last walk.
    fn kept_under(&self, item: &DriveItem) -> bool {
        if self.cache.is_kept(&item.drivewsid) {
            return true;
        }
        let chosen = self.cache.kept().chosen;
        let inodes = self
            .inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut parent = item.parent_id.clone();
        for _ in 0..64 {
            let Some(p) = parent else { break };
            if chosen.contains(&p) {
                return true;
            }
            parent = inodes
                .ino_of(&p)
                .and_then(|i| inodes.item(i))
                .and_then(|it| it.parent_id.clone());
        }
        false
    }

    fn keep(me: &Arc<Self>, id: &str, on: bool) {
        me.cache.set_kept(|k| {
            if on {
                k.chosen.insert(id.to_string());
            } else {
                k.chosen.remove(id);
            }
        });
        Self::settle_kept(me);
    }

    /// A folder's items for a walk: as stored, or asked of Apple when never listed.
    fn items_of(&self, folder: &str) -> Vec<DriveItem> {
        if let Some(stored) = self.store.children(folder).filter(|s| !s.is_empty()) {
            return stored;
        }
        let (remote, id) = (Arc::clone(&self.remote), folder.to_string());
        self.rt
            .block_on(async move { tokio::time::timeout(LIST_WAIT, remote.list(&id)).await })
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default()
    }

    fn walk(&self, folder: &str, files: &mut Vec<DriveItem>, depth: u32) {
        if depth > 64 {
            return;
        }
        for item in self.items_of(folder) {
            if item.is_container() {
                self.walk(&item.drivewsid, files, depth + 1);
            } else {
                files.push(item);
            }
        }
    }

    /// Everything kept made whole, behind everything else: each chosen folder walked for the
    /// files in it (noted, so no trim lets them go), and every kept file's missing parts
    /// fetched. One walk at a time; asked again meanwhile, it walks once more after. Run when
    /// the folder comes up, when what is kept changes, and after a kept folder is looked at
    /// (a file added on another device is kept too).
    fn settle_kept(me: &Arc<Self>) {
        use std::sync::atomic::Ordering;
        if me.cache.kept().chosen.is_empty() {
            return;
        }
        if me.keeping.swap(true, Ordering::SeqCst) {
            me.again.store(true, Ordering::SeqCst);
            return;
        }
        let me = Arc::clone(me);
        std::thread::spawn(move || loop {
            me.again.store(false, Ordering::SeqCst);
            let chosen = me.cache.kept().chosen;
            // Files inside a chosen folder (noted as such), and files chosen themselves (kept by
            // being chosen, never noted as inside: let go of, they must not stay kept).
            let mut files = Vec::new();
            let mut alone = Vec::new();
            let mut unknown = false;
            for id in &chosen {
                if id.starts_with("FOLDER::") {
                    me.walk(id, &mut files, 0);
                } else {
                    let found = {
                        let inodes = me
                            .inodes
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        inodes.ino_of(id).and_then(|i| inodes.item(i)).cloned()
                    };
                    match found {
                        Some(item) => alone.push(item),
                        // Not seen yet this run: fetched once its folder is looked at.
                        None => unknown = true,
                    }
                }
            }
            let within: std::collections::BTreeSet<String> =
                files.iter().map(|f| f.drivewsid.clone()).collect();
            me.cache.set_kept(|k| {
                if unknown {
                    k.within.extend(within);
                } else {
                    k.within = within;
                }
            });
            for file in files.into_iter().chain(alone) {
                if me.cache.whole(&file) {
                    continue;
                }
                let (cache, remote) = (Arc::clone(&me.cache), Arc::clone(&me.remote));
                let _ = me
                    .rt
                    .block_on(async move { cache.fetch_whole(remote.as_ref(), &file).await });
            }
            me.keeping.store(false, Ordering::SeqCst);
            if !(me.again.swap(false, Ordering::SeqCst) && !me.keeping.swap(true, Ordering::SeqCst))
            {
                break;
            }
        });
    }

    /// Whether a folder was asked of Apple within `FRESH`; if not, it is marked asked now (so
    /// many looks at once ask once).
    fn stale(&self, folder: &str) -> bool {
        let mut looked = self
            .looked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = std::time::Instant::now();
        if looked
            .get(folder)
            .is_some_and(|at| now.duration_since(*at) < FRESH)
        {
            return false;
        }
        looked.insert(folder.to_string(), now);
        true
    }

    /// A folder looked at: shown from what is stored, and asked of Apple behind that when its
    /// listing is older than `FRESH` — a file saved on the phone is there on the next look.
    fn revalidate(me: &Arc<Self>, folder: &str) {
        if me.stale(folder) {
            let (remote, id) = (Arc::clone(&me.remote), folder.to_string());
            // A kept folder walked again after: what was added on another device is kept too.
            let kept = me.cache.kept().chosen.contains(folder)
                || me.item_by_id(folder).is_some_and(|f| me.kept_under(&f));
            let again = Arc::clone(me);
            me.rt.spawn(async move {
                let _ = tokio::time::timeout(LIST_WAIT, remote.list(&id)).await;
                if kept {
                    Self::settle_kept(&again);
                }
            });
        }
    }

    fn item_by_id(&self, id: &str) -> Option<DriveItem> {
        let inodes = self
            .inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inodes.ino_of(id).and_then(|i| inodes.item(i)).cloned()
    }

    fn relist(&self, folder: &str) {
        let (remote, id) = (Arc::clone(&self.remote), folder.to_string());
        let _ = self
            .rt
            .block_on(async move { tokio::time::timeout(LIST_WAIT, remote.list(&id)).await });
    }

    /// A folder by its id, found in what the store holds by walking down from the top: for a
    /// folder the table has not met since a restart.
    fn find_folder(&self, id: &str) -> Option<DriveItem> {
        let root = self.item(1)?;
        if root.drivewsid == id {
            return Some(root);
        }
        let mut queue = std::collections::VecDeque::from([root.drivewsid]);
        let mut seen = 0usize;
        while let Some(at) = queue.pop_front() {
            for child in self.store.children(&at).unwrap_or_default() {
                if !child.is_container() {
                    continue;
                }
                if child.drivewsid == id {
                    return Some(child);
                }
                seen += 1;
                if seen > 20_000 {
                    return None;
                }
                queue.push_back(child.drivewsid);
            }
        }
        None
    }

    fn item(&self, ino: u64) -> Option<DriveItem> {
        self.inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .item(ino)
            .cloned()
    }

    fn pending(&self, ino: u64) -> Option<Pending> {
        self.writes
            .as_ref()?
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&ino)
            .cloned()
    }

    fn with_pending<R>(&self, ino: u64, f: impl FnOnce(&mut Pending) -> R) -> Option<R> {
        let writes = self.writes.as_ref()?;
        let mut items = writes
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.get_mut(&ino).map(f)
    }

    fn attr(&self, ino: u64, item: &DriveItem) -> FileAttr {
        // Apple lists no dates for a folder: it shows when the folder was mounted rather than
        // 1 January 1970, which a file manager sorts and labels as if it were real.
        let mounted = self.mounted;
        let secs = |s: Option<i64>| {
            s.filter(|&s| s > 0).map_or(mounted, |s| {
                UNIX_EPOCH + Duration::from_secs(u64::try_from(s).unwrap_or(0))
            })
        };
        let dir = item.is_container();
        // A file's length as served once that is known (a Pages document's is not its
        // listing's), else as listed.
        let (mut size, mut mtime) = (
            if dir { 0 } else { self.cache.size(item) },
            secs(item.modified_at),
        );
        // A file being written shows what has been written, as of when.
        if let (Some(p), Some(w)) = (self.pending(ino), self.writes.as_ref()) {
            size = std::fs::metadata(w.staging(ino, &p.name)).map_or(size, |m| m.len());
            mtime = p.modified;
        }
        FileAttr {
            ino: INodeNo(ino),
            size,
            blocks: size.div_ceil(512),
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: secs(item.created_at.or(item.modified_at)),
            kind: if dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            // Someone else's share, read only until shares are supported: said in its modes too,
            // so a file manager greys out what it would only be refused.
            perm: match (dir, self.writes.is_some() && !self.read_only) {
                (true, _) if self.read_only || item.shared_with_me() => 0o555,
                (false, _) if item.shared_with_me() => 0o444,
                (true, _) => 0o755,
                (false, true) if !self.add_only => 0o644,
                (false, true) => 0o444,
                (false, false) => 0o444,
            },
            nlink: if dir { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    /// A folder's items with their names and inodes: from the library — or, never opened
    /// before, from Apple once — with the files being written here among them.
    fn children(&self, ino: u64) -> Result<Vec<(String, u64, DriveItem)>, Errno> {
        Ok(self.listing(ino)?.entries.clone())
    }

    /// The folder's listing: the one worked out last while nothing it came from has moved,
    /// else worked out again (see `Listing`).
    fn listing(&self, ino: u64) -> Result<Arc<Listing>, Errno> {
        // Asked first: a change landing while this works the listing out moves it past what is
        // recorded here, and the next look works it out again.
        let version = self.store.version();
        let folder = self.item(ino).ok_or(Errno::ENOENT)?;
        if !folder.is_container() {
            return Err(Errno::ENOTDIR);
        }
        // Files being written here: a new one is shown beside Apple's; one taking another's
        // place is shown in it, under its own inode.
        let mut local: Vec<(u64, Pending)> = Vec::new();
        if let Some(w) = &self.writes {
            let pending = w
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            local = pending
                .iter()
                .filter(|(_, p)| p.folder == folder.drivewsid)
                .map(|(i, p)| (*i, p.clone()))
                .collect();
        }
        local.sort_by_key(|(i, _)| *i);
        let local_key: Vec<LocalKey> = local
            .iter()
            .map(|(i, p)| (*i, p.name.clone(), p.replaces.clone()))
            .collect();
        if let Some(version) = version {
            let generation = self
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .generation();
            let kept = self
                .listings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&ino)
                .cloned();
            if let Some(kept) = kept.filter(|l| {
                l.version == version && l.generation == generation && l.local == local_key
            }) {
                return Ok(kept);
            }
        }
        let stored = self.store.children(&folder.drivewsid);
        // A folder Apple says has things in it, with nothing stored, was never listed.
        let listed = stored.filter(|s| !s.is_empty() || folder.child_count == Some(0));
        let from_store = listed.is_some();
        let mut items = match listed {
            Some(items) => items,
            None => {
                let remote = Arc::clone(&self.remote);
                let id = folder.drivewsid.clone();
                self.rt
                    .block_on(
                        async move { tokio::time::timeout(LIST_WAIT, remote.list(&id)).await },
                    )
                    .map_err(|_| Errno::ETIMEDOUT)?
                    .map_err(|_| Errno::EIO)?
            }
        };
        let mut inodes = self
            .inodes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (pino, p) in &local {
            if let Some(r) = &p.replaces {
                items.retain(|i| i.drivewsid != r.drivewsid);
            }
            // Its own inode keeps the name it is being written under.
            if let Some(it) = inodes.item(*pino).cloned() {
                if it.drivewsid.starts_with("LOCAL::")
                    || p.replaces
                        .as_ref()
                        .is_some_and(|r| r.drivewsid == it.drivewsid)
                {
                    items.retain(|i| i.drivewsid != it.drivewsid);
                }
            }
        }
        let mut out: Vec<(String, u64, DriveItem)> = named(&items)
            .into_iter()
            .map(|(name, it)| {
                let child = inodes.ino(&it);
                inodes.place(child, ino, &name);
                (name, child, it)
            })
            .collect();
        for (pino, p) in local {
            let Some(it) = inodes.item(pino).cloned().or_else(|| p.replaces.clone()) else {
                continue;
            };
            inodes.place(pino, ino, &p.name);
            out.push((p.name.clone(), pino, it));
        }
        let generation = inodes.generation();
        drop(inodes);
        let mut listing = Listing::of(out);
        // Kept only when it came from the store: one read from Apple was stored on the way, which
        // moves the store's version, and is worked out again from there next time.
        if let (Some(version), true) = (version, from_store) {
            listing.version = version;
            listing.generation = generation;
            listing.local = local_key;
            let listing = Arc::new(listing);
            let mut kept = self
                .listings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if kept.len() >= KEPT_LISTINGS && !kept.contains_key(&ino) {
                kept.clear();
            }
            kept.insert(ino, Arc::clone(&listing));
            return Ok(listing);
        }
        Ok(Arc::new(listing))
    }

    /// The item called `name` in folder `parent`.
    fn child(&self, parent: u64, name: &OsStr) -> Result<(u64, DriveItem), Errno> {
        let listing = self.listing(parent)?;
        let at = *name
            .to_str()
            .and_then(|n| listing.by_name.get(n))
            .ok_or(Errno::ENOENT)?;
        let (_, ino, item) = &listing.entries[at];
        Ok((*ino, item.clone()))
    }

    /// A file of Apple's made writable: its bytes copied here (unless they are about to be
    /// thrown away), to be sent back in its place.
    fn begin_writing(&self, ino: u64, item: &DriveItem, keep: bool) -> Result<(), Errno> {
        if self.add_only {
            return Err(Errno::EROFS);
        }
        if item.shared_with_me() {
            return Err(Errno::EACCES);
        }
        let writes = self.writes.as_ref().ok_or(Errno::EROFS)?;
        if self.pending(ino).is_some() {
            return Ok(());
        }
        let name = {
            let inodes = self
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            inodes
                .path(ino)
                .and_then(|p| p.rsplit('/').next())
                .map_or_else(|| item.display_name(), str::to_string)
        };
        let staged = writes.staging(ino, &name);
        std::fs::create_dir_all(staged.parent().ok_or(Errno::EIO)?).map_err(disk)?;
        let file = std::fs::File::create(&staged).map_err(disk)?;
        if keep {
            let (cache, remote, it) = (
                Arc::clone(&self.cache),
                Arc::clone(&self.remote),
                item.clone(),
            );
            // All of it, at its length as served: copied at the listing's, a Keynote document
            // was cut short or padded, and sent back so.
            let size = {
                let (c, r, i) = (Arc::clone(&cache), Arc::clone(&remote), it.clone());
                self.rt
                    .block_on(async move { c.sized(r.as_ref(), &i).await })
                    .map_err(|_| Errno::EIO)?
            };
            let mut at = 0;
            while at < size {
                let len = crate::CHUNK.min(size - at);
                let (c, r, i) = (Arc::clone(&cache), Arc::clone(&remote), it.clone());
                let bytes = self
                    .rt
                    .block_on(async move { c.read(r.as_ref(), &i, at, len).await })
                    .map_err(|_| Errno::EIO)?;
                file.write_all_at(&bytes, at).map_err(disk)?;
                at += len;
            }
        }
        writes
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                ino,
                Pending {
                    folder: item
                        .parent_id
                        .clone()
                        .unwrap_or_else(|| crate::ROOT.to_string()),
                    name,
                    replaces: Some(item.clone()),
                    open: 0,
                    dirty: !keep,
                    generation: 0,
                    modified: SystemTime::now(),
                    sender: false,
                    quiet_since: None,
                    sending: false,
                    failure: None,
                },
            );
        Ok(())
    }

    /// Sent to Apple once left alone a moment (writes::SETTLE), and tried again until Apple
    /// takes it. Only the bytes that were sent are counted as sent.
    fn schedule(me: &Arc<Self>, ino: u64, after: Duration) {
        // One sender per file: a second close, or a read-back, never starts a second send of
        // the same bytes. Claimed under the same lock that counts who has it open.
        let claimed = me
            .with_pending(ino, |p| {
                p.quiet_since = Some(std::time::Instant::now());
                if p.sender {
                    false
                } else {
                    p.sender = true;
                    true
                }
            })
            .unwrap_or(false);
        if !claimed {
            return;
        }
        let me = Arc::clone(me);
        let rt = me.rt.clone();
        rt.spawn(async move {
            tokio::time::sleep(after).await;
            let mut wait = Duration::from_secs(5);
            loop {
                // Left alone long enough? Open again, or nothing to send: the sender stands
                // down, and the next close claims it again.
                loop {
                    let state = me.with_pending(ino, |p| {
                        if p.open > 0 || !p.dirty || writes::stays_local(&p.name) {
                            p.sender = false;
                            Some(false)
                        } else if p.quiet_since.is_some_and(|q| q.elapsed() < writes::SETTLE) {
                            None
                        } else {
                            Some(true)
                        }
                    });
                    match state {
                        None | Some(Some(false)) => return,
                        Some(Some(true)) => break,
                        Some(None) => tokio::time::sleep(Duration::from_millis(250)).await,
                    }
                }
                let Some(p) = me.pending(ino) else { return };
                let generation = p.generation;
                let Some(writes) = me.writes.clone() else {
                    return;
                };
                let folder = {
                    let inodes = me
                        .inodes
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    inodes
                        .ino_of(&p.folder)
                        .and_then(|f| inodes.item(f).cloned())
                };
                // A folder not browsed since a restart is not in the table yet: found in what
                // the library holds, from the top down. Not found at all, it is looked for
                // again later — given up, a file waiting there from before was never sent.
                let folder = folder.or_else(|| me.find_folder(&p.folder));
                let Some(folder) = folder else {
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(LONGEST_RETRY);
                    continue;
                };
                let key = format!("{}/{}", p.folder, p.name);
                // Behind any send of the same name still under way, so this one finds it
                // finished and replaces it as this computer's own.
                let lane = writes.lane(&key);
                let _turn = lane.lock().await;
                let ours = writes
                    .sent
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(&key)
                    .cloned();
                // Sent from a copy of the bytes as they are now: a write during the send
                // changes the file, never what is on its way.
                let out_dir = writes.staging(ino, &p.name).with_file_name(".out");
                let out = out_dir.join(&p.name);
                let copied = std::fs::create_dir_all(&out_dir).is_ok()
                    && std::fs::copy(writes.staging(ino, &p.name), &out).is_ok();
                me.with_pending(ino, |p| p.sending = true);
                let result = if copied {
                    // Where only adding is, a send is an addition and nothing else: never a
                    // replacement of a file found under the same name.
                    let r = if me.add_only {
                        me.remote
                            .upload(&folder, &out)
                            .await
                            .map(|item| writes::Sent {
                                item,
                                conflicted: false,
                            })
                    } else {
                        writes::send(
                            me.remote.as_ref(),
                            &folder,
                            &out,
                            p.replaces.as_ref(),
                            ours.as_ref(),
                        )
                        .await
                    };
                    let _ = std::fs::remove_dir_all(&out_dir);
                    r
                } else {
                    Err("the bytes could not be read".into())
                };
                me.with_pending(ino, |p| {
                    p.sending = false;
                    p.failure = result.as_ref().err().map(|e| e.chars().take(160).collect());
                });
                match result {
                    Ok(sent) => {
                        if !sent.conflicted {
                            // This computer's own, so its next save replaces it as such.
                            writes
                                .sent
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .insert(key, sent.item.clone());
                        }
                        // Deleted while on its way: what was just sent goes too, into Recently
                        // Deleted — kept, it came back after the delete had said it was gone.
                        if me.pending(ino).is_none() {
                            let _ = me.remote.delete(&sent.item).await;
                            let _ = me.remote.list(&folder.drivewsid).await;
                            return;
                        }
                        let _ = me.remote.list(&folder.drivewsid).await;
                        // Written again — or renamed, or deleted — while on its way, the wait for
                        // the listing included: decided and let go of in one step, so a save
                        // landing in that wait is never dropped with the staged copy.
                        // None: deleted in that wait.
                        let done: Option<bool> = match &me.writes {
                            None => Some(true),
                            Some(w) => {
                                let mut items = w
                                    .items
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                match items.get_mut(&ino) {
                                    Some(now) if now.generation != generation => {
                                        // What Apple has now is what the next send replaces.
                                        now.replaces = Some(sent.item.clone());
                                        Some(false)
                                    }
                                    Some(_) => {
                                        items.remove(&ino);
                                        Some(true)
                                    }
                                    None => None,
                                }
                            }
                        };
                        let Some(done) = done else {
                            let _ = me.remote.delete(&sent.item).await;
                            return;
                        };
                        if done {
                            if let Some(w) = &me.writes {
                                w.drop_staging(ino);
                                w.save();
                            }
                            if !sent.conflicted {
                                me.inodes
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .bind(ino, &sent.item);
                                // A save is written into the same file at Apple; should it
                                // ever come back under another id, a file chosen to be kept on
                                // this computer stays kept as its new self.
                                if let Some(old) = p.replaces.as_ref() {
                                    if old.drivewsid != sent.item.drivewsid
                                        && me.cache.kept().chosen.contains(&old.drivewsid)
                                    {
                                        me.cache.set_kept(|k| {
                                            k.chosen.remove(&old.drivewsid);
                                            k.chosen.insert(sent.item.drivewsid.clone());
                                        });
                                    }
                                }
                            }
                            return;
                        }
                    }
                    Err(e) => {
                        quiet_note(&e);
                        tokio::time::sleep(wait).await;
                        wait = (wait * 2).min(LONGEST_RETRY);
                    }
                }
            }
        });
    }

    /// A file or (an empty) folder deleted, into Apple's Recently Deleted. A file made here
    /// and never sent is simply let go of.
    fn remove(&self, parent: u64, name: &OsStr, folder: bool) -> Result<(), Errno> {
        let from = self.item(parent).ok_or(Errno::ENOENT)?;
        let (ino, item) = self.child(parent, name)?;
        // Someone else's share, or anything in it: never deleted from here (a mount has no
        // way to say what Apple would do to their copy too).
        if item.shared_with_me() || from.shared_with_me() {
            return Err(Errno::EACCES);
        }
        if folder != item.is_container() {
            return Err(if folder {
                Errno::ENOTDIR
            } else {
                Errno::EISDIR
            });
        }
        if let Some(p) = self.pending(ino) {
            if let Some(w) = &self.writes {
                w.items
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&ino);
                w.drop_staging(ino);
                w.save();
            }
            return match p.replaces {
                Some(old) => self.change(self.remote.delete(&old), &[&from]),
                None => Ok(()),
            };
        }
        // A folder goes only when empty, as on any disk: Apple would take everything in it,
        // and a file manager or `rm -r` empties it first, file by file.
        if folder && !self.children(ino).map(|c| c.is_empty()).unwrap_or(false) {
            return Err(Errno::ENOTEMPTY);
        }
        if folder {
            self.empty_at_apple(&item)?;
        }
        self.change(self.remote.delete(&item), &[&from])
    }

    /// A folder about to go is asked of Apple first, right before: empty here can be empty as
    /// this computer last listed it, and files put in it on another device since would go to
    /// Recently Deleted with it, with no word — Apple deletes a folder whole, against its
    /// newest version. Anything there now is "not empty"; not reaching Apple deletes nothing.
    fn empty_at_apple(&self, folder: &DriveItem) -> Result<(), Errno> {
        let remote = Arc::clone(&self.remote);
        let id = folder.drivewsid.clone();
        let there = self
            .rt
            .block_on(async move { tokio::time::timeout(LIST_WAIT, remote.list(&id)).await })
            .map_err(|_| Errno::ETIMEDOUT)?
            .map_err(|e| {
                quiet_note(&e);
                Errno::EIO
            })?;
        // Listed, what Apple has is kept (the app stores every listing): the next look finds
        // it there.
        if there.is_empty() {
            Ok(())
        } else {
            Err(Errno::ENOTEMPTY)
        }
    }

    /// An item moved and renamed at Apple: into `to`, then given `newname`.
    fn move_and_rename(
        &self,
        item: &DriveItem,
        from: &DriveItem,
        to: &DriveItem,
        name: &OsStr,
        newname: &str,
    ) -> Result<(), Errno> {
        if from.drivewsid != to.drivewsid {
            self.change(self.remote.move_to(item, to), &[from, to])?;
        }
        if name != OsStr::new(newname) {
            let moved = self
                .store
                .children(&to.drivewsid)
                .and_then(|c| c.into_iter().find(|i| i.drivewsid == item.drivewsid))
                .unwrap_or_else(|| item.clone());
            self.change(self.remote.rename(&moved, newname), &[to])?;
        }
        Ok(())
    }

    /// A folder made in iCloud Drive. A trash folder at the top is refused: allowed, "Move to
    /// Trash" in GNOME Files would make a hidden folder in iCloud Drive and move the file into
    /// it. Refused, GIO finds no trash here and Files offers to delete the file instead, which
    /// sends it to Recently Deleted, where a deleted file belongs.
    fn mkdir(&self, parent: u64, name: &OsStr) -> Result<(u64, DriveItem), Errno> {
        let folder = self.item(parent).ok_or(Errno::ENOENT)?;
        let name = name.to_str().ok_or(Errno::EINVAL)?;
        if (parent == 1 && writes::is_trash_folder(name)) || folder.shared_with_me() {
            return Err(Errno::EACCES);
        }
        if self.child(parent, OsStr::new(name)).is_ok() {
            return Err(Errno::EEXIST);
        }
        self.change(self.remote.mkdir(&folder, name), &[&folder])?;
        self.child(parent, OsStr::new(name))
    }

    fn rename(
        me: &Arc<Self>,
        parent: u64,
        name: &OsStr,
        newparent: u64,
        newname: &OsStr,
    ) -> Result<(), Errno> {
        let (from, to) = (
            me.item(parent).ok_or(Errno::ENOENT)?,
            me.item(newparent).ok_or(Errno::ENOENT)?,
        );
        let (ino, item) = me.child(parent, name)?;
        // Nothing renamed or moved in, out of or within someone else's share.
        if item.shared_with_me() || from.shared_with_me() || to.shared_with_me() {
            return Err(Errno::EACCES);
        }
        let newname = newname.to_str().ok_or(Errno::EINVAL)?;
        // Nor is one made by renaming a folder to it.
        if newparent == 1 && writes::is_trash_folder(newname) {
            return Err(Errno::EACCES);
        }
        let target = me.child(newparent, OsStr::new(newname)).ok();
        if let Some((tino, titem)) = &target {
            if *tino == ino {
                return Ok(());
            }
            if titem.is_container() || item.is_container() {
                return Err(Errno::EEXIST);
            }
        }
        let pending = me.pending(ino);
        // Added and not yet sent, under another name or among other new files — a copy's
        // temporary name made its own — is all that can be renamed where only adding is.
        if me.add_only
            && (pending.is_none()
                || target
                    .as_ref()
                    .is_some_and(|(t, _)| me.pending(*t).is_none())
                || !me.remote.accepts(&to))
        {
            return Err(Errno::EROFS);
        }
        match (pending, target) {
            // A file written here taking another's name — an editor's save: what it replaces
            // (Apple's file, or another being written) goes, and Apple's into Recently Deleted
            // only once the new bytes are there (writes::send).
            (Some(p), Some((tino, titem))) => {
                let theirs = me.pending(tino);
                if let Some(w) = &me.writes {
                    w.items
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(&tino);
                    w.drop_staging(tino);
                }
                let replaced = theirs.map_or(Some(titem), |t| t.replaces);
                // The file this one was copied from, if it was Apple's and is left behind by
                // the rename, goes as a rename would take it.
                if let Some(old) = p
                    .replaces
                    .as_ref()
                    .filter(|o| replaced.as_ref().is_none_or(|r| r.drivewsid != o.drivewsid))
                {
                    me.change(me.remote.delete(old), &[&from])?;
                }
                me.restage(ino, &p.name, newname)?;
                me.with_pending(ino, |now| {
                    now.folder = to.drivewsid.clone();
                    now.name = newname.to_string();
                    now.replaces = replaced;
                    now.dirty = true;
                    now.generation += 1;
                });
            }
            // Apple's file taking another's name: the other into Recently Deleted first.
            (None, Some((tino, titem))) => {
                match me.pending(tino) {
                    Some(t) => {
                        if let Some(w) = &me.writes {
                            w.items
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .remove(&tino);
                            w.drop_staging(tino);
                        }
                        if let Some(old) = t.replaces {
                            me.change(me.remote.delete(&old), &[&to])?;
                        }
                    }
                    None => me.change(me.remote.delete(&titem), &[&to])?,
                }
                me.move_and_rename(&item, &from, &to, name, newname)?;
            }
            // A file written here, to a free name: Apple's copy (if any) is moved and renamed
            // there, and what is waiting follows it.
            (Some(p), None) => {
                if let Some(old) = &p.replaces {
                    me.move_and_rename(old, &from, &to, name, newname)?;
                    let now = me
                        .store
                        .children(&to.drivewsid)
                        .and_then(|c| c.into_iter().find(|i| i.drivewsid == old.drivewsid));
                    me.with_pending(ino, |w| {
                        if let Some(n) = now {
                            // Its version moved with the rename; not a change made elsewhere.
                            w.replaces = Some(n);
                        }
                    });
                }
                me.restage(ino, &p.name, newname)?;
                me.with_pending(ino, |now| {
                    now.folder = to.drivewsid.clone();
                    now.name = newname.to_string();
                    // A change, when there is something to send or a send on its way: one
                    // under the old name is followed by one under the new, which replaces it
                    // — finished as it was, the rename was lost.
                    if now.dirty || now.sending {
                        now.generation += 1;
                        now.dirty = true;
                    }
                });
            }
            (None, None) => me.move_and_rename(&item, &from, &to, name, newname)?,
        }
        if let Some(w) = &me.writes {
            w.save();
        }
        if me.pending(ino).is_some_and(|p| p.open == 0 && p.dirty) {
            Self::schedule(me, ino, writes::SETTLE);
        }
        Ok(())
    }

    fn restage(&self, ino: u64, from: &str, to: &str) -> Result<(), Errno> {
        self.writes
            .as_ref()
            .ok_or(Errno::EROFS)?
            .rename_staging(ino, from, to)
            .map_err(|_| Errno::EIO)
    }
}

/// Why a change failed, said in a development build (see the app's quiet!): never a name.
fn quiet_note(e: &str) {
    if cfg!(debug_assertions) {
        eprintln!(
            "drive folder: a change was refused: {}",
            e.chars().take(120).collect::<String>()
        );
    }
}

/// A request that panics is answered with an error (its reply, dropped unanswered, says EIO)
/// rather than ending the mount: a panic in the session's thread left every app using the
/// folder with "Transport endpoint is not connected" until Skykeep was restarted.
fn guarded(f: impl FnOnce()) {
    if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        let why = e
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| e.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        // What the panic said may name a file: printed, never in the diagnostics log.
        crate::note_about("a Drive folder request failed inside Skykeep", &why);
    }
}

impl Filesystem for DriveFs {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        guarded(|| {
            let inner = &self.0;
            let found = inner.child(parent.0, name);
            // A name not there in a listing gone stale — a file just saved on the phone, asked for
            // by its path — has its folder asked of Apple behind the answer, never in front of it:
            // file managers ask after names that are not there all the time (.hidden, desktop.ini),
            // and Apple's first answer can take seconds.
            if matches!(found, Err(Errno::ENOENT)) {
                if let Some(folder) = inner.item(parent.0) {
                    Inner::revalidate(inner, &folder.drivewsid);
                }
            }
            match found {
                Ok((ino, item)) => reply.entry(&TTL, &inner.attr(ino, &item), Generation(0)),
                Err(e) => reply.error(e),
            }
        })
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        guarded(|| match self.0.item(ino.0) {
            Some(item) => reply.attr(&TTL, &self.0.attr(ino.0, &item)),
            None => reply.error(Errno::ENOENT),
        })
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        guarded(|| {
            let inner = &self.0;
            let Some(item) = inner.item(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            // Only a file's length is Apple's to keep; modes and times are taken as they are.
            if size.is_some() && inner.read_only {
                return reply.error(Errno::EROFS);
            }
            if let (Some(size), false) = (size, item.is_container()) {
                if let Err(e) = inner.begin_writing(ino.0, &item, size > 0) {
                    return reply.error(e);
                }
                let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) else {
                    return reply.error(Errno::EROFS);
                };
                let ok = std::fs::OpenOptions::new()
                    .write(true)
                    .open(w.staging(ino.0, &p.name))
                    .and_then(|f| f.set_len(size))
                    .is_ok();
                if !ok {
                    return reply.error(Errno::EIO);
                }
                inner.with_pending(ino.0, |p| {
                    p.dirty = true;
                    p.generation += 1;
                    p.modified = SystemTime::now();
                });
                if inner.pending(ino.0).is_some_and(|p| p.open == 0) {
                    Inner::schedule(inner, ino.0, writes::SETTLE);
                }
            }
            reply.attr(&TTL, &inner.attr(ino.0, &item));
        })
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        guarded(|| {
            let inner = &self.0;
            let children = match inner.children(ino.0) {
                Ok(c) => c,
                Err(e) => return reply.error(e),
            };
            if offset == 0 {
                if let Some(folder) = inner.item(ino.0) {
                    inner.fill_thumbnails(&folder, &children);
                    Inner::revalidate(inner, &folder.drivewsid);
                }
            }
            let parent = inner
                .item(ino.0)
                .and_then(|i| i.parent_id)
                .and_then(|p| inner.inodes.lock().ok().and_then(|n| n.ino_of(&p)))
                .unwrap_or(1);
            let mut all: Vec<(u64, FileType, String)> = vec![
                (ino.0, FileType::Directory, ".".into()),
                (parent, FileType::Directory, "..".into()),
            ];
            all.extend(children.into_iter().map(|(name, i, item)| {
                (
                    i,
                    if item.is_container() {
                        FileType::Directory
                    } else {
                        FileType::RegularFile
                    },
                    name,
                )
            }));
            for (n, (i, kind, name)) in all
                .into_iter()
                .enumerate()
                .skip(usize::try_from(offset).unwrap_or(0))
            {
                if reply.add(INodeNo(i), n as u64 + 1, kind, name) {
                    break;
                }
            }
            reply.ok();
        })
    }

    fn open(&self, req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        guarded(|| {
            let inner = &self.0;
            let Some(item) = inner.item(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            if item.is_container() {
                return reply.error(Errno::EISDIR);
            }
            let writing = flags.acc_mode() != OpenAccMode::O_RDONLY;
            let truncate = flags.0 & libc::O_TRUNC != 0;
            if writing && (inner.writes.is_none() || inner.read_only) {
                return reply.error(Errno::EROFS);
            }
            if inner.pending(ino.0).is_none() {
                if writing {
                    if let Err(e) = inner.begin_writing(ino.0, &item, !truncate) {
                        return reply.error(e);
                    }
                } else {
                    // A thumbnailer is not let download a file only to draw it small.
                    let pid = req.pid();
                    let comm =
                        std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
                    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline"))
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default();
                    if is_thumbnailer(&comm, &cmdline)
                        && !inner.cache.whole(&item)
                        && !inner.remote.is_local(&item)
                    {
                        return reply.error(Errno::EACCES);
                    }
                    // A file served at another length than listed (iWork's packages): its length
                    // asked of Apple before it is read, and read past what the kernel last heard,
                    // so it is never cut short at the listing's length.
                    if crate::served_size_differs(&item) && !inner.remote.is_local(&item) {
                        let (c, r, i) = (
                            Arc::clone(&inner.cache),
                            Arc::clone(&inner.remote),
                            item.clone(),
                        );
                        let _ = inner
                            .rt
                            .block_on(async move { c.sized(r.as_ref(), &i).await });
                        return reply.opened(FileHandle(0), FopenFlags::FOPEN_DIRECT_IO);
                    }
                    return reply.opened(FileHandle(0), FopenFlags::empty());
                }
            }
            if truncate {
                if let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) {
                    let _ = std::fs::OpenOptions::new()
                        .write(true)
                        .open(w.staging(ino.0, &p.name))
                        .and_then(|f| f.set_len(0));
                }
            }
            inner.with_pending(ino.0, |p| {
                p.open += 1;
                if truncate {
                    p.dirty = true;
                    p.generation += 1;
                    p.modified = SystemTime::now();
                }
            });
            // The kernel's page cache is not kept for a file being written: every read is ours.
            reply.opened(FileHandle(ino.0), FopenFlags::FOPEN_DIRECT_IO);
        })
    }

    fn create(
        &self,
        req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        guarded(|| {
            if self.0.read_only {
                return reply.error(Errno::EROFS);
            }
            let inner = &self.0;
            let Some(writes) = inner.writes.clone() else {
                return reply.error(Errno::EROFS);
            };
            let Some(folder) = inner.item(parent.0) else {
                return reply.error(Errno::ENOENT);
            };
            let Some(name) = name.to_str() else {
                return reply.error(Errno::EINVAL);
            };
            if inner.add_only && !inner.remote.accepts(&folder) {
                return reply.error(Errno::EROFS);
            }
            if folder.shared_with_me() {
                return reply.error(Errno::EACCES);
            }
            if let Ok((ino, item)) = inner.child(parent.0, OsStr::new(name)) {
                if flags & libc::O_EXCL != 0 {
                    return reply.error(Errno::EEXIST);
                }
                if item.is_container() {
                    return reply.error(Errno::EISDIR);
                }
                let truncate = flags & libc::O_TRUNC != 0;
                if inner.pending(ino).is_none() {
                    if let Err(e) = inner.begin_writing(ino, &item, !truncate) {
                        return reply.error(e);
                    }
                }
                if truncate {
                    if let Some(p) = inner.pending(ino) {
                        let _ = std::fs::OpenOptions::new()
                            .write(true)
                            .open(writes.staging(ino, &p.name))
                            .and_then(|f| f.set_len(0));
                    }
                }
                inner.with_pending(ino, |p| {
                    p.open += 1;
                    if truncate {
                        p.dirty = true;
                        p.generation += 1;
                    }
                });
                let _ = req;
                return reply.created(
                    &TTL,
                    &inner.attr(ino, &item),
                    Generation(0),
                    FileHandle(ino),
                    FopenFlags::FOPEN_DIRECT_IO,
                );
            }
            let ino = inner
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .local(&folder.drivewsid, name);
            let staged = writes.staging(ino, name);
            let made = staged
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::File::create(&staged));
            if let Err(e) = made {
                return reply.error(disk(e));
            }
            writes
                .items
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    ino,
                    Pending {
                        folder: folder.drivewsid.clone(),
                        name: name.to_string(),
                        replaces: None,
                        open: 1,
                        dirty: true,
                        generation: 1,
                        modified: SystemTime::now(),
                        sender: false,
                        quiet_since: None,
                        sending: false,
                        failure: None,
                    },
                );
            inner
                .inodes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .place(ino, parent.0, name);
            writes.save();
            let Some(item) = inner.item(ino) else {
                return reply.error(Errno::EIO);
            };
            reply.created(
                &TTL,
                &inner.attr(ino, &item),
                Generation(0),
                FileHandle(ino),
                FopenFlags::FOPEN_DIRECT_IO,
            );
        })
    }

    fn read(
        &self,
        req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock: Option<LockOwner>,
        reply: ReplyData,
    ) {
        guarded(|| {
            let inner = &self.0;
            // A file being written reads what has been written.
            if let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) {
                let mut buf = vec![0u8; size as usize];
                return match std::fs::File::open(w.staging(ino.0, &p.name))
                    .and_then(|f| f.read_at(&mut buf, offset))
                {
                    Ok(n) => reply.data(&buf[..n]),
                    Err(_) => reply.error(Errno::EIO),
                };
            }
            let Some(item) = inner.item(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            // What a program reading here makes Skykeep download is counted under its name
            // (quince_record): the folder's reads are where a file manager or an indexer that
            // opens every file shows up.
            let why = quince_record::Why::Folder(program(req.pid()));
            // Kept whole by the app (a photo's original): read from there, no second copy. One
            // still downloading is read as it lands: each read waits for its own bytes only,
            // so a video opened from the folder starts playing long before it is all here.
            let arriving = quince_record::because_now(why.clone(), || inner.remote.arriving(&item));
            if let Some(arriving) = arriving {
                inner.rt.spawn(async move {
                    let bytes = match arriving.await {
                        Ok(crate::Arriving::Whole(p)) => {
                            let mut buf = vec![0u8; size as usize];
                            std::fs::File::open(p)
                                .and_then(|f| f.read_at(&mut buf, offset))
                                .ok()
                                .map(|n| {
                                    buf.truncate(n);
                                    buf
                                })
                        }
                        Ok(crate::Arriving::Landing(landing)) => {
                            crate::read_landing(landing.as_ref(), offset, u64::from(size)).await
                        }
                        Err(_) => None,
                    };
                    match bytes {
                        Some(b) => reply.data(&b),
                        None => reply.error(Errno::EIO),
                    }
                });
                return;
            }
            let (cache, remote) = (Arc::clone(&inner.cache), Arc::clone(&inner.remote));
            // Answered when the bytes are here, without holding up the folder's other requests.
            inner.rt.spawn(quince_record::because(why, async move {
                match cache
                    .read(remote.as_ref(), &item, offset, u64::from(size))
                    .await
                {
                    Ok(bytes) => reply.data(&bytes),
                    Err(_) => reply.error(Errno::EIO),
                }
            }));
        })
    }

    fn write(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        guarded(|| {
            let inner = &self.0;
            let (Some(w), Some(p)) = (&inner.writes, inner.pending(ino.0)) else {
                return reply.error(Errno::EBADF);
            };
            let written = std::fs::OpenOptions::new()
                .write(true)
                .open(w.staging(ino.0, &p.name))
                .and_then(|f| f.write_all_at(data, offset));
            if let Err(e) = written {
                return reply.error(disk(e));
            }
            inner.with_pending(ino.0, |p| {
                p.dirty = true;
                p.generation += 1;
                p.modified = SystemTime::now();
            });
            reply.written(u32::try_from(data.len()).unwrap_or(u32::MAX));
        })
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _lock: LockOwner,
        reply: ReplyEmpty,
    ) {
        guarded(|| {
            reply.ok();
        })
    }

    fn fsync(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        guarded(|| {
            // On this computer's disk is as far as a sync can promise; Apple's copy follows.
            if let (Some(w), Some(p)) = (&self.0.writes, self.0.pending(ino.0)) {
                let _ = std::fs::File::open(w.staging(ino.0, &p.name)).and_then(|f| f.sync_all());
            }
            reply.ok();
        })
    }

    fn release(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        _flags: OpenFlags,
        _lock: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        guarded(|| {
            let inner = &self.0;
            // Some(true): last close of a changed file, to send. Some(false): last close of a file
            // opened for writing and never written — its copy is let go of, or every read went on
            // being served from that copy and changes made on the phone stayed unseen.
            let last = inner.with_pending(ino.0, |p| {
                p.open = p.open.saturating_sub(1);
                (p.open == 0).then_some(p.dirty || p.sending || p.sender)
            });
            match last {
                Some(Some(true)) => {
                    if let Some(w) = &inner.writes {
                        w.save();
                    }
                    Inner::schedule(inner, ino.0, writes::SETTLE);
                }
                Some(Some(false)) => {
                    if let Some(w) = &inner.writes {
                        w.items
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&ino.0);
                        w.drop_staging(ino.0);
                        w.save();
                    }
                }
                _ => {}
            }
            reply.ok();
        })
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        guarded(|| {
            if self.0.read_only || self.0.add_only {
                return reply.error(Errno::EROFS);
            }
            match self.0.mkdir(parent.0, name) {
                Ok((ino, item)) => reply.entry(&TTL, &self.0.attr(ino, &item), Generation(0)),
                Err(e) => reply.error(e),
            }
        })
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        guarded(|| {
            if self.0.read_only {
                return reply.error(Errno::EROFS);
            }
            match self.0.remove(parent.0, name, false) {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(e),
            }
        })
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        guarded(|| {
            if self.0.read_only || self.0.add_only {
                return reply.error(Errno::EROFS);
            }
            match self.0.remove(parent.0, name, true) {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(e),
            }
        })
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        guarded(|| {
            if self.0.read_only {
                return reply.error(Errno::EROFS);
            }
            match Inner::rename(&self.0, parent.0, name, newparent.0, newname) {
                Ok(()) => reply.ok(),
                Err(e) => reply.error(e),
            }
        })
    }

    /// `user.quince.keep`: "1" on a file or folder kept on this computer (chosen, or inside a
    /// chosen folder); no such attribute otherwise. The only attribute these folders keep.
    fn getxattr(&self, _req: &Request, ino: INodeNo, name: &OsStr, size: u32, reply: ReplyXattr) {
        guarded(|| {
            let inner = &self.0;
            let Some(item) = inner.item(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            if name != OsStr::new(KEEP_ATTR) || !inner.kept_under(&item) {
                return reply.error(Errno::NO_XATTR);
            }
            match size {
                0 => reply.size(1),
                n if n < 1 => reply.error(Errno::ERANGE),
                _ => reply.data(b"1"),
            }
        })
    }

    fn listxattr(&self, _req: &Request, ino: INodeNo, size: u32, reply: ReplyXattr) {
        guarded(|| {
            let inner = &self.0;
            let Some(item) = inner.item(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            let names: &[u8] = if inner.kept_under(&item) {
                b"user.quince.keep\0"
            } else {
                b""
            };
            let len = u32::try_from(names.len()).unwrap_or(u32::MAX);
            if size == 0 {
                reply.size(len)
            } else if size < len {
                reply.error(Errno::ERANGE)
            } else {
                reply.data(names)
            }
        })
    }

    /// Kept on this computer (any value but "0"), or let go of ("0"), as the window's Keep on
    /// This Computer does.
    fn setxattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        name: &OsStr,
        value: &[u8],
        _flags: i32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        guarded(|| {
            let inner = &self.0;
            if name != OsStr::new(KEEP_ATTR) {
                return reply.error(Errno::ENOTSUP);
            }
            // Only what the folder can fetch is kept: a file still being written here is already
            // on this computer, and the Photos folder reads from the app's own photo cache.
            if inner.add_only || inner.read_only || inner.writes.is_none() {
                return reply.error(Errno::ENOTSUP);
            }
            let Some(item) = inner.item(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            if item.drivewsid.starts_with("LOCAL::") {
                return reply.error(Errno::ENOTSUP);
            }
            Inner::keep(inner, &item.drivewsid, value != b"0");
            reply.ok();
        })
    }

    fn removexattr(&self, _req: &Request, ino: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        guarded(|| {
            let inner = &self.0;
            let Some(item) = inner.item(ino.0) else {
                return reply.error(Errno::ENOENT);
            };
            // Only what was chosen itself can be let go of; a file kept by its folder stays kept.
            if name != OsStr::new(KEEP_ATTR) || !inner.cache.kept().chosen.contains(&item.drivewsid)
            {
                return reply.error(Errno::NO_XATTR);
            }
            Inner::keep(inner, &item.drivewsid, false);
            reply.ok();
        })
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        guarded(|| {
            let inner = &self.0;
            // The account's own storage, asked at most once a minute: a file manager checks there
            // is room before copying in.
            let known = *inner
                .space
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let space = match known {
                Some((s, at)) if at.elapsed() < Duration::from_secs(60) => Some(s),
                _ => {
                    let remote = Arc::clone(&inner.remote);
                    let got = inner
                        .rt
                        .block_on(async move {
                            tokio::time::timeout(Duration::from_secs(5), remote.space()).await
                        })
                        .ok()
                        .and_then(Result::ok);
                    if let Some(s) = got {
                        *inner
                            .space
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some((s, std::time::Instant::now()));
                    }
                    got.or(known.map(|(s, _)| s))
                }
            };
            let block = 4096u64;
            match (space, inner.writes.is_some()) {
                (Some((used, total)), true) => {
                    let free = total.saturating_sub(used) / block;
                    reply.statfs(total / block, free, free, 0, 0, 4096, 255, 4096);
                }
                _ => reply.statfs(0, 0, 0, 0, 0, 4096, 255, 4096),
            }
        })
    }
}

/// The folder mounted at `at`, served on its own threads until the returned session is
/// dropped. A mount left behind by a crash ("Transport endpoint is not connected") is let go
/// of first.
///
/// # Errors
/// When the folder cannot be made or FUSE refuses the mount.
pub fn mount(fs: DriveFs, at: &Path) -> std::io::Result<fuser::BackgroundSession> {
    if let Err(e) = std::fs::metadata(at) {
        if e.raw_os_error() == Some(libc::ENOTCONN) {
            let _ = std::process::Command::new("fusermount3")
                .arg("-uz")
                .arg(at)
                .status();
        }
    }
    std::fs::create_dir_all(at)?;
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::FSName("quince".into()),
        MountOption::Subtype("quince".into()),
        MountOption::NoSuid,
        MountOption::NoDev,
        // Files from iCloud are never run from here (none is given an executable mode
        // either): one put in a shared folder by someone else stays a file to open.
        MountOption::NoExec,
        MountOption::NoAtime,
        MountOption::DefaultPermissions,
    ];
    config.n_threads = Some(4);
    let inner = Arc::clone(&fs.0);
    let session = fuser::spawn_mount(fs, at, &config)?;
    Inner::send_waiting(&inner);
    // What was kept here before is made whole again, behind (a file may have changed on
    // another device since).
    Inner::settle_kept(&inner);
    Ok(session)
}

/// The program a file system request came from, by name ("nautilus", "tracker-extract"):
/// `pid` is the asking thread's, so its process (the Tgid in /proc/<pid>/status) is named, as
/// the thread's own name is often a pool's. Kept for the last few, as a folder's reads come in
/// bursts from one program. Unknown (gone, or outside this pid namespace) is said so.
fn program(pid: u32) -> String {
    static NAMES: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
    let mut names = NAMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, n)) = names.iter().find(|(p, _)| *p == pid) {
        return n.clone();
    }
    let tgid = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Tgid:"))
                .and_then(|t| t.trim().parse::<u32>().ok())
        })
        .unwrap_or(pid);
    let name = std::fs::read_to_string(format!("/proc/{tgid}/comm"))
        .map(|n| n.trim().to_string())
        .ok()
        .filter(|n| pid != 0 && !n.is_empty())
        .unwrap_or_else(|| "an unknown program".to_string());
    if names.len() >= 32 {
        names.remove(0);
    }
    names.push((pid, name.clone()));
    name
}

#[cfg(test)]
mod listing_tests {
    /// A request that panics ends with that request: the mount's thread goes on.
    #[test]
    fn a_request_that_panics_never_ends_the_mount() {
        let mut ran = false;
        guarded(|| ran = true);
        assert!(ran);
        guarded(|| panic!("a bug in one request"));
        // Still here: the session's loop would take the next request.
    }

    use super::*;
    use crate::{Fut, ROOT};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    struct Apple;
    impl Remote for Apple {
        fn list(&self, _folder: &str) -> Fut<Vec<DriveItem>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    /// A library that counts how often it is read, and whose version a test moves.
    struct Library {
        items: Mutex<Vec<DriveItem>>,
        reads: AtomicUsize,
        version: AtomicU64,
    }
    impl Store for Library {
        fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            (folder == ROOT).then(|| self.items.lock().unwrap().clone())
        }
        fn version(&self) -> Option<u64> {
            Some(self.version.load(Ordering::SeqCst))
        }
    }

    fn file(n: usize) -> DriveItem {
        DriveItem {
            drivewsid: format!("FILE::{n}"),
            parent_id: Some(ROOT.into()),
            name: format!("Scan {n}"),
            extension: Some("pdf".into()),
            kind: "FILE".into(),
            size: Some(10),
            child_count: None,
            created_at: Some(1_700_000_000),
            modified_at: Some(1_700_000_000),
            etag: Some("e1".into()),
            share: None,
        }
    }

    /// A file manager listing a folder of many files looks each name up after: the folder is
    /// read from the library once for all of them, while nothing in it changes — and again,
    /// with the change, once something does.
    #[test]
    fn a_folders_names_are_looked_up_without_reading_it_again() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let library = Arc::new(Library {
            items: Mutex::new((0..2000).map(file).collect()),
            reads: AtomicUsize::new(0),
            version: AtomicU64::new(1),
        });
        let fs = DriveFs::new(
            Arc::new(Apple),
            Arc::clone(&library) as Arc<dyn Store>,
            Arc::new(Cache::new(cache.path()).unwrap()),
            rt.handle().clone(),
        );
        let inner = &fs.0;
        let listed = inner.children(1).unwrap();
        assert_eq!(listed.len(), 2000);
        for (name, ino, item) in &listed {
            let (found, it) = inner.child(1, OsStr::new(name)).unwrap();
            assert_eq!((found, &it), (*ino, item));
        }
        assert!(matches!(
            inner.child(1, OsStr::new("desktop.ini")),
            Err(Errno::ENOENT)
        ));
        assert_eq!(
            library.reads.load(Ordering::SeqCst),
            1,
            "two thousand look-ups, one read"
        );

        // A file saved elsewhere lands in the library: the next look sees it.
        library.items.lock().unwrap().push(file(2000));
        library.version.fetch_add(1, Ordering::SeqCst);
        assert!(inner.child(1, OsStr::new("Scan 2000.pdf")).is_ok());
        assert_eq!(library.reads.load(Ordering::SeqCst), 2);
        // The same answers as working it out from nothing.
        let again = inner.children(1).unwrap();
        assert_eq!(again.len(), 2001);
        assert_eq!(&again[..2000], &listed[..]);
    }

    /// A folder that is empty as this computer last listed it, with a file put in it on
    /// another device since: removing it asks Apple first and is refused as not empty, so the
    /// file is not sent to Recently Deleted with it. One Apple also finds empty goes.
    #[test]
    fn a_folder_filled_elsewhere_is_not_removed_as_empty() {
        use std::sync::atomic::AtomicBool;
        struct Elsewhere {
            filled: AtomicBool,
            deleted: Mutex<Vec<String>>,
        }
        impl Remote for Elsewhere {
            fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
                let items = if folder == "FOLDER::Old" && self.filled.load(Ordering::SeqCst) {
                    vec![DriveItem {
                        parent_id: Some("FOLDER::Old".into()),
                        ..file(7)
                    }]
                } else {
                    Vec::new()
                };
                Box::pin(async move { Ok(items) })
            }
            fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
                Box::pin(async { Ok(Vec::new()) })
            }
            fn delete(&self, item: &DriveItem) -> Fut<()> {
                self.deleted.lock().unwrap().push(item.drivewsid.clone());
                Box::pin(async { Ok(()) })
            }
        }
        // The library: the folder at the top, listed empty when last looked at.
        struct Listed;
        impl Store for Listed {
            fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
                match folder {
                    ROOT => Some(vec![DriveItem {
                        drivewsid: "FOLDER::Old".into(),
                        parent_id: Some(ROOT.into()),
                        name: "Old".into(),
                        extension: None,
                        kind: "FOLDER".into(),
                        size: None,
                        child_count: Some(0),
                        created_at: None,
                        modified_at: None,
                        etag: Some("f1".into()),
                        share: None,
                    }]),
                    _ => Some(Vec::new()),
                }
            }
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let remote = Arc::new(Elsewhere {
            filled: AtomicBool::new(true),
            deleted: Mutex::new(Vec::new()),
        });
        let fs = DriveFs::new(
            Arc::clone(&remote) as Arc<dyn Remote>,
            Arc::new(Listed),
            Arc::new(Cache::new(cache.path()).unwrap()),
            rt.handle().clone(),
        );
        assert!(matches!(
            fs.0.remove(1, OsStr::new("Old"), true),
            Err(Errno::ENOTEMPTY)
        ));
        assert!(remote.deleted.lock().unwrap().is_empty(), "nothing deleted");

        remote.filled.store(false, Ordering::SeqCst);
        assert!(fs.0.remove(1, OsStr::new("Old"), true).is_ok());
        assert_eq!(
            *remote.deleted.lock().unwrap(),
            vec!["FOLDER::Old".to_string()]
        );
    }

    /// GNOME Files' "Move to Trash" makes `.Trash-<uid>` at the top of the mount: refused, so
    /// no hidden folder is made in iCloud Drive and Files deletes the file instead. Folders
    /// that only look like it are made as ever.
    #[test]
    fn no_trash_folder_is_made_at_the_top() {
        struct Folders(Mutex<Vec<String>>);
        impl Remote for Folders {
            fn list(&self, _folder: &str) -> Fut<Vec<DriveItem>> {
                Box::pin(async { Ok(Vec::new()) })
            }
            fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
                Box::pin(async { Ok(Vec::new()) })
            }
            fn mkdir(&self, parent: &DriveItem, name: &str) -> Fut<DriveItem> {
                self.0.lock().unwrap().push(name.to_string());
                let made = DriveItem {
                    drivewsid: format!("FOLDER::{name}"),
                    parent_id: Some(parent.drivewsid.clone()),
                    name: name.into(),
                    extension: None,
                    kind: "FOLDER".into(),
                    size: None,
                    child_count: Some(0),
                    created_at: None,
                    modified_at: None,
                    etag: Some("f1".into()),
                    share: None,
                };
                Box::pin(async move { Ok(made) })
            }
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let remote = Arc::new(Folders(Mutex::new(Vec::new())));
        let library = Arc::new(Library {
            items: Mutex::new(Vec::new()),
            reads: AtomicUsize::new(0),
            version: AtomicU64::new(1),
        });
        let fs = DriveFs::new(
            Arc::clone(&remote) as Arc<dyn Remote>,
            library as Arc<dyn Store>,
            Arc::new(Cache::new(cache.path()).unwrap()),
            rt.handle().clone(),
        );
        for name in [".Trash-1000", ".Trash"] {
            assert!(matches!(
                fs.0.mkdir(1, OsStr::new(name)),
                Err(Errno::EACCES)
            ));
        }
        assert!(remote.0.lock().unwrap().is_empty(), "nothing made at Apple");
        let _ = fs.0.mkdir(1, OsStr::new("Trash-notes"));
        let _ = fs.0.mkdir(1, OsStr::new(".Trash-x"));
        assert_eq!(
            *remote.0.lock().unwrap(),
            vec!["Trash-notes".to_string(), ".Trash-x".to_string()]
        );
        assert!(crate::writes::is_trash_folder(".Trash-1000"));
        assert!(!crate::writes::is_trash_folder(".Trash-"));
    }

    /// Someone else's share is read only here: nothing in it, nor it, is deleted, renamed or
    /// moved, nothing is moved or made in it, and its modes say so — refused here, so nothing
    /// is ever asked of Apple.
    #[test]
    fn what_is_shared_with_me_is_read_only() {
        struct Writes(Mutex<Vec<String>>);
        impl Remote for Writes {
            fn list(&self, _folder: &str) -> Fut<Vec<DriveItem>> {
                Box::pin(async { Ok(Vec::new()) })
            }
            fn read(&self, _item: &DriveItem, _offset: u64, _len: u64) -> Fut<Vec<u8>> {
                Box::pin(async { Ok(Vec::new()) })
            }
            fn mkdir(&self, _parent: &DriveItem, name: &str) -> Fut<DriveItem> {
                self.0.lock().unwrap().push(format!("mkdir {name}"));
                Box::pin(async { Err("no".to_string()) })
            }
            fn rename(&self, item: &DriveItem, _name: &str) -> Fut<()> {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("rename {}", item.drivewsid));
                Box::pin(async { Ok(()) })
            }
            fn move_to(&self, item: &DriveItem, _folder: &DriveItem) -> Fut<()> {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("move {}", item.drivewsid));
                Box::pin(async { Ok(()) })
            }
            fn delete(&self, item: &DriveItem) -> Fut<()> {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("delete {}", item.drivewsid));
                Box::pin(async { Ok(()) })
            }
        }
        let share = "SHARED_FOLDER::com.apple.CloudDocs::trip";
        let at = |id: &str, name: &str, kind: &str, parent: &str| DriveItem {
            drivewsid: id.into(),
            parent_id: Some(parent.into()),
            name: name.into(),
            extension: None,
            kind: kind.into(),
            size: (kind == "FILE").then_some(3),
            child_count: (kind == "FOLDER").then_some(1),
            created_at: None,
            modified_at: None,
            etag: Some("e".into()),
            share: None,
        };
        struct Tree(Vec<DriveItem>);
        impl Store for Tree {
            fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
                Some(
                    self.0
                        .iter()
                        .filter(|i| i.parent_id.as_deref() == Some(folder))
                        .cloned()
                        .collect(),
                )
            }
        }
        let library = Arc::new(Tree(vec![
            at(share, "Trip", "FOLDER", ROOT),
            at("FILE::com.apple.CloudDocs::own", "own", "FILE", ROOT),
            at(
                "FILE_IN_SHARED_FOLDER::com.apple.CloudDocs::pic",
                "pic",
                "FILE",
                share,
            ),
        ]));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let remote = Arc::new(Writes(Mutex::new(Vec::new())));
        let fs = DriveFs::new(
            Arc::clone(&remote) as Arc<dyn Remote>,
            library as Arc<dyn Store>,
            Arc::new(Cache::new(cache.path()).unwrap()),
            rt.handle().clone(),
        );
        let (trip, folder) = fs.0.child(1, OsStr::new("Trip")).unwrap();
        let (pic_ino, pic) = fs.0.child(trip, OsStr::new("pic")).unwrap();
        let denied = |r: Result<(), Errno>| assert!(matches!(r, Err(Errno::EACCES)), "{r:?}");
        denied(fs.0.remove(trip, OsStr::new("pic"), false));
        denied(fs.0.remove(1, OsStr::new("Trip"), true));
        denied(Inner::rename(
            &fs.0,
            trip,
            OsStr::new("pic"),
            trip,
            OsStr::new("p2"),
        ));
        denied(Inner::rename(
            &fs.0,
            trip,
            OsStr::new("pic"),
            1,
            OsStr::new("pic"),
        ));
        denied(Inner::rename(
            &fs.0,
            1,
            OsStr::new("Trip"),
            1,
            OsStr::new("Trip 2"),
        ));
        denied(Inner::rename(
            &fs.0,
            1,
            OsStr::new("own"),
            trip,
            OsStr::new("own"),
        ));
        denied(fs.0.mkdir(trip, OsStr::new("New")).map(|_| ()));
        assert_eq!(fs.0.attr(trip, &folder).perm, 0o555);
        assert_eq!(fs.0.attr(pic_ino, &pic).perm, 0o444);
        assert!(
            remote.0.lock().unwrap().is_empty(),
            "nothing asked of Apple"
        );
        // The account's own file is renamed as ever.
        assert!(Inner::rename(&fs.0, 1, OsStr::new("own"), 1, OsStr::new("mine")).is_ok());
        assert_eq!(remote.0.lock().unwrap().len(), 1);
    }

    /// Without a version to go by, every look reads the library, as it always did.
    #[test]
    fn a_store_without_a_version_is_read_every_time() {
        struct Plain(Vec<DriveItem>, AtomicUsize);
        impl Store for Plain {
            fn children(&self, _folder: &str) -> Option<Vec<DriveItem>> {
                self.1.fetch_add(1, Ordering::SeqCst);
                Some(self.0.clone())
            }
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let store = Arc::new(Plain(vec![file(1)], AtomicUsize::new(0)));
        let fs = DriveFs::new(
            Arc::new(Apple),
            Arc::clone(&store) as Arc<dyn Store>,
            Arc::new(Cache::new(cache.path()).unwrap()),
            rt.handle().clone(),
        );
        for _ in 0..3 {
            assert!(fs.0.child(1, OsStr::new("Scan 1.pdf")).is_ok());
        }
        assert_eq!(store.1.load(Ordering::SeqCst), 3);
    }
}
