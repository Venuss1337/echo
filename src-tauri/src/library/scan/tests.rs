//! Scan pipeline tests on real temporary directories with real tagged files.

use super::*;
use crate::contract::{TrackQuery, TrackSort};
use crate::library::{
    artwork::fixtures::png,
    meta::fixtures::{Spec, silent_wav, write},
    query,
};
use std::{fs, process::Command};

struct Lab {
    _dir: tempfile::TempDir,
    database: PathBuf,
    artwork: PathBuf,
    music: PathBuf,
}

impl Lab {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("Music");
        fs::create_dir_all(&music).unwrap();
        let lab = Self {
            database: dir.path().join("library.sqlite3"),
            artwork: dir.path().join("artwork"),
            music,
            _dir: dir,
        };
        library::open(&lab.database).unwrap();
        lab
    }

    fn db(&self) -> Connection {
        library::open(&self.database).unwrap()
    }

    fn add_root(&self, path: &Path) -> i64 {
        let normalized = source::open_directory(path).unwrap();
        index::add_root(&self.db(), &normalized.to_string_lossy(), 1).unwrap()
    }

    fn env(&self, cancel: Arc<AtomicBool>) -> ScanEnv {
        ScanEnv {
            database: self.database.clone(),
            artwork_dir: self.artwork.clone(),
            cancel,
            workers: 2,
            progress: Arc::new(|_| {}),
            queued: 0,
            on_batch: None,
            max_pages: None,
        }
    }

    fn scan(&self, root: i64) -> ScanOutcome {
        let path: String = self
            .db()
            .query_row("SELECT path FROM library_roots WHERE id=?1", [root], |r| {
                r.get(0)
            })
            .unwrap();
        run(
            &self.env(Arc::new(AtomicBool::new(false))),
            root,
            Path::new(&path),
        )
        .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.db().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

fn tagged(title: &str, artist: &str, album: &str, track: u32) -> Spec<'static> {
    // Leak is fine in tests: it keeps the borrowed Spec simple.
    Spec {
        title: Some(Box::leak(title.to_owned().into_boxed_str())),
        artist: Some(Box::leak(artist.to_owned().into_boxed_str())),
        album: Some(Box::leak(album.to_owned().into_boxed_str())),
        track: Some(track),
        ..Spec::default()
    }
}

fn junction(link: &Path, target: &Path) {
    let made = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/j"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(made.status.success(), "could not create test junction");
}

fn bump_mtime(path: &Path) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
}

