use super::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request as Asked, Respond, ResponseTemplate};

// ── Names, paths, links ─────────────────────────────────────────────────────────────────────

fn name_in(dir: &Path, taken: &mut Taken, given: &str, folder: bool) -> String {
    let at = taken.name(dir, given, folder).unwrap();
    assert_eq!(at.parent(), Some(dir), "{given:?}");
    assert!(crate::saving::directly_in(dir, &at), "{given:?}");
    at.file_name().unwrap().to_string_lossy().into_owned()
}

/// Whatever a sharer called it, a name is one plain name in its own folder — on Linux and on
/// Windows alike, where `..\`, `C:`, a stream, a device or a trailing dot reach further.
#[test]
fn every_name_is_plain_and_stays_in_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    for (given, want) in [
        ("Report.pdf", "Report.pdf"),
        ("..", "Download"),
        (".", "Download"),
        ("", "Download"),
        (
            "../../.config/autostart/x.desktop",
            "configautostartx.desktop",
        ),
        (
            "..\\..\\AppData\\Roaming\\evil.bat",
            "AppDataRoamingevil.bat",
        ),
        ("/etc/passwd", "etcpasswd"),
        ("C:\\Windows\\evil.dll", "CWindowsevil.dll"),
        ("C:evil.txt", "Cevil.txt"),
        ("report.pdf:evil.exe", "report.pdfevil.exe"),
        ("a*b?c\"d<e>f|g.txt", "abcdefg.txt"),
        ("CON", "_CON"),
        ("con.txt", "_con.txt"),
        ("NUL .txt", "_NUL .txt"),
        ("COM1", "_COM1"),
        ("LPT\u{b9}.log", "_LPT\u{b9}.log"),
        ("trailing. . ", "trailing"),
        (" .hidden", "hidden"),
        ("\u{202E}fdp.exe", "fdp.exe"),
        ("写真 🎉.png", "写真 🎉.png"),
        ("Привет мир.docx", "Привет мир.docx"),
    ] {
        assert_eq!(
            name_in(dir.path(), &mut Taken::default(), given, false),
            want,
            "{given:?}"
        );
    }
    assert_eq!(
        name_in(dir.path(), &mut Taken::default(), "..", true),
        "Folder"
    );
    // Long names are cut to a length every file system takes, the extension kept, never ending
    // in a dot or a space Windows would drop.
    let long = name_in(
        dir.path(),
        &mut Taken::default(),
        &format!("{}. x.pdf", "あ".repeat(200)),
        false,
    );
    assert!(long.len() <= 120, "{} bytes", long.len());
    assert!(
        long.ends_with(".pdf") && !long.ends_with([' ', '.']),
        "{long}"
    );
}

#[test]
fn a_macs_decomposed_accents_are_saved_composed() {
    let dir = tempfile::tempdir().unwrap();
    let name = name_in(
        dir.path(),
        &mut Taken::default(),
        "Cafe\u{301} cre\u{300}me.txt",
        false,
    );
    assert_eq!(name, "Caf\u{e9} cr\u{e8}me.txt");
}

/// Two names one Windows, macOS or exFAT folder would take for one file — the same but for
/// case, or for how an accent is written, or once cleaned — are numbered.
#[test]
fn names_one_folder_would_take_for_one_are_numbered() {
    let dir = tempfile::tempdir().unwrap();
    let mut taken = Taken::default();
    let mut name = |given: &str, folder: bool| name_in(dir.path(), &mut taken, given, folder);
    assert_eq!(name("Photo.JPG", false), "Photo.JPG");
    assert_eq!(name("photo.jpg", false), "photo 2.jpg");
    assert_eq!(name("PHOTO.jpg", false), "PHOTO 3.jpg");
    assert_eq!(name("Caf\u{e9}.txt", false), "Caf\u{e9}.txt");
    assert_eq!(name("Cafe\u{301}.txt", false), "Caf\u{e9} 2.txt");
    assert_eq!(name("a:b.txt", false), "ab.txt");
    assert_eq!(name("ab.txt", false), "ab 2.txt");
    assert_eq!(name("v1.2", true), "v1.2");
    assert_eq!(name("V1.2", true), "V1.2 2");
    assert_eq!(name("archive.tar.gz", false), "archive.tar.gz");
    assert_eq!(name("Archive.tar.gz", false), "Archive 2.tar.gz");
}

/// Inside the download's own folder, names come from the tree alone — not from what is on disk
/// — so reading the same tree again names it the same way: what resuming depends on.
#[test]
fn inside_its_own_folder_the_same_tree_is_named_the_same_way() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"from before").unwrap();
    let first = Taken::default().ours(dir.path(), "a.txt", false).unwrap();
    assert_eq!(first, dir.path().join("a.txt"));
    let mut taken = Taken::default();
    taken.ours(dir.path(), "A.txt", false).unwrap();
    assert_eq!(
        taken.ours(dir.path(), "a.txt", false).unwrap(),
        dir.path().join("a 2.txt")
    );
}

#[test]
fn a_folder_is_always_made_new_and_numbered_past_what_is_there() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Photos")).unwrap();
    std::fs::write(dir.path().join("Photos 2"), b"a file").unwrap();
    let made = make_folder(&mut Taken::default(), dir.path(), "Photos").unwrap();
    assert_eq!(made, dir.path().join("Photos 3"));
    assert!(made.is_dir());
}

#[cfg(unix)]
#[test]
fn a_link_in_the_way_is_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), dir.path().join("Photos")).unwrap();
    std::os::unix::fs::symlink("/nonexistent/target", dir.path().join("Photos 2")).unwrap();
    let made = make_folder(&mut Taken::default(), dir.path(), "Photos").unwrap();
    assert_eq!(made, dir.path().join("Photos 3"));
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);

    // A folder of the download's own swapped for a link is noticed before a file goes in, and
    // a link is never taken for a folder made before.
    let top = dir.path().join("Photos 3");
    std::fs::create_dir(top.join("Trip")).unwrap();
    assert!(real_folders(&top, &top.join("Trip")));
    std::fs::remove_dir(top.join("Trip")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), top.join("Trip")).unwrap();
    assert!(!real_folders(&top, &top.join("Trip")));
    assert!(!real_folders(&top, Path::new("/etc")));
    assert!(ensure_folder(&mut Taken::default(), &top, "Trip").is_err());
}

