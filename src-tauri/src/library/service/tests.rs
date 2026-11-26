//! Coordinator tests with the real scheduler, scanner and watcher on temp directories.

use super::*;
use crate::contract::{ArtworkSize, ScanStatus, TrackSort};
use crate::library::{
    artwork::fixtures::png,
    meta::fixtures::{Spec, write},
};
use std::fs;

fn fast() -> Timing {
    Timing {
        debounce: Duration::from_millis(300),
        min_interval: Duration::from_millis(300),
        health: Duration::from_millis(300),
        reconcile: Duration::from_secs(3600),
        tick: Duration::from_millis(50),
    }
}

struct Lab {
    _dir: tempfile::TempDir,
    data: PathBuf,
    music: PathBuf,
}

impl Lab {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let music = dir.path().join("Music");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&music).unwrap();
        library::open(&data.join("library.sqlite3")).unwrap();
        Self {
            _dir: dir,
            data,
            music,
        }
    }

    fn service(&self) -> LibraryService {
        LibraryService::start_with(&self.data, fast()).unwrap()
    }

    fn song(&self, rel: &str, title: &str) {
        write(
            &self.music.join(rel),
            &Spec {
                title: Some(Box::leak(title.to_owned().into_boxed_str())),
                artist: Some("Band"),
                album: Some("LP"),
                track: Some(1),
                picture: Some(png(16, 16, 1)),
                ..Spec::default()
            },
        );
    }
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        thread::sleep(Duration::from_millis(40));
    }
    panic!("timed out waiting for {what}");
}

fn idle_with(service: &LibraryService, tracks: u32) -> bool {
    service
        .summary()
        .map(|s| s.scan.is_none() && s.tracks == tracks)
        .unwrap_or(false)
}

fn page(service: &LibraryService) -> Vec<crate::contract::TrackRow> {
    service
        .tracks(&TrackQuery {
            sort: TrackSort::Title,
            album_id: None,
            artist_id: None,
            offset: 0,
            limit: 100,
        })
        .unwrap()
        .rows
}

#[test]
fn adding_a_root_scans_in_the_background_and_reports_progress_and_results() {
    let lab = Lab::new();
    for n in 0..30 {
        lab.song(&format!("{n}.wav"), &format!("Song {n:02}"));
    }
    let service = lab.service();
    let seen = Arc::new(Mutex::new(Vec::<LibrarySummary>::new()));
    let sink = seen.clone();
    service
        .subscribe(Box::new(move |summary| {
            sink.lock().unwrap().push(summary.clone());
            true
        }))
        .unwrap();
    service.add_root(lab.music.clone()).unwrap();
    wait_for("initial scan", || idle_with(&service, 30));
    let summary = service.summary().unwrap();
    assert_eq!(
        (summary.roots.len(), summary.albums, summary.artists),
        (1, 1, 1)
    );
    let report = summary.roots[0].last_scan.as_ref().unwrap();
    assert_eq!((report.status, report.indexed), (ScanStatus::Complete, 30));
    let updates = seen.lock().unwrap();
    assert!(
        updates.iter().any(|u| u.scan.is_some()),
        "progress was published"
    );
    let revisions: Vec<u64> = updates
        .iter()
        .map(|u| u.revision.parse().unwrap())
        .collect();
    assert!(
        revisions.windows(2).all(|pair| pair[0] < pair[1]),
        "revisions only increase"
    );
    drop(updates);
    // Artwork is served from the cache by id.
    let row = &page(&service)[0];
    let bytes = service
        .artwork(row.artwork_id.unwrap(), ArtworkSize::Thumb)
        .unwrap();
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    assert_eq!(
        service.artwork(9999, ArtworkSize::Thumb),
        Err(LibraryFault::NotFound)
    );
}

#[test]
fn watcher_hints_trigger_a_reconciling_rescan() {
    let lab = Lab::new();
    lab.song("a.wav", "First");
    lab.song("b.wav", "Second");
    let service = lab.service();
    service.add_root(lab.music.clone()).unwrap();
    wait_for("initial scan", || idle_with(&service, 2));
    lab.song("c.wav", "Third");
    wait_for("added file", || idle_with(&service, 3));
    fs::remove_file(lab.music.join("a.wav")).unwrap();
    wait_for("removed file flagged", || {
        page(&service)
            .iter()
            .any(|row| row.title == "First" && !row.available)
    });
    // The row survives as an unavailable track instead of disappearing.
    assert_eq!(service.summary().unwrap().tracks, 3);
    assert_eq!(service.summary().unwrap().unavailable_tracks, 1);
}