#[test]
fn initial_scan_extracts_tags_artwork_relationships_and_search() {
    let lab = Lab::new();
    let cover = png(64, 64, 4);
    for (n, title) in ["Intro", "Middle", "Outro"].iter().enumerate() {
        let mut spec = tagged(title, "Nils Frahm", "Spaces", n as u32 + 1);
        spec.album_artist = Some("Nils Frahm");
        spec.picture = Some(cover.clone());
        write(&lab.music.join("Spaces").join(format!("{n}.wav")), &spec);
    }
    fs::write(lab.music.join("Spaces").join("cover.jpg"), b"not audio").unwrap();
    fs::write(lab.music.join("notes.txt"), b"ignore me").unwrap();
    fs::write(lab.music.join("bare.wav"), silent_wav()).unwrap();
    let root = lab.add_root(&lab.music);
    let result = lab.scan(root);
    assert_eq!(result.status, ScanStatus::Complete);
    assert_eq!(
        (result.discovered, result.indexed, result.failed),
        (4, 4, 0)
    );
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks"), 4);
    assert_eq!(lab.count("SELECT COUNT(*) FROM albums"), 1);
    assert_eq!(lab.count("SELECT COUNT(*) FROM artists"), 1);
    // Three identical covers share one cached thumbnail.
    assert_eq!(lab.count("SELECT COUNT(*) FROM artwork"), 1);
    let hash: String = lab
        .db()
        .query_row("SELECT hash FROM artwork", [], |r| r.get(0))
        .unwrap();
    assert!(artwork::load(&lab.artwork, &hash).is_ok());
    assert!(lab.count("SELECT COUNT(*) FROM albums WHERE artwork_id IS NOT NULL") == 1);
    let db = lab.db();
    let found = query::search(&db, "mid", 10).unwrap();
    assert_eq!(found.tracks.len(), 1);
    assert_eq!(found.tracks[0].album, "Spaces");
    let bare: String = db
        .query_row(
            "SELECT title FROM tracks WHERE rel_path='bare.wav'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bare, "bare");
    assert!(index::search_is_consistent(&db));
    assert!(lab.count("SELECT last_complete_scan_at IS NOT NULL FROM library_roots") == 1);
}

#[test]
fn rescan_skips_unchanged_files_and_rereads_only_what_changed() {
    let lab = Lab::new();
    for n in 0..12 {
        write(
            &lab.music.join(format!("{n}.wav")),
            &tagged(&format!("Song {n}"), "A", "LP", n + 1),
        );
    }
    let root = lab.add_root(&lab.music);
    assert_eq!(lab.scan(root).indexed, 12);
    let again = lab.scan(root);
    assert_eq!(
        (again.status, again.indexed, again.unchanged),
        (ScanStatus::Complete, 0, 12)
    );
    write(
        &lab.music.join("3.wav"),
        &tagged("Renamed Song", "A", "LP", 4),
    );
    bump_mtime(&lab.music.join("3.wav"));
    write(&lab.music.join("new.wav"), &tagged("Added", "B", "EP", 1));
    let third = lab.scan(root);
    assert_eq!((third.indexed, third.unchanged), (2, 11));
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks"), 13);
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks WHERE title='Renamed Song'"),
        1
    );
    assert!(index::search_is_consistent(&lab.db()));
    // The retagged file left no stale search entry behind.
    assert!(
        query::search(&lab.db(), "song 3", 10)
            .unwrap()
            .tracks
            .is_empty()
    );
}

#[test]
fn a_renamed_or_moved_file_keeps_its_track_identity() {
    let lab = Lab::new();
    write(&lab.music.join("old.wav"), &tagged("Keep Me", "A", "LP", 1));
    write(&lab.music.join("other.wav"), &tagged("Other", "A", "LP", 2));
    let root = lab.add_root(&lab.music);
    lab.scan(root);
    let id: i64 = lab
        .db()
        .query_row("SELECT id FROM tracks WHERE title='Keep Me'", [], |r| {
            r.get(0)
        })
        .unwrap();
    fs::create_dir(lab.music.join("moved")).unwrap();
    fs::rename(
        lab.music.join("old.wav"),
        lab.music.join("moved").join("new name.wav"),
    )
    .unwrap();
    let result = lab.scan(root);
    assert_eq!(result.status, ScanStatus::Complete);
    let (same_id, path, state): (i64, String, i64) = lab
        .db()
        .query_row(
            "SELECT id, rel_path, state FROM tracks WHERE title='Keep Me'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (same_id, path.as_str(), state),
        (id, r"moved\new name.wav", 0)
    );
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks"), 2);
}

#[test]
fn deleted_files_become_missing_never_deleted_and_recover_when_restored() {
    let lab = Lab::new();
    write(&lab.music.join("stay.wav"), &tagged("Stay", "A", "LP", 1));
    write(&lab.music.join("go.wav"), &tagged("Go", "A", "LP", 2));
    let root = lab.add_root(&lab.music);
    lab.scan(root);
    let saved = fs::read(lab.music.join("go.wav")).unwrap();
    fs::remove_file(lab.music.join("go.wav")).unwrap();
    let result = lab.scan(root);
    assert_eq!((result.status, result.missing), (ScanStatus::Complete, 1));
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks"),
        2,
        "no row is deleted"
    );
    assert_eq!(lab.count("SELECT state FROM tracks WHERE title='Go'"), 1);
    let page = query::tracks_page(
        &lab.db(),
        &TrackQuery {
            sort: TrackSort::Title,
            album_id: None,
            artist_id: None,
            offset: 0,
            limit: 10,
        },
    )
    .unwrap();
    assert!(
        !page
            .rows
            .iter()
            .find(|row| row.title == "Go")
            .unwrap()
            .available
    );
    fs::write(lab.music.join("go.wav"), saved).unwrap();
    // Restoring bytes with the original size but a new timestamp is a change.
    lab.scan(root);
    assert_eq!(lab.count("SELECT state FROM tracks WHERE title='Go'"), 0);
}