#[test]
fn disk_errors_are_said_in_words() {
    #[cfg(unix)]
    let (full, long) = (libc::ENOSPC, libc::ENAMETOOLONG);
    #[cfg(windows)]
    let (full, long) = (112, 206);
    let full = std::io::Error::from_raw_os_error(full);
    assert!(no_room(&full));
    assert!(disk_words(&full).contains("enough space"));
    let long = std::io::Error::from_raw_os_error(long);
    assert!(!no_room(&long));
    assert!(disk_words(&long).contains("too long"));
    assert!(no_room(&std::io::Error::from(
        std::io::ErrorKind::StorageFull
    )));
    assert_eq!(size_words(512), "512 bytes");
    assert_eq!(size_words(3_200_000_000), "3.2 GB");
}

#[test]
fn this_computers_free_space_can_be_read() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_space(dir.path()).is_some_and(|n| n > 0));
}

#[tokio::test]
async fn cancel_stops_what_runs_and_what_is_to_come() {
    let stop = Stop::default();
    let running = {
        let stop = stop.clone();
        tokio::spawn(async move {
            stop.guard(tokio::time::sleep(Duration::from_secs(30)))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    stop.stop();
    let stopped = tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .expect("stopped at once")
        .unwrap();
    assert!(stopped.is_none());
    assert!(
        stop.guard(async { 1 }).await.is_none(),
        "nothing begins once stopped"
    );
    assert!(stop.0.running.lock().unwrap().is_empty());
    let fresh = Stop::default();
    assert_eq!(fresh.guard(async { 2 }).await, Some(2));
    assert!(
        fresh.0.running.lock().unwrap().is_empty(),
        "what finishes lets go of its stop"
    );
}

/// The journal: the same items in any order are one download; only a key's own shape names a
/// file; a record comes back as it went in.
#[test]
fn the_journal_keeps_what_it_is_given_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::new(dir.path().join("downloads"));
    let (a, b) = (
        item(&file("a", "a", None, b"")),
        item(&folder("b", "B", vec![])),
    );
    assert_eq!(
        Journal::key(&[a.clone(), b.clone()]),
        Journal::key(&[b.clone(), a.clone()])
    );
    assert_ne!(
        Journal::key(std::slice::from_ref(&a)),
        Journal::key(std::slice::from_ref(&b))
    );
    let key = Journal::key(&[a.clone(), b.clone()]);
    let record = Record {
        key: key.clone(),
        title: "2 items".into(),
        items: vec![a, b],
        placed: vec![Placed {
            drivewsid: "x".into(),
            path: "/d/B".into(),
            folder: true,
        }],
        started_at: 7,
    };
    journal.save(&record).unwrap();
    assert_eq!(journal.load(&key), Some(record.clone()));
    assert_eq!(journal.unfinished(), vec![record]);
    assert!(journal.load("../../etc/passwd").is_none());
    assert!(journal.file("0123").is_none());
    journal.forget(&key);
    assert!(journal.unfinished().is_empty());
}

// ── A Drive of its own ──────────────────────────────────────────────────────────────────────

const ZONE: &str = "com.apple.CloudDocs";

#[derive(Clone)]
struct Node {
    id: String,
    name: String,
    ext: Option<String>,
    folder: bool,
    bytes: Vec<u8>,
    children: Vec<Node>,
    /// When iCloud says it was last changed, in seconds.
    changed: Option<i64>,
}

fn file(id: &str, name: &str, ext: Option<&str>, bytes: &[u8]) -> Node {
    Node {
        id: id.into(),
        name: name.into(),
        ext: ext.map(Into::into),
        folder: false,
        bytes: bytes.to_vec(),
        children: vec![],
        changed: None,
    }
}

fn folder(id: &str, name: &str, children: Vec<Node>) -> Node {
    Node {
        id: id.into(),
        name: name.into(),
        ext: None,
        folder: true,
        bytes: vec![],
        children,
        changed: None,
    }
}

fn wsid(n: &Node) -> String {
    format!(
        "{}::{ZONE}::{}",
        if n.folder { "FOLDER" } else { "FILE" },
        n.id
    )
}

fn as_json(n: &Node, parent: &str) -> Value {
    json!({
        "drivewsid": wsid(n), "docwsid": n.id, "parentId": parent, "name": n.name,
        "extension": n.ext, "type": if n.folder { "FOLDER" } else { "FILE" },
        "size": if n.folder { -1 } else { n.bytes.len() as i64 }, "etag": "1",
        "dateModified": n.changed.map(|s| s * 1000),
    })
}

fn item(n: &Node) -> DriveItem {
    DriveItem {
        drivewsid: wsid(n),
        parent_id: None,
        name: n.name.clone(),
        extension: n.ext.clone(),
        kind: if n.folder { "FOLDER" } else { "FILE" }.into(),
        size: (!n.folder).then_some(n.bytes.len() as i64),
        child_count: None,
        created_at: None,
        modified_at: n.changed,
        etag: None,
    }
}

/// `retrieveItemDetailsInFolders` for the pretend tree.
struct Listings {
    folders: HashMap<String, Vec<Value>>,
    broken: HashSet<String>,
    signed_out: bool,
}

impl Respond for Listings {
    fn respond(&self, request: &Asked) -> ResponseTemplate {
        if self.signed_out {
            return ResponseTemplate::new(450);
        }
        let asked: Value = serde_json::from_slice(&request.body).unwrap();
        let id = asked[0]["drivewsid"].as_str().unwrap().to_string();
        if self.broken.contains(&id) {
            return ResponseTemplate::new(500);
        }
        match self.folders.get(&id) {
            Some(items) => ResponseTemplate::new(200)
                .set_body_json(json!([{ "drivewsid": id, "status": "OK", "items": items }])),
            None => ResponseTemplate::new(200)
                .set_body_json(json!([{ "items": [], "status": "ID_INVALID" }])),
        }
    }
}

/// `download/by_id`: a link to the bytes.
struct Links(String);

impl Respond for Links {
    fn respond(&self, request: &Asked) -> ResponseTemplate {
        let id = request
            .url
            .query_pairs()
            .find(|(k, _)| k == "document_id")
            .map(|(_, v)| v.into_owned())
            .unwrap();
        ResponseTemplate::new(200)
            .set_body_json(json!({ "data_token": { "url": format!("{}/bytes/{id}", self.0) } }))
    }
}

type Hits = Arc<Mutex<HashMap<String, u32>>>;

/// The bytes themselves: some busy for a while, some very slow, every fetch counted.
struct Bytes {
    files: HashMap<String, Vec<u8>>,
    busy: Mutex<HashMap<String, u32>>,
    slow: HashSet<String>,
    /// After this many fetches, every one is busy: a connection that goes and stays gone.
    busy_after: Option<u32>,
    hits: Hits,
}

impl Respond for Bytes {
    fn respond(&self, request: &Asked) -> ResponseTemplate {
        let id = request.url.path().rsplit('/').next().unwrap().to_string();
        let asked: u32 = {
            let mut hits = self.hits.lock().unwrap();
            *hits.entry(id.clone()).or_default() += 1;
            hits.values().sum()
        };
        if self.busy_after.is_some_and(|n| asked > n) {
            return ResponseTemplate::new(503);
        }
        if let Some(left) = self.busy.lock().unwrap().get_mut(&id).filter(|n| **n > 0) {
            *left -= 1;
            return ResponseTemplate::new(503);
        }
        let reply = ResponseTemplate::new(200).set_body_bytes(self.files[&id].clone());
        if self.slow.contains(&id) {
            reply.set_delay(Duration::from_secs(30))
        } else {
            reply
        }
    }
}

#[derive(Default)]
struct Setup {
    broken: Vec<&'static str>,
    busy: Vec<(&'static str, u32)>,
    slow: Vec<&'static str>,
    busy_after: Option<u32>,
    signed_out: bool,
    free_space: Option<fn(&Path) -> Option<u64>>,
}

fn index(
    n: &Node,
    folders: &mut HashMap<String, Vec<Value>>,
    files: &mut HashMap<String, Vec<u8>>,
) {
    if n.folder {
        folders.insert(
            wsid(n),
            n.children.iter().map(|c| as_json(c, &wsid(n))).collect(),
        );
        for c in &n.children {
            index(c, folders, files);
        }
    } else {
        files.insert(n.id.clone(), n.bytes.clone());
    }
}

async fn drive_of(roots: &[Node], setup: Setup) -> (MockServer, Drive, Hits) {
    let server = MockServer::builder()
        .disable_request_recording()
        .start()
        .await;
    let (mut folders, mut files) = (HashMap::new(), HashMap::new());
    for r in roots {
        index(r, &mut folders, &mut files);
    }
    Mock::given(method("POST"))
        .and(path("/retrieveItemDetailsInFolders"))
        .respond_with(Listings {
            folders,
            broken: setup
                .broken
                .iter()
                .map(|id| format!("FOLDER::{ZONE}::{id}"))
                .collect(),
            signed_out: setup.signed_out,
        })
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/ws/{ZONE}/download/by_id")))
        .respond_with(Links(server.uri()))
        .mount(&server)
        .await;
    let hits: Hits = Arc::default();
    Mock::given(method("GET"))
        .and(path_regex("^/bytes/"))
        .respond_with(Bytes {
            files,
            busy: Mutex::new(
                setup
                    .busy
                    .iter()
                    .map(|(id, n)| (id.to_string(), *n))
                    .collect(),
            ),
            slow: setup.slow.iter().map(ToString::to_string).collect(),
            busy_after: setup.busy_after,
            hits: hits.clone(),
        })
        .mount(&server)
        .await;
    let drive = Drive {
        http: reqwest::Client::new(),
        drivews: server.uri(),
        docws: server.uri(),
        params: vec![],
        retry_after: Duration::from_millis(2),
        free_space: setup.free_space.unwrap_or(|_| None),
        room_every: Duration::ZERO,
    };
    (server, drive, hits)
}

type Heard = Arc<Mutex<Vec<DownloadEvent>>>;

fn hearing() -> (Heard, Emit) {
    let heard: Heard = Arc::default();
    let emit: Emit = {
        let heard = heard.clone();
        Arc::new(move |e| heard.lock().unwrap().push(e))
    };
    (heard, emit)
}

async fn download(
    drive: &Drive,
    roots: &[&Node],
    into: &Path,
    journal: Option<&Journal>,
    emit: Emit,
    stop: &Stop,
) -> Result<DownloadReport, String> {
    let request = Request {
        batch: "b",
        title: "t",
        items: roots.iter().map(|n| item(n)).collect(),
        into,
        journal,
        backup: None,
    };
    run(drive, request, emit, stop).await
}

/// Every file and folder under `dir`, by its path inside it, with a file's bytes.
fn on_disk(dir: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, at: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in std::fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if std::fs::symlink_metadata(&path).unwrap().is_dir() {
                out.insert(format!("{rel}/"), None);
                walk(root, &path, out);
            } else {
                out.insert(rel, Some(std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn photos() -> Node {
    folder(
        "photos",
        "Photos",
        vec![
            file("beach", "beach", Some("jpg"), b"sand"),
            file("rc", "../../.bashrc", None, b"rm -rf"),
            file("con", "CON", Some("txt"), b"device"),
            file("p1", "Photo", Some("JPG"), b"upper"),
            file("p2", "photo", Some("jpg"), b"lower"),
            file("nfd", "Cafe\u{301}", Some("txt"), b"decomposed"),
            file("nfc", "Caf\u{e9}", Some("txt"), b"composed"),
            file("cjk", "写真 🎉", Some("png"), b"emoji"),
            file("flaky", "flaky", Some("bin"), b"second time lucky"),
            file("long", &"长".repeat(150), Some("pdf"), b"long"),
            folder(
                "trip",
                "Trip",
                vec![
                    file("map", "map", Some("pdf"), b"%PDF"),
                    folder("empty", "Empty", vec![]),
                    folder("up", "..", vec![file("x", "x", Some("txt"), b"x")]),
                    folder("odd", "a:b*c?", vec![]),
                    folder("dots", "trailing. . ", vec![]),
                ],
            ),
        ],
    )
}

/// A folder arrives as the same tree, every name made safe, clashes numbered, a busy moment
/// ridden out — beside a folder of the same name already in Downloads, untouched — and once it
/// is all there, nothing of it is kept to resume.
#[tokio::test]
async fn a_folder_arrives_as_the_same_tree() {
    let tree = photos();
    let (_server, drive, _) = drive_of(
        std::slice::from_ref(&tree),
        Setup {
            busy: vec![("flaky", 2)],
            ..Setup::default()
        },
    )
    .await;
    let downloads = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    std::fs::create_dir(downloads.path().join("Photos")).unwrap();
    std::fs::write(downloads.path().join("Photos/mine.txt"), b"mine").unwrap();
    let (heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        Some(&journal),
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();

    let long = format!("{}.pdf", "长".repeat(38));
    let mut want: BTreeMap<String, Option<Vec<u8>>> = [
        ("Photos/", None),
        ("Photos/mine.txt", Some(&b"mine"[..])),
        ("Photos 2/", None),
        ("Photos 2/beach.jpg", Some(b"sand")),
        ("Photos 2/bashrc", Some(b"rm -rf")),
        ("Photos 2/_CON.txt", Some(b"device")),
        ("Photos 2/Photo.JPG", Some(b"upper")),
        ("Photos 2/photo 2.jpg", Some(b"lower")),
        ("Photos 2/Caf\u{e9}.txt", Some(b"composed")),
        ("Photos 2/Caf\u{e9} 2.txt", Some(b"decomposed")),
        ("Photos 2/写真 🎉.png", Some(b"emoji")),
        ("Photos 2/flaky.bin", Some(b"second time lucky")),
        ("Photos 2/Trip/", None),
        ("Photos 2/Trip/map.pdf", Some(b"%PDF")),
        ("Photos 2/Trip/Empty/", None),
        ("Photos 2/Trip/Folder/", None),
        ("Photos 2/Trip/Folder/x.txt", Some(b"x")),
        ("Photos 2/Trip/abc/", None),
        ("Photos 2/Trip/trailing/", None),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.map(<[u8]>::to_vec)))
    .collect();
    want.insert(format!("Photos 2/{long}"), Some(b"long".to_vec()));
    assert_eq!(on_disk(downloads.path()), want);
    assert_eq!(
        report.saved,
        vec![downloads
            .path()
            .join("Photos 2")
            .to_string_lossy()
            .into_owned()]
    );
    assert!(
        report.failed.is_empty() && !report.cancelled && report.problem.is_none(),
        "{report:?}"
    );
    assert!(
        !report.resumable && journal.unfinished().is_empty(),
        "nothing kept once it is all there"
    );

    let heard = heard.lock().unwrap();
    let bytes: u64 = want.values().flatten().map(|b| b.len() as u64).sum::<u64>() - 4;
    let planned: Vec<_> = heard.iter().filter(|e| e.state == "planned").collect();
    assert_eq!(
        planned.first().map(|e| (e.sent, e.total)),
        Some((12, bytes)),
        "the total is known before a byte arrives"
    );
    let first_done = heard.iter().position(|e| e.state == "done").unwrap();
    assert!(heard.iter().position(|e| e.state == "planned").unwrap() < first_done);
    assert_eq!(heard.iter().filter(|e| e.state == "done").count(), 12);
    let map = heard
        .iter()
        .find(|e| e.name == "map.pdf" && e.state == "done")
        .unwrap();
    assert_eq!(
        (map.within.as_str(), map.sent, map.total),
        ("Photos 2/Trip", 4, 4)
    );
}

/// Files and folders chosen together land side by side in Downloads — here a Downloads that is
/// a link to a folder on another disk, as a moved Downloads often is.
#[cfg(unix)]
#[tokio::test]
async fn a_selection_lands_side_by_side_in_a_downloads_kept_elsewhere() {
    let notes = file("notes", "notes", Some("txt"), b"new notes");
    let docs = folder("docs", "Docs", vec![file("a", "a", Some("txt"), b"a")]);
    let (_server, drive, _) = drive_of(&[notes.clone(), docs.clone()], Setup::default()).await;
    let disk = tempfile::tempdir().unwrap();
    let other = disk.path().join("other-disk/Downloads");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("notes.txt"), b"old notes").unwrap();
    let downloads = disk.path().join("Downloads");
    std::os::unix::fs::symlink(&other, &downloads).unwrap();

    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&notes, &docs],
        &downloads,
        None,
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(other.join("notes.txt")).unwrap(),
        b"old notes"
    );
    assert_eq!(
        std::fs::read(other.join("notes 2.txt")).unwrap(),
        b"new notes"
    );
    assert_eq!(std::fs::read(other.join("Docs/a.txt")).unwrap(), b"a");
    assert_eq!(
        report.saved,
        vec![
            downloads.join("notes 2.txt").to_string_lossy().into_owned(),
            downloads.join("Docs").to_string_lossy().into_owned(),
        ]
    );
}

fn big() -> Node {
    folder(
        "big",
        "Big",
        vec![
            file("fast", "fast", Some("txt"), b"here"),
            folder(
                "later",
                "Later",
                vec![file("slow", "slow", Some("bin"), &[9; 2 << 20])],
            ),
        ],
    )
}

/// Cancel drops what is arriving and keeps what arrived; folders it made that are left empty
/// go again. Downloaded again — after a restart, read from the journal afresh — it goes on in
/// the same folder: what arrived is not fetched twice.
#[tokio::test]
async fn cancelled_it_keeps_what_arrived_and_resumes_where_it_was() {
    let tree = big();
    let downloads = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    {
        let (_server, drive, _) = drive_of(
            std::slice::from_ref(&tree),
            Setup {
                slow: vec!["slow"],
                ..Setup::default()
            },
        )
        .await;
        let stop = Stop::default();
        let (heard, emit) = hearing();
        // Cancel pressed while the slow one is on its way: its request is out, its bytes not back.
        let emit: Emit = {
            let stop = stop.clone();
            Arc::new(move |e: DownloadEvent| {
                let go = e.name == "slow.bin" && e.state == "downloading";
                emit(e);
                if go {
                    let stop = stop.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        stop.stop();
                    });
                }
            })
        };
        let journal = Journal::new(state.path().to_path_buf());
        let began = Instant::now();
        let report = download(
            &drive,
            &[&tree],
            downloads.path(),
            Some(&journal),
            emit,
            &stop,
        )
        .await
        .unwrap();
        assert!(began.elapsed() < Duration::from_secs(10), "stopped at once");
        assert!(
            report.cancelled && report.problem.is_none() && report.resumable,
            "{report:?}"
        );
        let disk = on_disk(downloads.path());
        assert_eq!(disk.get("Big/fast.txt"), Some(&Some(b"here".to_vec())));
        assert!(
            !disk
                .keys()
                .any(|k| k.contains("Later") || k.contains(".part")),
            "{disk:?}"
        );
        let slow: Vec<_> = heard
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.name == "slow.bin")
            .map(|e| e.state)
            .collect();
        assert_eq!(slow.last(), Some(&"cancelled"));
    }
    // Skykeep quit and opened again: only the journal on disk is left of it.
    let journal = Journal::new(state.path().to_path_buf());
    let unfinished = journal.unfinished();
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].title, "t");
    let (_server, drive, hits) = drive_of(std::slice::from_ref(&tree), Setup::default()).await;
    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        Some(&journal),
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert!(report.failed.is_empty() && !report.resumable, "{report:?}");
    let disk = on_disk(downloads.path());
    assert_eq!(
        disk.keys().filter(|k| k.starts_with("Big")).count(),
        4,
        "{:?}",
        disk.keys()
    );
    assert_eq!(
        disk.get("Big/Later/slow.bin")
            .map(|b| b.as_ref().map(Vec::len)),
        Some(Some(2 << 20))
    );
    assert!(
        !disk.keys().any(|k| k.starts_with("Big 2")),
        "the same folder, not a second one"
    );
    assert_eq!(
        hits.lock().unwrap().get("fast"),
        None,
        "what arrived before is not fetched again"
    );
    assert!(journal.unfinished().is_empty());
}

