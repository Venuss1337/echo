//! Scale tests and benchmarks.
//!
//! Two different things are measured and must not be conflated:
//!
//! * **Database-only**: synthetic [`TrackMeta`] rows written straight through the
//!   index layer. No files, no Lofty, no filesystem. This isolates SQLite, FTS5
//!   and query cost.
//! * **Filesystem scan**: real tagged audio files on disk, walked, opened
//!   through the checked-open boundary, parsed by Lofty and indexed.
//!
//! The default (non-ignored) tests assert correctness at 10,000 and 20,000
//! rows. The ignored tests print timings and are meant for `--release`:
//! `bun run test:library-scale`.

use super::{
    index::{self, Stamp, Upsert},
    meta::{
        TrackMeta,
        fixtures::{Spec, write},
    },
    query,
    scan::{self, ScanEnv},
};
use crate::contract::{AlbumQuery, AlbumSort, ArtistQuery, ScanStatus, TrackQuery, TrackSort};
use crate::library;
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use windows::Win32::System::{
    ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
    Threading::GetCurrentProcess,
};

const TRACKS_PER_ALBUM: usize = 12;
const ALBUMS_PER_ARTIST: usize = 10;

fn working_set_bytes() -> u64 {
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: the pseudo-handle is always valid and `counters` is sized by `cb`.
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb);
    }
    counters.WorkingSetSize as u64
}

fn mib(bytes: u64) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0
}

fn percentile(samples: &mut [Duration], fraction: f64) -> f64 {
    samples.sort();
    let index = ((samples.len() as f64 * fraction).ceil() as usize).saturating_sub(1);
    samples[index.min(samples.len() - 1)].as_secs_f64() * 1000.0
}

/// Deterministic metadata for track `n`: 12 tracks per album, 10 albums per
/// artist, with ASCII, accented and CJK names and a few empty tags.
fn synthetic(n: usize) -> (String, TrackMeta) {
    let album = n / TRACKS_PER_ALBUM;
    let artist = album / ALBUMS_PER_ARTIST;
    let names = [
        "Nocturne",
        "Lumière",
        "夜明け",
        "Harbour",
        "Cascade",
        "Œuvre",
        "Tidal",
        "Ember",
    ];
    let meta = TrackMeta {
        title: format!("{} {}", names[(n * 5) % names.len()], n),
        artists: vec![format!(
            "{} Ensemble {}",
            names[artist % names.len()],
            artist
        )],
        album_artist: Some(format!(
            "{} Ensemble {}",
            names[artist % names.len()],
            artist
        )),
        album: (!album.is_multiple_of(97))
            .then(|| format!("{} Collection {}", names[album % names.len()], album)),
        track_no: Some((n % TRACKS_PER_ALBUM) as u32 + 1),
        disc_no: Some(1),
        year: Some(1970 + (album % 50) as u32),
        genre: "Fixture".into(),
        duration_ms: 90_000 + (n as u64 % 400) * 1000,
        codec: "FLAC".into(),
        picture: None,
    };
    (format!("a{artist:04}\\b{album:05}\\{n:06}.flac"), meta)
}