#[test]
fn duplicate_names_and_conflicting_metadata_are_never_merged() {
    let lab = Lab::new();
    for folder in ["a", "b"] {
        write(
            &lab.music.join(folder).join("01 - Intro.wav"),
            &tagged("Intro", "Same Artist", "Same Album", 1),
        );
    }
    let mut compilation = tagged("Track", "X", "Greatest Hits", 1);
    compilation.album_artist = Some("Band One");
    write(&lab.music.join("c").join("1.wav"), &compilation);
    compilation.album_artist = Some("Band Two");
    write(&lab.music.join("d").join("1.wav"), &compilation);
    write(
        &lab.music.join("e").join("DUP.wav"),
        &tagged("Case", "A", "LP", 1),
    );
    let root = lab.add_root(&lab.music);
    let result = lab.scan(root);
    assert_eq!(result.indexed, 5);
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks"),
        5,
        "equal names in two folders are two tracks"
    );
    // Same title, different album artist: two albums. Same untagged tags in two folders: two albums.
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM albums WHERE title='Greatest Hits'"),
        2
    );
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM albums WHERE title='Same Album'"),
        2
    );
}

#[test]
fn corrupt_and_malformed_files_are_counted_and_isolated() {
    let lab = Lab::new();
    write(&lab.music.join("good.wav"), &tagged("Good", "A", "LP", 1));
    fs::write(lab.music.join("junk.mp3"), b"definitely not an mp3").unwrap();
    fs::write(lab.music.join("empty.flac"), b"").unwrap();
    let mut cut = silent_wav();
    cut.truncate(20);
    fs::write(lab.music.join("cut.wav"), cut).unwrap();
    let root = lab.add_root(&lab.music);
    let result = lab.scan(root);
    assert_eq!(result.status, ScanStatus::Complete);
    assert!(result.failed >= 2, "{result:?}");
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks WHERE title='Good'"),
        1
    );
    // Unreadable files are never indexed, so a retry on the next scan stays possible.
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks WHERE rel_path IN ('junk.mp3','empty.flac')"),
        0
    );
    let again = lab.scan(root);
    assert_eq!(again.status, ScanStatus::Complete);
}

#[test]
fn bad_artwork_never_fails_the_track() {
    let lab = Lab::new();
    let mut garbage = tagged("Garbage Art", "A", "LP", 1);
    garbage.picture = Some(b"\x89PNG not really".to_vec());
    write(&lab.music.join("1.wav"), &garbage);
    let mut huge = tagged("Huge Art", "A", "LP", 2);
    huge.picture = Some(png(5000, 5000, 0));
    write(&lab.music.join("2.wav"), &huge);
    let mut fine = tagged("Fine Art", "A", "LP", 3);
    fine.picture = Some(png(32, 32, 2));
    write(&lab.music.join("3.wav"), &fine);
    write(&lab.music.join("4.wav"), &tagged("No Art", "A", "LP", 4));
    let root = lab.add_root(&lab.music);
    let result = lab.scan(root);
    assert_eq!((result.indexed, result.failed), (4, 0));
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks WHERE artwork_id IS NULL"),
        3
    );
    assert_eq!(lab.count("SELECT COUNT(*) FROM artwork"), 1);
}