/// Resumed, a file there at another size than iCloud's — an older copy, or cut short by a
/// crash with no partial file of its own — is fetched again in its place, and partial files a
/// crash left behind are cleared.
#[tokio::test]
async fn resumed_a_wrong_sized_copy_is_replaced_and_leftovers_cleared() {
    let tree = folder(
        "r",
        "R",
        vec![
            file("a", "a", Some("txt"), b"right"),
            file("b", "b", Some("txt"), b"also right"),
        ],
    );
    let downloads = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    let top = downloads.path().join("R");
    std::fs::create_dir(&top).unwrap();
    std::fs::write(top.join("a.txt"), b"wrong!!").unwrap();
    std::fs::write(top.join("b.txt"), b"also right").unwrap();
    std::fs::write(top.join("a.txt.part"), b"crash").unwrap();
    std::fs::write(top.join("a.txt.part.1"), b"crash").unwrap();
    journal
        .save(&Record {
            key: Journal::key(&[item(&tree)]),
            title: "R".into(),
            items: vec![item(&tree)],
            placed: vec![Placed {
                drivewsid: wsid(&tree),
                path: top.clone(),
                folder: true,
            }],
            started_at: 1,
        })
        .unwrap();
    let (_server, drive, hits) = drive_of(std::slice::from_ref(&tree), Setup::default()).await;
    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        Some(&journal),
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(
        on_disk(downloads.path()).into_iter().collect::<Vec<_>>(),
        vec![
            ("R/".into(), None),
            ("R/a.txt".into(), Some(b"right".to_vec())),
            ("R/b.txt".into(), Some(b"also right".to_vec())),
        ]
    );
    assert_eq!(hits.lock().unwrap().get("b"), None);
}

