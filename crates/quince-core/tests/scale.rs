//! What a heavy library costs: synthetic photo libraries of 100,000 and 1,000,000 assets and
//! 50,000 notes, in a temporary folder, each call timed and the peak memory it took counted.
//! Run on purpose, not with every test:
//!
//! `SCALE=100000 cargo test -p quince-core --release --test scale -- --ignored --nocapture`
//!
//! `SCALE` is the number of photos (100,000 when unset); `SCALE_DIR` where the library is made
//! (a tmpfs is best; the system's temporary folder when unset).

use quince_core::photos::{Kind, Order, OrderBy, Photo, Source, View};
use quince_core::Library;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Counts the bytes held now and the most held since the count was last reset.
struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: passed straight to the system allocator with the layout given.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as allocated above.
        unsafe { System.dealloc(ptr, layout) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Run `f`, and say how long it took and the most memory it held beyond what was held before.
fn measure<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let started = Instant::now();
    let out = f();
    let took = started.elapsed();
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(before);
    println!(
        "{what:<44} {:>9.1} ms  {:>9.1} MB peak",
        took.as_secs_f64() * 1000.0,
        peak as f64 / 1_048_576.0
    );
    out
}

fn photo(i: usize) -> Photo {
    let taken = 1_500_000_000 + (i as i64) * 300;
    Photo {
        id: format!("A{i:08}-7B1C-4E2F-9A3D-{i:012}"),
        master: format!("M{i:08}-1C2D-4E5F-8A9B-{i:012}"),
        taken_at: taken,
        added_at: Some(taken + if i.is_multiple_of(7) { 86_400 * 30 } else { 60 }),
        kind: if i.is_multiple_of(20) {
            Kind::Video
        } else if i.is_multiple_of(9) {
            Kind::Live
        } else {
            Kind::Image
        },
        width: 4032,
        height: 3024,
        duration: i.is_multiple_of(20).then_some(12.5),
        filename: Some(format!("IMG_{:04}.HEIC", i % 10_000)),
        size: Some(2_400_000),
        uti: Some("public.heic".into()),
        favorite: i.is_multiple_of(50),
        hidden: i.is_multiple_of(997),
        deleted: i.is_multiple_of(991),
        deleted_at: i.is_multiple_of(991).then_some(taken + 10),
        expunge_at: None,
        title: i
            .is_multiple_of(100)
            .then(|| format!("Trip day {}", i % 31)),
        caption: i
            .is_multiple_of(250)
            .then(|| "Sunset over the bay".to_string()),
        rating: i.is_multiple_of(333) as i64 * 4,
        keywords: if i.is_multiple_of(400) {
            vec!["beach".into(), "family".into()]
        } else {
            Vec::new()
        },
        latitude: Some(17.7),
        longitude: Some(83.3),
        rendered: if i.is_multiple_of(40) { 8 } else { 0 },
        subtype: if i.is_multiple_of(60) { 3 } else { 0 },
        burst: (i % 1000 < 5).then(|| format!("B{}", i / 1000)),
        burst_pick: i.is_multiple_of(1000),
        burst_count: 0,
        live_size: None,
        change_tag: Some(format!("t{i}")),
    }
}

fn note(i: usize) -> quince_core::notes::Note {
    quince_core::notes::Note {
        guid: format!("N{i:010}"),
        folder_guid: None,
        title: format!("Note {i}"),
        snippet: Some("Groceries and things".into()),
        body: Some(format!(
            "Note {i}\nmilk eggs bread\nThe quick brown fox jumps {i} times.\n"
        )),
        locked: false,
        pinned: false,
        created_at: Some(1_700_000_000),
        modified_at: Some(1_700_000_000 + i as i64),
        editable: true,
        etag: Some(format!("e{i}")),
        runs: None,
    }
}