#[test]
fn unicode_and_over_260_character_paths_are_indexed_and_openable() {
    let lab = Lab::new();
    write(
        &lab.music.join("日本語の音楽").join("Æther – ünïcode ♫.wav"),
        &tagged("Ünïcode ♫", "Björk", "Vespertine", 1),
    );
    let mut deep = lab.music.clone();
    while deep.as_os_str().len() < 300 {
        deep.push("segment-with-a-fairly-long-directory-name");
    }
    write(&deep.join("deep track.wav"), &tagged("Deep", "A", "LP", 1));
    let root = lab.add_root(&lab.music);
    let result = lab.scan(root);
    assert_eq!(
        (result.status, result.indexed, result.failed),
        (ScanStatus::Complete, 2, 0)
    );
    let rel: String = lab
        .db()
        .query_row("SELECT rel_path FROM tracks WHERE title='Deep'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(rel.len() > 260 - lab.music.as_os_str().len());
    // The stored path plus root resolves through the same checked-open used for playback.
    assert!(source::open_selected(&lab.music.join(&rel)).is_ok());
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks WHERE title='Ünïcode ♫'"),
        1
    );
    assert_eq!(
        query::search(&lab.db(), "unicode", 5).unwrap().tracks.len(),
        1
    );
}

#[test]
fn junction_loops_and_links_are_skipped_without_hanging() {
    let lab = Lab::new();
    write(
        &lab.music.join("real").join("song.wav"),
        &tagged("Real", "A", "LP", 1),
    );
    junction(&lab.music.join("real").join("back to root"), &lab.music);
    let outside = tempfile::tempdir().unwrap();
    write(
        &outside.path().join("elsewhere.wav"),
        &tagged("Outside", "A", "LP", 1),
    );
    junction(&lab.music.join("escape"), outside.path());
    let root = lab.add_root(&lab.music);
    let started = Instant::now();
    let result = lab.scan(root);
    for link in [
        lab.music.join("real").join("back to root"),
        lab.music.join("escape"),
    ] {
        fs::remove_dir(link).unwrap();
    }
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!((result.status, result.indexed), (ScanStatus::Complete, 1));
    assert!(result.skipped >= 2);
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks WHERE title='Outside'"),
        0
    );
}