/// A file changed in iCloud since the first run — the same size, a typo fixed — is fetched
/// again on resume; one that is the version iCloud has is left alone. Every downloaded file
/// carries the date it was last changed in iCloud, which is how the two are told apart.
#[tokio::test]
async fn resumed_a_file_changed_in_icloud_since_is_fetched_again() {
    const MONDAY: i64 = 1_793_000_000;
    const TUESDAY: i64 = MONDAY + 86_400;
    let mut edited = file("a", "a", Some("txt"), b"fixed");
    edited.changed = Some(TUESDAY);
    let mut same = file("b", "b", Some("txt"), b"as it was");
    same.changed = Some(MONDAY);
    let tree = folder("r", "R", vec![edited, same]);
    let downloads = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    let top = downloads.path().join("R");
    std::fs::create_dir(&top).unwrap();
    let at = |secs: i64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
    for (name, bytes) in [("a.txt", &b"fixd!"[..]), ("b.txt", &b"as it was"[..])] {
        std::fs::write(top.join(name), bytes).unwrap();
        let f = std::fs::File::options()
            .write(true)
            .open(top.join(name))
            .unwrap();
        f.set_modified(at(MONDAY)).unwrap();
    }
    journal
        .save(&Record {
            key: Journal::key(&[item(&tree)]),
            title: "R".into(),
            items: vec![item(&tree)],
            placed: vec![Placed {
                drivewsid: wsid(&tree),
                path: top.clone(),
                folder: true,
            }],
            started_at: 1,
        })
        .unwrap();
    let (_server, drive, hits) = drive_of(std::slice::from_ref(&tree), Setup::default()).await;
    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        Some(&journal),
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(std::fs::read(top.join("a.txt")).unwrap(), b"fixed");
    assert_eq!(
        hits.lock().unwrap().get("b"),
        None,
        "the current one is kept"
    );
    let changed = std::fs::metadata(top.join("a.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(changed, at(TUESDAY), "a download carries iCloud's date");
}

/// A journal edited to point outside Downloads is not followed: the download starts afresh.
#[tokio::test]
async fn a_journal_pointing_outside_downloads_is_not_followed() {
    let tree = folder("r", "R", vec![file("a", "a", Some("txt"), b"x")]);
    let downloads = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    journal
        .save(&Record {
            key: Journal::key(&[item(&tree)]),
            title: "R".into(),
            items: vec![item(&tree)],
            placed: vec![Placed {
                drivewsid: wsid(&tree),
                path: elsewhere.path().to_path_buf(),
                folder: true,
            }],
            started_at: 1,
        })
        .unwrap();
    let (_server, drive, _) = drive_of(std::slice::from_ref(&tree), Setup::default()).await;
    let (_heard, emit) = hearing();
    download(
        &drive,
        &[&tree],
        downloads.path(),
        Some(&journal),
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    assert_eq!(
        std::fs::read(downloads.path().join("R/a.txt")).unwrap(),
        b"x"
    );
}

/// A folder that cannot be read is said, and not left behind empty as if that were what it
/// held; everything else arrives — and once it can be read, resuming fetches only it.
#[tokio::test]
async fn a_folder_that_cannot_be_read_is_said_and_can_be_gone_back_for() {
    let tree = folder(
        "top",
        "Top",
        vec![
            file("ok", "ok", Some("txt"), b"fine"),
            folder(
                "broken",
                "Broken",
                vec![file("lost", "lost", Some("txt"), b"found")],
            ),
        ],
    );
    let downloads = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    {
        let (_server, drive, _) = drive_of(
            std::slice::from_ref(&tree),
            Setup {
                broken: vec!["broken"],
                ..Setup::default()
            },
        )
        .await;
        let (heard, emit) = hearing();
        let report = download(
            &drive,
            &[&tree],
            downloads.path(),
            Some(&journal),
            emit,
            &Stop::default(),
        )
        .await
        .unwrap();
        let disk = on_disk(downloads.path());
        assert_eq!(disk.keys().collect::<Vec<_>>(), ["Top/", "Top/ok.txt"]);
        assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
        assert!(
            report.failed[0].starts_with("Top/Broken: ") && report.failed[0].contains("busy"),
            "{:?}",
            report.failed
        );
        assert!(report.resumable);
        assert!(heard
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.folder && e.state == "failed" && e.name == "Broken"));
    }
    let (_server, drive, hits) = drive_of(std::slice::from_ref(&tree), Setup::default()).await;
    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        Some(&journal),
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert!(report.failed.is_empty() && !report.resumable, "{report:?}");
    assert_eq!(
        std::fs::read(downloads.path().join("Top/Broken/lost.txt")).unwrap(),
        b"found"
    );
    assert_eq!(hits.lock().unwrap().get("ok"), None);
}

/// A fresh sign-in needed stops the lot before anything is written.
#[tokio::test]
async fn a_lapsed_sign_in_stops_everything() {
    let tree = photos();
    let (_server, drive, _) = drive_of(
        std::slice::from_ref(&tree),
        Setup {
            signed_out: true,
            ..Setup::default()
        },
    )
    .await;
    let downloads = tempfile::tempdir().unwrap();
    let (_heard, emit) = hearing();
    let err = download(
        &drive,
        &[&tree],
        downloads.path(),
        None,
        emit,
        &Stop::default(),
    )
    .await
    .unwrap_err();
    assert!(err.contains("sign-in"), "{err}");
    assert!(on_disk(downloads.path()).is_empty());
}

/// A download bigger than the room there is is refused before a byte — or a folder — is
/// written, and says what it needs.
#[tokio::test]
async fn a_download_too_big_for_the_disk_is_refused_before_anything_is_written() {
    let tree = folder(
        "big",
        "Big",
        vec![folder(
            "in",
            "In",
            vec![file("f", "film", Some("mov"), &[7; 100])],
        )],
    );
    let (_server, drive, hits) = drive_of(
        std::slice::from_ref(&tree),
        Setup {
            free_space: Some(|_| Some(10)),
            ..Setup::default()
        },
    )
    .await;
    let downloads = tempfile::tempdir().unwrap();
    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        None,
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    let problem = report.problem.clone().unwrap_or_default();
    assert!(
        problem.contains("isn\u{2019}t room")
            && problem.contains("100 bytes")
            && problem.contains("10 bytes"),
        "{problem}"
    );
    assert!(!report.cancelled && report.saved.is_empty(), "{report:?}");
    assert!(on_disk(downloads.path()).is_empty());
    assert!(hits.lock().unwrap().is_empty(), "not a byte asked for");
}

static ROOM: AtomicU64 = AtomicU64::new(u64::MAX);

/// Room that runs out while it downloads — something else filling the disk — pauses it, with
/// what arrived kept and the rest resumable.
#[tokio::test]
async fn room_running_out_as_it_goes_pauses_it() {
    let tree = folder(
        "f",
        "F",
        (0..40)
            .map(|n| {
                file(
                    &format!("f{n}"),
                    &format!("{n:02}"),
                    Some("bin"),
                    &[1; 1000],
                )
            })
            .collect(),
    );
    let (_server, drive, _) = drive_of(
        std::slice::from_ref(&tree),
        Setup {
            free_space: Some(|_| Some(ROOM.load(Ordering::SeqCst))),
            ..Setup::default()
        },
    )
    .await;
    let downloads = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    let (_heard, emit) = hearing();
    // The disk fills up the moment the first file lands; the next look at it notices.
    let emit: Emit = Arc::new(move |e: DownloadEvent| {
        if e.state == "done" {
            ROOM.store(500, Ordering::SeqCst);
        }
        emit(e);
    });
    let stop = Stop::default();
    let report = tokio::time::timeout(
        Duration::from_secs(30),
        download(
            &drive,
            &[&tree],
            downloads.path(),
            Some(&journal),
            emit,
            &stop,
        ),
    )
    .await
    .expect("it ends")
    .unwrap();
    let problem = report.problem.clone().unwrap_or_default();
    assert!(problem.contains("running out of space"), "{report:?}");
    assert!(report.resumable && !report.cancelled);
}

/// A connection that stays lost pauses the download — it does not go on to fail every file —
/// and it can be gone on with.
#[tokio::test]
async fn a_lost_connection_pauses_rather_than_failing_everything() {
    let tree = folder(
        "f",
        "F",
        (0..60)
            .map(|n| file(&format!("f{n}"), &format!("{n:02}"), Some("bin"), b"x"))
            .collect(),
    );
    let (_server, drive, hits) = drive_of(
        std::slice::from_ref(&tree),
        Setup {
            busy_after: Some(5),
            ..Setup::default()
        },
    )
    .await;
    let downloads = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        Some(&journal),
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    let problem = report.problem.clone().unwrap_or_default();
    assert!(problem.contains("connection"), "{report:?}");
    assert!(report.resumable);
    let asked = hits.lock().unwrap().len();
    assert!(asked < 60, "stopped asking: {asked} of 60 files were tried");
}

/// Thousands of files: every one arrives; the reading never gets more than AHEAD files ahead of
/// the fetching; and a small file costs the window one event.
#[tokio::test]
async fn a_folder_of_thousands_arrives_whole_in_bounded_memory() {
    const MANY: usize = 1500;
    let tree = folder(
        "many",
        "Many",
        (0..MANY)
            .map(|n| file(&format!("f{n}"), &format!("file {n:04}"), Some("txt"), b"x"))
            .collect(),
    );
    let (_server, drive, _) = drive_of(std::slice::from_ref(&tree), Setup::default()).await;
    let downloads = tempfile::tempdir().unwrap();
    let (heard, emit) = hearing();
    let report = download(
        &drive,
        &[&tree],
        downloads.path(),
        None,
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(
        std::fs::read_dir(downloads.path().join("Many"))
            .unwrap()
            .count(),
        MANY
    );
    assert!(
        report.peak_waiting <= AHEAD + AT_ONCE + 1,
        "{} files waited at once",
        report.peak_waiting
    );
    let events = heard.lock().unwrap().len();
    assert!(
        events <= MANY + 50,
        "{events} events for {MANY} small files"
    );
}

/// A tree deeper than any real one is saved as deep as DEEPEST, the rest said.
#[tokio::test]
async fn a_tree_too_deep_is_saved_as_far_as_it_sensibly_goes() {
    let levels = DEEPEST + 4;
    let mut node = folder(
        &format!("d{levels}"),
        "z",
        vec![file("bottom", "bottom", Some("txt"), b"b")],
    );
    for n in (0..levels).rev() {
        let mut children = vec![node];
        if n == 10 {
            children.push(file("mid", "mid", Some("txt"), b"m"));
        }
        node = folder(&format!("d{n}"), "z", children);
    }
    let (_server, drive, _) = drive_of(std::slice::from_ref(&node), Setup::default()).await;
    let downloads = tempfile::tempdir().unwrap();
    let (_heard, emit) = hearing();
    let report = download(
        &drive,
        &[&node],
        downloads.path(),
        None,
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert!(report.failed[0].contains("too many folders deep"));
    let disk = on_disk(downloads.path());
    assert!(disk
        .keys()
        .any(|k| k.ends_with("/mid.txt") && k.matches('/').count() == 11));
    assert!(!disk.keys().any(|k| k.ends_with("bottom.txt")));
    let deepest = disk.keys().map(|k| k.matches('/').count()).max().unwrap();
    assert_eq!(deepest, DEEPEST, "{deepest} folders deep");
}

// ── At scale ────────────────────────────────────────────────────────────────────────────────

/// A Drive that keeps nothing: every listing and every file made up as it is asked for, so what
/// the scale test measures is the download's own memory. (wiremock keeps every request it
/// answers — a gigabyte by the hundred-thousandth file.) `folders` folders in `Archive`, each
/// of `each` two-byte files.
async fn synthetic_drive(folders: usize, each: usize) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let uri = base.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let _ = socket.set_nodelay(true);
            tokio::spawn(answer(socket, base.clone(), folders, each));
        }
    });
    uri
}