#[test]
fn removing_a_root_forgets_the_index_but_never_touches_the_music() {
    let lab = Lab::new();
    for n in 0..120 {
        lab.song(&format!("{n}.wav"), &format!("S{n}"));
    }
    let service = lab.service();
    service.add_root(lab.music.clone()).unwrap();
    // Remove while the first scan is still running.
    thread::sleep(Duration::from_millis(100));
    let id = service.summary().unwrap().roots[0].id;
    service.remove_root(id).unwrap();
    let summary = service.summary().unwrap();
    assert_eq!(
        (
            summary.roots.len(),
            summary.tracks,
            summary.albums,
            summary.artists
        ),
        (0, 0, 0, 0)
    );
    assert!(summary.scan.is_none());
    assert_eq!(fs::read_dir(&lab.music).unwrap().count(), 120);
    assert_eq!(service.remove_root(id), Err(LibraryFault::NotFound));
    let db = library::open(&lab.data.join("library.sqlite3")).unwrap();
    assert!(index::search_is_consistent(&db));
    // Re-adding the same folder starts over cleanly.
    service.add_root(lab.music.clone()).unwrap();
    wait_for("rescan", || idle_with(&service, 120));
}

#[test]
fn roots_are_validated_and_overlaps_rejected() {
    let lab = Lab::new();
    lab.song("a.wav", "A");
    fs::create_dir_all(lab.music.join("Sub")).unwrap();
    let service = lab.service();
    assert_eq!(
        service.add_root(PathBuf::from(r"\\server\share\music")),
        Err(LibraryFault::UnsupportedLocation)
    );
    assert_eq!(
        service.add_root(PathBuf::from("relative")),
        Err(LibraryFault::UnsupportedLocation)
    );
    assert_eq!(
        service.add_root(lab.music.join("missing")),
        Err(LibraryFault::Invalid)
    );
    service.add_root(lab.music.clone()).unwrap();
    assert_eq!(
        service.add_root(lab.music.clone()),
        Err(LibraryFault::AlreadyAdded)
    );
    assert_eq!(
        service.add_root(lab.music.join("Sub")),
        Err(LibraryFault::Overlapping)
    );
    assert_eq!(service.summary().unwrap().roots.len(), 1);
}

#[test]
fn a_cancelled_scan_can_be_resumed_and_cancel_is_idempotent() {
    let lab = Lab::new();
    for n in 0..400 {
        lab.song(&format!("{n}.wav"), &format!("S{n}"));
    }
    let service = lab.service();
    service.add_root(lab.music.clone()).unwrap();
    let started = Instant::now();
    service.cancel_scan().unwrap();
    service.cancel_scan().unwrap();
    wait_for("scan stop", || service.summary().unwrap().scan.is_none());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancellation acknowledged promptly"
    );
    assert_eq!(service.summary().unwrap().unavailable_tracks, 0);
    service.scan(None).unwrap();
    wait_for("full scan", || idle_with(&service, 400));
}

#[test]
fn an_unplugged_root_goes_offline_with_its_tracks_and_returns_on_its_own() {
    let lab = Lab::new();
    for n in 0..6 {
        lab.song(&format!("{n}.wav"), &format!("S{n}"));
    }
    let service = lab.service();
    service.add_root(lab.music.clone()).unwrap();
    wait_for("scan", || idle_with(&service, 6));
    let away = lab.music.with_file_name("Music-away");
    fs::rename(&lab.music, &away).unwrap();
    wait_for("offline", || {
        let s = service.summary().unwrap();
        !s.roots[0].online && s.unavailable_tracks == 6
    });
    assert_eq!(service.summary().unwrap().tracks, 6);
    assert!(page(&service).iter().all(|row| !row.available));
    fs::rename(&away, &lab.music).unwrap();
    wait_for("back online", || {
        let s = service.summary().unwrap();
        s.roots[0].online && s.unavailable_tracks == 0 && s.scan.is_none()
    });
}

#[test]
fn playback_reports_flag_only_files_that_are_really_gone() {
    let lab = Lab::new();
    lab.song("here.wav", "Here");
    lab.song("gone.wav", "Gone");
    let service = lab.service();
    service.add_root(lab.music.clone()).unwrap();
    wait_for("scan", || idle_with(&service, 2));
    let rows = page(&service);
    let id_of = |title: &str| rows.iter().find(|r| r.title == title).unwrap().id;
    service.report_unavailable(id_of("Here"));
    fs::remove_file(lab.music.join("gone.wav")).unwrap();
    service.report_unavailable(id_of("Gone"));
    wait_for("flag", || {
        service.summary().unwrap().unavailable_tracks >= 1
    });
    thread::sleep(Duration::from_millis(400));
    let after = page(&service);
    assert!(after.iter().find(|r| r.title == "Here").unwrap().available);
    assert!(!after.iter().find(|r| r.title == "Gone").unwrap().available);
}

#[test]
fn restart_restores_roots_and_reconciles_changes_made_while_closed() {
    let lab = Lab::new();
    lab.song("a.wav", "A");
    {
        let service = lab.service();
        service.add_root(lab.music.clone()).unwrap();
        wait_for("scan", || idle_with(&service, 1));
        service.shutdown();
    }
    lab.song("b.wav", "B");
    let service = lab.service();
    wait_for("startup reconcile", || idle_with(&service, 2));
    assert_eq!(service.summary().unwrap().roots.len(), 1);
}