#[test]
fn an_unreadable_directory_never_causes_its_tracks_to_be_flagged() {
    let lab = Lab::new();
    write(
        &lab.music.join("open").join("a.wav"),
        &tagged("Open", "A", "LP", 1),
    );
    write(
        &lab.music.join("locked").join("b.wav"),
        &tagged("Locked", "A", "LP", 1),
    );
    let root = lab.add_root(&lab.music);
    assert_eq!(lab.scan(root).indexed, 2);
    let locked = lab.music.join("locked");
    let acl = |args: &[&str]| {
        let out = Command::new("icacls")
            .arg(&locked)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "icacls {args:?}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    };
    acl(&["/deny", "*S-1-1-0:(RD)"]);
    let result = std::panic::catch_unwind(|| lab.scan(root));
    acl(&["/remove:d", "*S-1-1-0"]);
    let result = result.unwrap();
    assert_eq!(result.status, ScanStatus::Partial, "{result:?}");
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks WHERE state = 0"), 2);
}

#[test]
fn a_disconnected_or_emptied_root_keeps_every_row_and_recovers() {
    let lab = Lab::new();
    for n in 0..5 {
        write(
            &lab.music.join(format!("{n}.wav")),
            &tagged(&format!("T{n}"), "A", "LP", n + 1),
        );
    }
    let root = lab.add_root(&lab.music);
    lab.scan(root);
    let away = lab.music.with_file_name("Music-unplugged");
    fs::rename(&lab.music, &away).unwrap();
    let offline = lab.scan(root);
    assert_eq!(offline.status, ScanStatus::Offline);
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks WHERE state = 0"), 5);
    assert_eq!(lab.count("SELECT online FROM library_roots"), 0);
    let roots = query::roots(&lab.db()).unwrap();
    assert_eq!((roots[0].online, roots[0].unavailable_tracks), (false, 5));
    // A drive that reappears empty (wrong mount, failed medium) is not trusted either.
    fs::create_dir(&lab.music).unwrap();
    let emptied = lab.scan(root);
    assert_eq!(emptied.status, ScanStatus::Offline);
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks WHERE state = 0"), 5);
    fs::remove_dir(&lab.music).unwrap();
    fs::rename(&away, &lab.music).unwrap();
    let back = lab.scan(root);
    assert_eq!((back.status, back.unchanged), (ScanStatus::Complete, 5));
    assert_eq!(lab.count("SELECT online FROM library_roots"), 1);
}

#[test]
fn a_cancelled_scan_keeps_committed_work_and_flags_nothing_missing() {
    let lab = Lab::new();
    for n in 0..450 {
        write(
            &lab.music
                .join(format!("{:02}", n / 50))
                .join(format!("{n}.wav")),
            &tagged(&format!("T{n}"), "A", "LP", n % 20 + 1),
        );
    }
    let root = lab.add_root(&lab.music);
    let cancel = Arc::new(AtomicBool::new(false));
    let mut env = lab.env(cancel.clone());
    env.on_batch = Some(Arc::new({
        let cancel = cancel.clone();
        move |processed| {
            if processed >= 100 {
                cancel.store(true, Ordering::Release);
            }
        }
    }));
    let path = lab.music.to_string_lossy().into_owned();
    let cancelled = run(&env, root, Path::new(&path)).unwrap();
    assert_eq!(cancelled.status, ScanStatus::Cancelled);
    let partial = lab.count("SELECT COUNT(*) FROM tracks");
    assert!((100..450).contains(&partial), "{partial}");
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks WHERE state <> 0"), 0);
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM scans WHERE status = 'cancelled'"),
        1
    );
    let finished = lab.scan(root);
    assert_eq!(finished.status, ScanStatus::Complete);
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks"), 450);
    assert_eq!(finished.unchanged as i64, partial);
    assert!(index::search_is_consistent(&lab.db()));
}

#[test]
fn readers_and_a_checkpoint_writer_run_alongside_a_scan_without_errors() {
    let lab = Lab::new();
    for n in 0..300 {
        write(
            &lab.music.join(format!("{n}.wav")),
            &tagged(&format!("Concurrent {n}"), "Band", "LP", n % 12 + 1),
        );
    }
    let root = lab.add_root(&lab.music);
    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();
    for _ in 0..3 {
        let (database, stop) = (lab.database.clone(), stop.clone());
        threads.push(thread::spawn(move || {
            let reader = library::open_reader(&database).unwrap();
            let mut last_total = 0;
            let mut queries = 0_u32;
            while !stop.load(Ordering::Acquire) {
                let page = query::tracks_page(
                    &reader,
                    &TrackQuery {
                        sort: TrackSort::Title,
                        album_id: None,
                        artist_id: None,
                        offset: 0,
                        limit: 50,
                    },
                )
                .unwrap();
                assert!(page.total >= last_total, "a reader saw the library shrink");
                last_total = page.total;
                query::search(&reader, "concurrent", 20).unwrap();
                queries += 1;
            }
            queries
        }));
    }
    let checkpoint_writer = {
        let (database, stop) = (lab.database.clone(), stop.clone());
        thread::spawn(move || {
            let db = library::open(&database).unwrap();
            let mut writes = 0_u32;
            while !stop.load(Ordering::Acquire) {
                db.execute(
                    "INSERT INTO playback_session (id, payload) VALUES (1, ?1)
                     ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
                    [format!("{{\"n\":{writes}}}")],
                )
                .unwrap();
                writes += 1;
                thread::sleep(Duration::from_millis(5));
            }
            writes
        })
    };
    let result = lab.scan(root);
    stop.store(true, Ordering::Release);
    assert_eq!((result.status, result.indexed), (ScanStatus::Complete, 300));
    let queries: u32 = threads.into_iter().map(|t| t.join().unwrap()).sum();
    let writes = checkpoint_writer.join().unwrap();
    assert!(queries > 0 && writes > 0);
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks"), 300);
}