#[test]
#[ignore = "a benchmark: run it on purpose"]
fn a_heavy_library() {
    let n: usize = std::env::var("SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let base = std::env::var("SCALE_DIR").map_or_else(|_| std::env::temp_dir(), Into::into);
    let dir = tempfile::tempdir_in(base).unwrap();
    let mut lib = Library::open(dir.path()).unwrap();
    println!("\n{n} photos, 50000 notes, in {}", dir.path().display());

    measure("store every photo (first pass)", || {
        lib.atomically(|l| {
            for chunk in (0..n).collect::<Vec<_>>().chunks(5_000) {
                let batch: Vec<Photo> = chunk.iter().map(|&i| photo(i)).collect();
                l.put_photos(&batch)?;
            }
            Ok::<_, quince_core::Error>(())
        })
        .unwrap()
    });
    let again: Vec<Photo> = (0..50_000.min(n)).map(photo).collect();
    measure("store 50000 photos again, unchanged", || {
        lib.atomically(|l| l.put_photos(&again)).unwrap()
    });
    drop(again);

    let notes: Vec<_> = (0..50_000).map(note).collect();
    measure("store 50000 notes (first pass)", || {
        lib.atomically(|l| l.put_notes(&notes)).unwrap()
    });
    measure("store 50000 notes again, unchanged", || {
        lib.atomically(|l| l.put_notes(&notes)).unwrap()
    });
    drop(notes);

    let library = Source::View(View::Library);
    let taken = Some(Order {
        by: OrderBy::Taken,
        oldest: false,
    });
    let index = measure("photo_index (Library)", || {
        lib.photo_index(&library, taken).unwrap()
    });
    measure("photo_index (Recents)", || {
        lib.photo_index(&Source::View(View::Recents), None).unwrap()
    });
    measure("photo_index (Favorites)", || {
        lib.photo_index(&Source::View(View::Favorites), None)
            .unwrap()
    });
    measure("photo_index (Videos)", || {
        lib.photo_index(&Source::View(View::Videos), None).unwrap()
    });
    measure("photo_page first 200", || {
        lib.photo_page(&library, taken, None, 200).unwrap()
    });
    let mid = index.ids.len() / 2;
    let after = quince_core::photos::After {
        key: index.keys[mid],
        id: index.ids[mid].clone(),
    };
    measure("photo_page 200 from the middle", || {
        lib.photo_page(&library, taken, Some(&after), 200).unwrap()
    });
    let ids: Vec<String> = index.ids[mid..mid + 200].to_vec();
    measure("photos_by_ids 200", || lib.photos_by_ids(&ids).unwrap());
    drop(index);
    measure("photo_counts", || lib.photo_counts().unwrap());
    measure("photo_months", || lib.photo_months().unwrap());
    if std::env::var_os("SCALE_PLAN").is_some() {
        let mut st = lib
            .conn()
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id FROM photos WHERE +deleted = 0 AND +hidden = 0 AND id IN
                 (SELECT d.key FROM search_text CROSS JOIN search_docs d ON d.id = search_text.rowid
                  WHERE search_text MATCH '\"beach\"*' AND d.kind = 'photo')
                 ORDER BY taken_at DESC, id LIMIT 500",
            )
            .unwrap();
        let rows = st.query_map([], |r| r.get::<_, String>(3)).unwrap();
        println!("plan: {:?}", rows.collect::<Result<Vec<_>, _>>().unwrap());
    }
    measure("search_photos \"beach\"", || {
        lib.search_photos("beach", 500).unwrap()
    });
    measure("search_photos \"img_12\"", || {
        lib.search_photos("img_12", 500).unwrap()
    });
    measure("search_photos \"beach\" again", || {
        lib.search_photos("beach", 500).unwrap()
    });
    measure("search_photos \"sunset bay\"", || {
        lib.search_photos("sunset bay", 500).unwrap()
    });
    measure("whole library read as photos (old grid path)", || {
        lib.photos(View::Library).unwrap().len()
    });
    measure(
        "whole library read as shapes, a page at a time (grid path)",
        || {
            let (mut k, mut after) = (0usize, None);
            loop {
                let (page, next) = lib.grid_shapes(None, after.as_ref(), 2_000).unwrap();
                k += page.len();
                match next {
                    Some(n) => after = Some(n),
                    None => return k,
                }
            }
        },
    );
    measure("photo_months again, nothing changed", || {
        lib.photo_months().unwrap()
    });
    measure("newest 600 originals (keep_ahead)", || {
        lib.newest_photos(600).unwrap().len()
    });
    measure("every video's id (keep_ahead)", || {
        lib.video_ids().unwrap().len()
    });
    // A picture asked for while a pass stores: on its own read-only connection, as the app now
    // reads one, beside the pass's write (WAL) rather than behind it.
    let changed: Vec<Photo> = (0..50_000.min(n))
        .map(|i| {
            let mut p = photo(i);
            p.change_tag = Some(format!("u{i}"));
            p
        })
        .collect();
    let path = dir.path().to_path_buf();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let asking = {
        let done = done.clone();
        std::thread::spawn(move || {
            let reader = Library::open_reader(&path).unwrap();
            let mut waits = Vec::new();
            let mut i = 0usize;
            while !done.load(Ordering::Relaxed) || waits.len() < 50 {
                let started = Instant::now();
                reader.photo(&photo(i * 7919 % n).id).unwrap();
                waits.push(started.elapsed());
                i += 1;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            waits.sort();
            waits
        })
    };
    measure("store 50000 changed photos, a reader asking", || {
        lib.atomically(|l| l.put_photos(&changed)).unwrap()
    });
    done.store(true, Ordering::Relaxed);
    let waits = asking.join().unwrap();
    let p = |q: f64| waits[((waits.len() as f64 - 1.0) * q) as usize].as_secs_f64() * 1000.0;
    println!(
        "{:<44} p50 {:.2} ms  p99 {:.2} ms  max {:.2} ms ({} asks)",
        "a picture's photo read during that store",
        p(0.5),
        p(0.99),
        p(1.0),
        waits.len()
    );
    println!(
        "library file: {:.0} MB",
        std::fs::metadata(dir.path().join("library.db")).map_or(0, |m| m.len()) as f64
            / 1_048_576.0
    );
    if std::env::var_os("SCALE_KEEP").is_some() {
        println!("kept at {}", dir.keep().display());
    }
}