fn seed(db: &mut Connection, root: i64, count: usize) -> Duration {
    let started = Instant::now();
    for chunk_start in (0..count).step_by(500) {
        let tx = db.transaction().unwrap();
        for n in chunk_start..(chunk_start + 500).min(count) {
            let (rel, meta) = synthetic(n);
            let folder = rel
                .rsplit_once('\\')
                .map_or("", |(dir, _)| dir)
                .to_lowercase();
            index::upsert_track(
                &tx,
                &Upsert {
                    root_id: root,
                    rel_path: &rel,
                    stamp: Stamp {
                        size: 1_000_000 + n as u64,
                        mtime_ns: n as i64,
                    },
                    meta: &meta,
                    artwork: None,
                    folder_key: &folder,
                    scan_id: 1,
                    moved_from: None,
                    now: 1,
                },
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    started.elapsed()
}

struct QueryTimings {
    page_p50: f64,
    page_p95: f64,
    deep_page_p95: f64,
    search_p50: f64,
    search_p95: f64,
    album_page_p95: f64,
    artist_page_p95: f64,
}

fn time_queries(db: &Connection, count: usize) -> QueryTimings {
    let tracks = |sort, offset| {
        query::tracks_page(
            db,
            &TrackQuery {
                sort,
                album_id: None,
                artist_id: None,
                offset,
                limit: 100,
            },
        )
        .unwrap()
    };
    let mut pages = Vec::new();
    let mut deep = Vec::new();
    for round in 0..60 {
        for sort in [
            TrackSort::Title,
            TrackSort::Artist,
            TrackSort::Album,
            TrackSort::Recent,
        ] {
            let started = Instant::now();
            let page = tracks(sort, ((round * 997) % (count / 100)) as u32 * 100 % 400);
            pages.push(started.elapsed());
            assert_eq!(page.total as usize, count);
            let started = Instant::now();
            let page = tracks(sort, (count as u32).saturating_sub(100));
            deep.push(started.elapsed());
            assert!(!page.rows.is_empty());
        }
    }
    let queries = [
        "noct",
        "lumi",
        "夜明",
        "harbour collection 4",
        "ensemble 1",
        "tidal 19",
        "œuvre",
        "collection 12",
    ];
    let mut searches = Vec::new();
    for round in 0..40 {
        for text in queries {
            let started = Instant::now();
            query::search(db, text, 24).unwrap();
            searches.push(started.elapsed());
            let _ = round;
        }
    }
    let mut albums = Vec::new();
    let mut artists = Vec::new();
    for round in 0..40_u32 {
        let started = Instant::now();
        query::albums_page(
            db,
            &AlbumQuery {
                sort: AlbumSort::Artist,
                artist_id: None,
                role: None,
                offset: (round * 37) % 1000,
                limit: 100,
            },
        )
        .unwrap();
        albums.push(started.elapsed());
        let started = Instant::now();
        query::artists_page(
            db,
            &ArtistQuery {
                offset: (round * 11) % 100,
                limit: 100,
            },
        )
        .unwrap();
        artists.push(started.elapsed());
    }
    QueryTimings {
        page_p50: percentile(&mut pages.clone(), 0.5),
        page_p95: percentile(&mut pages, 0.95),
        deep_page_p95: percentile(&mut deep, 0.95),
        search_p50: percentile(&mut searches.clone(), 0.5),
        search_p95: percentile(&mut searches, 0.95),
        album_page_p95: percentile(&mut albums, 0.95),
        artist_page_p95: percentile(&mut artists, 0.95),
    }
}

fn emit(kind: &str, count: usize, fields: serde_json::Value) {
    let line = serde_json::json!({ "kind": kind, "tracks": count, "result": fields });
    println!("LIBRARY_BENCH {line}");
    if let Some(path) = std::env::var_os("ECHO_BENCH_OUT") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{line}").unwrap();
    }
}

fn database_only(count: usize, timed: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.sqlite3");
    let mut db = library::open(&path).unwrap();
    let root = index::add_root(&db, r"C:\Bench", 1).unwrap();
    let before = working_set_bytes();
    let insert = seed(&mut db, root, count);
    let after = working_set_bytes();

    let counts = query::counts(&db).unwrap();
    assert_eq!(counts.tracks as usize, count);
    // Every 97th album is deliberately untagged and therefore has no album row.
    let albums = count.div_ceil(TRACKS_PER_ALBUM);
    assert_eq!(
        counts.albums as usize,
        (0..albums).filter(|a| !a.is_multiple_of(97)).count()
    );
    assert!(index::search_is_consistent(&db));

    // Pagination covers every row exactly once.
    let mut seen = std::collections::HashSet::with_capacity(count);
    for offset in (0..count).step_by(200) {
        let page = query::tracks_page(
            &db,
            &TrackQuery {
                sort: TrackSort::Title,
                album_id: None,
                artist_id: None,
                offset: offset as u32,
                limit: 200,
            },
        )
        .unwrap();
        for row in page.rows {
            assert!(seen.insert(row.id));
        }
    }
    assert_eq!(seen.len(), count);

    // Unicode and accent-insensitive search work at scale.
    assert!(!query::search(&db, "lumiere", 10).unwrap().tracks.is_empty());
    assert!(!query::search(&db, "夜明け", 10).unwrap().tracks.is_empty());
    // Ligatures such as Œ are letters of their own, not accented forms of "oe".
    assert!(!query::search(&db, "œuvre", 10).unwrap().tracks.is_empty());

    let timings = time_queries(&db, count);
    let started = Instant::now();
    index::rebuild_search(&mut db).unwrap();
    let rebuild = started.elapsed();
    assert!(index::search_is_consistent(&db));
    drop(db);
    let size = std::fs::metadata(&path).unwrap().len();
    if timed {
        emit(
            "database-only",
            count,
            serde_json::json!({
                "insertSeconds": insert.as_secs_f64(),
                "insertRowsPerSecond": (count as f64 / insert.as_secs_f64()).round(),
                "ftsRebuildSeconds": rebuild.as_secs_f64(),
                "pageP50Ms": timings.page_p50, "pageP95Ms": timings.page_p95,
                "lastPageP95Ms": timings.deep_page_p95,
                "searchP50Ms": timings.search_p50, "searchP95Ms": timings.search_p95,
                "albumPageP95Ms": timings.album_page_p95, "artistPageP95Ms": timings.artist_page_p95,
                "databaseMiB": mib(size),
                "workingSetGrowthMiB": mib(after.saturating_sub(before)),
                "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            }),
        );
    }
}

#[test]
fn database_only_10k_is_correct() {
    database_only(10_000, false);
}

#[test]
fn database_only_20k_is_correct() {
    database_only(20_000, false);
}

#[test]
#[ignore = "timed benchmark: run with --release via bun run test:library-scale"]
fn bench_database_only_10k_and_20k() {
    database_only(10_000, true);
    database_only(20_000, true);
}

/// Writes `count` real tagged WAV files into `root`, 12 per album folder.
fn generate_files(root: &Path, count: usize) -> Duration {
    let started = Instant::now();
    for n in 0..count {
        let album = n / TRACKS_PER_ALBUM;
        let artist = album / ALBUMS_PER_ARTIST;
        let title = format!("Fixture track {n}");
        let artist_name = format!("Fixture Ensemble {artist}");
        let album_name = format!("Fixture Collection {album}");
        write(
            &root
                .join(format!("a{artist:04}"))
                .join(format!("b{album:05}"))
                .join(format!("{n:06}.wav")),
            &Spec {
                title: Some(&title),
                artist: Some(&artist_name),
                album_artist: Some(&artist_name),
                album: Some(&album_name),
                track: Some((n % TRACKS_PER_ALBUM) as u32 + 1),
                disc: Some(1),
                year: Some(1970 + (album % 50) as u32),
                genre: Some("Fixture"),
                // A cover for most albums; many albums share the same bytes.
                picture: (!album.is_multiple_of(5))
                    .then(|| super::artwork::fixtures::png(48, 48, (album % 40) as u8)),
            },
        );
    }
    started.elapsed()
}

fn filesystem(count: usize, timed: bool) {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("Music");
    std::fs::create_dir_all(&music).unwrap();
    let generation = generate_files(&music, count);
    let database: PathBuf = dir.path().join("library.sqlite3");
    let db = library::open(&database).unwrap();
    let normalized = crate::playback::source::open_directory(&music).unwrap();
    let root = index::add_root(&db, &normalized.to_string_lossy(), 1).unwrap();
    drop(db);

    // Sample the working set while scanning to capture the peak, not the end.
    let peak = Arc::new(AtomicU64::new(0));
    let sampling = Arc::new(AtomicBool::new(true));
    let sampler = {
        let (peak, sampling) = (peak.clone(), sampling.clone());
        std::thread::spawn(move || {
            while sampling.load(Ordering::Acquire) {
                peak.fetch_max(working_set_bytes(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(25));
            }
        })
    };
    let baseline = working_set_bytes();
    let env = ScanEnv {
        database: database.clone(),
        artwork_dir: dir.path().join("artwork"),
        cancel: Arc::new(AtomicBool::new(false)),
        workers: std::thread::available_parallelism()
            .map_or(2, |n| n.get().saturating_sub(1).clamp(1, 4)),
        progress: Arc::new(|_| {}),
        queued: 0,
        on_batch: None,
        max_pages: None,
    };
    let started = Instant::now();
    let first = scan::run(&env, root, &normalized).unwrap();
    let initial = started.elapsed();
    let initial_peak = peak.load(Ordering::Relaxed);
    assert_eq!(first.status, ScanStatus::Complete);
    assert_eq!((first.indexed as usize, first.failed), (count, 0));

    peak.store(0, Ordering::Relaxed);
    let started = Instant::now();
    let second = scan::run(&env, root, &normalized).unwrap();
    let rescan = started.elapsed();
    let rescan_peak = peak.load(Ordering::Relaxed);
    sampling.store(false, Ordering::Release);
    sampler.join().unwrap();
    assert_eq!(second.status, ScanStatus::Complete);
    assert_eq!(
        (second.indexed, second.unchanged as usize),
        (0, count),
        "unchanged files are never re-read"
    );

    let db = library::open_reader(&database).unwrap();
    assert_eq!(query::counts(&db).unwrap().tracks as usize, count);
    assert!(
        !query::search(&db, "fixture track 77", 10)
            .unwrap()
            .tracks
            .is_empty()
    );
    let timings = time_queries(&db, count);
    if timed {
        emit(
            "filesystem-scan",
            count,
            serde_json::json!({
                "fixtureGenerationSeconds": generation.as_secs_f64(),
                "initialScanSeconds": initial.as_secs_f64(),
                "initialFilesPerSecond": (count as f64 / initial.as_secs_f64()).round(),
                "unchangedRescanSeconds": rescan.as_secs_f64(),
                "unchangedFilesPerSecond": (count as f64 / rescan.as_secs_f64()).round(),
                "scanWorkers": env.workers,
                "baselineWorkingSetMiB": mib(baseline),
                "initialScanPeakWorkingSetMiB": mib(initial_peak),
                "rescanPeakWorkingSetMiB": mib(rescan_peak),
                "searchP95Ms": timings.search_p95, "pageP95Ms": timings.page_p95,
                "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
            }),
        );
    }
}

#[test]
#[ignore = "timed benchmark: writes real files; run with --release via bun run test:library-scale"]
fn bench_filesystem_scan_10k() {
    filesystem(10_000, true);
}

#[test]
#[ignore = "timed benchmark: writes real files; run with --release via bun run test:library-scale"]
fn bench_filesystem_scan_20k() {
    filesystem(20_000, true);
}

/// A small always-on check that the real pipeline still scales linearly enough
/// to finish: 600 files must scan and rescan within a generous bound.
#[test]
fn filesystem_scan_600_files_completes_and_rescan_skips_all() {
    filesystem(600, false);
}

/// Artwork costs per cover, and playlist costs at ten thousand entries.
#[test]
#[ignore = "timed benchmark: run with --profile scale via bun run test:library-scale"]
fn bench_artwork_and_user_data() {
    use super::{artwork, palette, user};
    use crate::contract::PlaylistAct;
    let source = artwork::fixtures::png(3000, 3000, 5);
    let timed = |work: &mut dyn FnMut()| {
        let mut samples: Vec<Duration> = (0..15)
            .map(|_| {
                let started = Instant::now();
                work();
                started.elapsed()
            })
            .collect();
        percentile(&mut samples, 0.5)
    };
    let thumb = artwork::make_thumbnail(&source).unwrap();
    let before = working_set_bytes();
    let large = artwork::make_variant(&source, artwork::LARGE_EDGE).unwrap();
    let thumb_ms = timed(&mut || drop(artwork::make_thumbnail(&source).unwrap()));
    let large_ms =
        timed(&mut || drop(artwork::make_variant(&source, artwork::LARGE_EDGE).unwrap()));
    let palette_ms = timed(&mut || drop(palette::extract(&thumb.jpeg).unwrap()));
    let after = working_set_bytes();
    emit(
        "artwork-3000px-png",
        1,
        serde_json::json!({
            "thumb320Ms": thumb_ms, "large1024Ms": large_ms, "paletteMs": palette_ms,
            "largeKiB": large.jpeg.len() / 1024,
            "workingSetGrowthAfterMiB": mib(after.saturating_sub(before)),
        }),
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("user.sqlite3");
    let mut db = library::open(&path).unwrap();
    let root = index::add_root(&db, r"C:\Bench", 1).unwrap();
    seed(&mut db, root, 20_000);
    let ids: Vec<u32> = (1..=1000).collect();
    let list = user::act(
        &mut db,
        &PlaylistAct::Create {
            name: "Ten thousand".into(),
            description: String::new(),
            track_ids: vec![],
        },
        1,
    )
    .unwrap()
    .id;
    let started = Instant::now();
    for _ in 0..10 {
        user::act(
            &mut db,
            &PlaylistAct::AddTracks {
                id: list,
                track_ids: ids.clone(),
            },
            1,
        )
        .unwrap();
    }
    let add = started.elapsed().as_secs_f64();
    let first = user::playlist_entries(&db, list, 0, 1).unwrap().rows[0].entry_id;
    let last = user::playlist_entries(&db, list, 9_999, 1).unwrap().rows[0].entry_id;
    let mut flip = false;
    let move_ms = timed(&mut || {
        flip = !flip;
        user::act(
            &mut db,
            &PlaylistAct::Move {
                id: list,
                entry_id: last,
                before: flip.then_some(first),
            },
            1,
        )
        .unwrap();
    });
    let page_ms = timed(&mut || drop(user::playlist_entries(&db, list, 9_900, 100).unwrap()));
    let card_ms = timed(&mut || drop(user::playlists(&db).unwrap()));
    let started = Instant::now();
    for n in 0..5_000_u32 {
        user::record_play(&mut db, n % 900 + 1, i64::from(n), 31_000).unwrap();
    }
    let record_ms = started.elapsed().as_secs_f64() * 1000.0 / 5_000.0;
    let home_ms = timed(&mut || drop(user::home(&db).unwrap()));
    emit(
        "user-data",
        20_000,
        serde_json::json!({
            "addTenThousandSeconds": add, "moveEntryP50Ms": move_ms, "deepPageP50Ms": page_ms,
            "playlistSummaryP50Ms": card_ms, "recordPlayMeanMs": record_ms, "homeP50Ms": home_ms,
        }),
    );
}