/// One connection, kept alive: HTTP/1.1 as reqwest speaks it, request after request.
async fn answer(mut socket: tokio::net::TcpStream, base: String, folders: usize, each: usize) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    let (reading, mut writing) = socket.split();
    let mut reading = tokio::io::BufReader::new(reading);
    loop {
        let mut line = String::new();
        if reading.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let target = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let mut length = 0usize;
        loop {
            let mut header = String::new();
            if reading.read_line(&mut header).await.unwrap_or(0) == 0 {
                return;
            }
            if header == "\r\n" {
                break;
            }
            if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                length = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; length];
        if reading.read_exact(&mut body).await.is_err() {
            return;
        }
        let reply = synthetic_reply(&target, &body, &base, folders, each);
        // One write: a head and a body sent apart wait on each other's acknowledgement.
        let mut whole = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            reply.len()
        )
        .into_bytes();
        whole.extend_from_slice(&reply);
        if writing.write_all(&whole).await.is_err() {
            return;
        }
    }
}

fn synthetic_reply(target: &str, body: &[u8], base: &str, folders: usize, each: usize) -> Vec<u8> {
    let entry = |kind: &str, id: String, name: String, ext: Option<&str>, size: i64| {
        json!({ "drivewsid": format!("{kind}::{ZONE}::{id}"), "name": name, "extension": ext,
                "type": kind, "size": size, "etag": "1" })
    };
    if target.starts_with("/retrieveItemDetailsInFolders") {
        let asked: Value = serde_json::from_slice(body).unwrap();
        let id = asked[0]["drivewsid"].as_str().unwrap().to_string();
        let items: Vec<Value> = match id.rsplit("::").next().unwrap() {
            "archive" => (0..folders)
                .map(|f| entry("FOLDER", format!("d{f}"), format!("{f:04}"), None, -1))
                .collect(),
            d => {
                let f: usize = d.trim_start_matches('d').parse().unwrap();
                (0..each)
                    .map(|n| {
                        entry(
                            "FILE",
                            format!("f{f}-{n}"),
                            format!("{n:03}"),
                            Some("bin"),
                            2,
                        )
                    })
                    .collect()
            }
        };
        serde_json::to_vec(&json!([{ "drivewsid": id, "status": "OK", "items": items }])).unwrap()
    } else if let Some(query) = target.strip_prefix(&format!("/ws/{ZONE}/download/by_id?")) {
        let doc = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("document_id="))
            .unwrap();
        serde_json::to_vec(&json!({ "data_token": { "url": format!("{base}/bytes/{doc}") } }))
            .unwrap()
    } else {
        b"ab".to_vec()
    }
}