#[test]
fn restart_and_reopen_preserve_the_index_and_playback_session() {
    let lab = Lab::new();
    write(&lab.music.join("a.wav"), &tagged("Persist", "A", "LP", 1));
    let root = lab.add_root(&lab.music);
    lab.scan(root);
    lab.db()
        .execute(
            "INSERT INTO playback_session (id, payload) VALUES (1, 'keep')",
            [],
        )
        .unwrap();
    // Simulate a crash during a later scan, then reopen as a fresh process would.
    let db = lab.db();
    index::begin_scan(&db, root, 5).unwrap();
    drop(db);
    let db = lab.db();
    assert_eq!(index::repair_interrupted(&db, 6).unwrap(), 1);
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks"), 1);
    let payload: String = db
        .query_row("SELECT payload FROM playback_session", [], |r| r.get(0))
        .unwrap();
    assert_eq!(payload, "keep");
    let again = lab.scan(root);
    assert_eq!((again.status, again.unchanged), (ScanStatus::Complete, 1));
}

#[test]
fn a_full_disk_fails_the_scan_without_losing_or_flagging_anything() {
    let lab = Lab::new();
    for n in 0..20 {
        write(
            &lab.music.join(format!("{n}.wav")),
            &tagged(&format!("Kept {n}"), "A", "LP", n + 1),
        );
    }
    let root = lab.add_root(&lab.music);
    lab.scan(root);
    for n in 0..500 {
        let long = format!("Overflow {n} {}", "x".repeat(400));
        write(
            &lab.music.join(format!("new{n}.wav")),
            &tagged(&long, &long, &long, 1),
        );
    }
    let pages: u32 = lab.count("SELECT page_count FROM pragma_page_count") as u32;
    let mut env = lab.env(Arc::new(AtomicBool::new(false)));
    env.max_pages = Some(pages + 40);
    let path = lab.music.to_string_lossy().into_owned();
    let error = run(&env, root, Path::new(&path)).unwrap_err();
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DiskFull)
    );
    assert_eq!(
        lab.count("SELECT COUNT(*) FROM tracks WHERE title LIKE 'Kept%' AND state = 0"),
        20
    );
    assert!(index::search_is_consistent(&lab.db()));
    // With space available again the same scan completes.
    let recovered = lab.scan(root);
    assert_eq!(recovered.status, ScanStatus::Complete);
    assert_eq!(lab.count("SELECT COUNT(*) FROM tracks"), 520);
}

#[test]
fn a_directory_swapped_for_a_junction_after_listing_is_never_read() {
    let lab = Lab::new();
    let album = lab.music.join("album");
    write(&album.join("song.wav"), &tagged("Inside", "A", "LP", 1));
    let outside = tempfile::tempdir().unwrap();
    write(
        &outside.path().join("song.wav"),
        &tagged("Outside secret", "A", "LP", 1),
    );
    // What the walker queued while `album` was still a real directory.
    let job = || ReadJob {
        rel: r"album\song.wav".into(),
        abs: album.join("song.wav"),
        stamp: Stamp {
            size: 1,
            mtime_ns: 1,
        },
        folder_key: String::new(),
        moved_from: None,
    };
    let env = lab.env(Arc::new(AtomicBool::new(false)));
    let art = Mutex::new(HashMap::new());
    meta::configure_thread();
    assert_eq!(process(job(), &art, &env).unwrap().meta.title, "Inside");
    fs::rename(&album, lab.music.join("album-real")).unwrap();
    junction(&album, outside.path());
    let swapped = process(job(), &art, &env);
    fs::remove_dir(&album).unwrap();
    assert!(swapped.is_none());
}