/// This process's resident memory, in kilobytes, where the system says.
fn resident_kb() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .trim()
        .trim_end_matches(" kB")
        .parse()
        .ok()
}

/// The largest folders people keep: a hundred thousand files, in a thousand folders. Every one
/// arrives; the reading's lead stays bounded, the window hears about one event a file, and the
/// download's memory does not grow with the files it has fetched.
///
/// Memory is the process's own, so the measuring runs in a process of its own: this test runs
/// itself again alone (the same test binary, filtered to this one test), and other tests
/// running alongside in this process can't add to the number.
#[test]
fn a_hundred_thousand_files_in_bounded_memory() {
    const ALONE: &str = "SKYKEEP_SCALE_TEST_ALONE";
    if std::env::var_os(ALONE).is_some() {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap()
            .block_on(hundred_thousand_files());
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "folder_download::tests::a_hundred_thousand_files_in_bounded_memory",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(ALONE, "1")
        .output()
        .expect("run the scale test alone");
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "the scale test, run alone, failed:\n{said}"
    );
    assert!(
        said.contains("1 passed"),
        "the scale test didn't run alone:\n{said}"
    );
}

async fn hundred_thousand_files() {
    const FOLDERS: usize = 1000;
    const EACH: usize = 100;
    let uri = synthetic_drive(FOLDERS, EACH).await;
    let drive = Drive {
        http: reqwest::Client::new(),
        drivews: uri.clone(),
        docws: uri,
        params: vec![],
        retry_after: Duration::from_millis(2),
        free_space,
        room_every: ROOM_EVERY,
    };
    let archive = folder("archive", "Archive", vec![]);
    let downloads = tempfile::tempdir().unwrap();
    let events = Arc::new(AtomicUsize::new(0));
    // Memory once the first ten thousand files are in, when everything a download holds for
    // its whole length has been made.
    let early = Arc::new(AtomicU64::new(0));
    let emit: Emit = {
        let (events, early) = (events.clone(), early.clone());
        let done = AtomicUsize::new(0);
        Arc::new(move |e: DownloadEvent| {
            events.fetch_add(1, Ordering::Relaxed);
            if e.state == "done" && done.fetch_add(1, Ordering::Relaxed) + 1 == 10_000 {
                early.store(resident_kb().unwrap_or(0), Ordering::Relaxed);
            }
        })
    };
    let began = Instant::now();
    let report = download(
        &drive,
        &[&archive],
        downloads.path(),
        None,
        emit,
        &Stop::default(),
    )
    .await
    .unwrap();
    let took = began.elapsed();
    let late = resident_kb().unwrap_or(0);
    assert!(
        report.failed.is_empty(),
        "{:?}",
        &report.failed[..report.failed.len().min(5)]
    );
    let mut files = 0;
    for dir in std::fs::read_dir(downloads.path().join("Archive")).unwrap() {
        files += std::fs::read_dir(dir.unwrap().path()).unwrap().count();
    }
    assert_eq!(files, FOLDERS * EACH);
    assert!(
        report.peak_waiting <= AHEAD + AT_ONCE + 1,
        "{} files waited at once",
        report.peak_waiting
    );
    let heard = events.load(Ordering::Relaxed);
    assert!(
        heard <= FOLDERS * EACH + 100,
        "{heard} events for {} files",
        FOLDERS * EACH
    );
    let early = early.load(Ordering::Relaxed);
    eprintln!(
        "100k files in {took:?}: {heard} events, at most {} waiting, {early} kB resident after 10k files, {late} kB after 100k",
        report.peak_waiting
    );
    if early > 0 && late > 0 {
        // Ninety thousand more files fetched, and next to nothing more held: memory that grew
        // with the files would be hundreds of megabytes by now.
        assert!(
            late.saturating_sub(early) < 48 * 1024,
            "grew from {early} kB to {late} kB"
        );
    }
}

/// A disk that really fills up part way — not a free-space number made up for the test, but
/// writes the disk refuses (ENOSPC) — stops the download with words a person can act on: what
/// fit is saved whole, nothing half-written is left (no partial file, no part of the one that
/// didn't fit), and it can be gone on with once there is room.
///
/// Needs a small filesystem of its own, so it runs only when given one:
///   sudo mount -t tmpfs -o size=1m,uid=$(id -u) tmpfs /tmp/skykeep-full
///   SKYKEEP_FULL_DISK=/tmp/skykeep-full cargo test -p quince-app real_full_disk -- --ignored
#[tokio::test]
#[ignore = "needs a tiny filesystem: SKYKEEP_FULL_DISK (see the comment)"]
async fn a_real_full_disk_stops_it_cleanly() {
    let Some(small) = std::env::var_os("SKYKEEP_FULL_DISK") else {
        panic!("set SKYKEEP_FULL_DISK to a small, empty filesystem");
    };
    let downloads = tempfile::tempdir_in(&small).unwrap();
    let tree = folder(
        "f",
        "F",
        vec![
            file("small", "fits", Some("bin"), &[7; 200_000]),
            file("large", "too-big", Some("bin"), &[9; 3_000_000]),
        ],
    );
    // The space check before the start is told there is plenty: the disk itself has to say no.
    let (_server, drive, _) = drive_of(
        std::slice::from_ref(&tree),
        Setup {
            free_space: Some(|_| Some(u64::MAX / 4)),
            ..Setup::default()
        },
    )
    .await;
    let state = tempfile::tempdir().unwrap();
    let journal = Journal::new(state.path().to_path_buf());
    let (_heard, emit) = hearing();
    let report = tokio::time::timeout(
        Duration::from_secs(60),
        download(
            &drive,
            &[&tree],
            downloads.path(),
            Some(&journal),
            emit,
            &Stop::default(),
        ),
    )
    .await
    .expect("it ends")
    .unwrap();
    let problem = report.problem.clone().unwrap_or_default();
    assert!(problem.contains("ran out of space"), "{report:?}");
    assert!(report.resumable, "it can be gone on with: {report:?}");
    let failed = format!("{:?}", report.failed);
    assert!(
        failed.contains("space left on the disk"),
        "the file that didn't fit says why: {failed}"
    );
    let left = on_disk(downloads.path());
    assert_eq!(
        left.get("F/fits.bin").cloned().flatten().map(|b| b.len()),
        Some(200_000),
        "what fit is saved whole: {:?}",
        left.keys()
    );
    assert!(
        left.keys()
            .all(|k| !k.contains("too-big") && !k.contains(".part")),
        "nothing half-written is left: {:?}",
        left.keys()
    );
}

/// A backup (`Request::backup`) carries on in the same folder every time, never numbering a new
/// one; a file changed in iCloud since is fetched again, the copy it had moved first into the
/// earlier versions, at the same place in the tree under its date; one unchanged is left alone.
#[tokio::test]
async fn a_backup_keeps_the_version_it_had_and_its_own_folder() {
    const MONDAY: i64 = 1_793_000_000;
    const TUESDAY: i64 = MONDAY + 86_400;
    let mut edited = file("a", "a", Some("txt"), b"fixed");
    edited.changed = Some(TUESDAY);
    let mut same = file("b", "b", Some("txt"), b"as it was");
    same.changed = Some(MONDAY);
    let tree = folder("r", "R", vec![folder("d", "Docs", vec![edited]), same]);
    let backup = tempfile::tempdir().unwrap();
    let top = backup.path().join("iCloud Drive");
    let earlier = backup.path().join("Earlier versions");
    std::fs::create_dir_all(top.join("Docs")).unwrap();
    let at = |secs: i64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
    for (name, bytes) in [("Docs/a.txt", &b"fixd!"[..]), ("b.txt", &b"as it was"[..])] {
        std::fs::write(top.join(name), bytes).unwrap();
        let f = std::fs::File::options()
            .write(true)
            .open(top.join(name))
            .unwrap();
        f.set_modified(at(MONDAY)).unwrap();
    }
    let (_server, drive, hits) = drive_of(std::slice::from_ref(&tree), Setup::default()).await;
    let (_heard, emit) = hearing();
    for _ in 0..2 {
        let request = Request {
            batch: "b",
            title: "iCloud Drive",
            items: vec![item(&tree)],
            into: backup.path(),
            journal: None,
            backup: Some(Backup {
                top: &top,
                earlier: &earlier,
            }),
        };
        let report = run(&drive, request, emit.clone(), &Stop::default())
            .await
            .unwrap();
        assert!(report.failed.is_empty(), "{report:?}");
    }
    assert_eq!(std::fs::read(top.join("Docs/a.txt")).unwrap(), b"fixed");
    let kept: Vec<_> = std::fs::read_dir(earlier.join("iCloud Drive/Docs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(kept.len(), 1, "one earlier version: {kept:?}");
    assert!(
        kept[0].starts_with("a (") && kept[0].ends_with(").txt"),
        "{kept:?}"
    );
    assert_eq!(
        std::fs::read(earlier.join("iCloud Drive/Docs").join(&kept[0])).unwrap(),
        b"fixd!",
        "the version it had"
    );
    assert_eq!(
        hits.lock().unwrap().get("b"),
        None,
        "the unchanged one is left alone"
    );
    assert_eq!(
        hits.lock().unwrap().get("a").copied(),
        Some(1),
        "fetched once, not again"
    );
    let names: Vec<_> = std::fs::read_dir(backup.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        !names.iter().any(|n| n.starts_with("iCloud Drive ")),
        "no second folder: {names:?}"
    );
}
