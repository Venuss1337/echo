//! Library and playback running together against the staged native decoder.
//!
//! Ignored by default: it needs `ECHO_LOCAL_NATIVE_ROOT` (see
//! `native/windows-x64/test-library-playback.ps1`). It uses the null audio
//! output, so it proves coordination and state, not audible sound.

use super::{
    meta::fixtures::{Spec, silent_wav_of, write_with},
    service::{LibraryService, Timing},
};
use crate::{
    contract::{PlayTarget, PlayerState, PlayerView},
    native::NativeLibrary,
    playback::{
        decoder::Output,
        queue::{Item, TrackRef},
        service::{Command, Hooks, PickMode, Service},
    },
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn spec(title: &str) -> Spec<'static> {
    Spec {
        title: Some(Box::leak(title.to_owned().into_boxed_str())),
        artist: Some("Band"),
        album: Some("LP"),
        ..Spec::default()
    }
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    panic!("timed out waiting for {what}");
}

#[test]
#[ignore = "needs the staged native DLLs; run through test-library-playback.ps1"]
fn playing_a_library_track_survives_scanning_and_reports_vanished_files() {
    let root = PathBuf::from(std::env::var_os("ECHO_LOCAL_NATIVE_ROOT").unwrap());
    let manifest = std::fs::read_to_string(root.join("dlls.json")).unwrap();
    // SAFETY: the harness passes the exclusively staged, reviewed artifact.
    let native = Arc::new(unsafe { NativeLibrary::open(&root, &manifest) }.unwrap());

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let music = dir.path().join("Música 日本");
    std::fs::create_dir_all(&data).unwrap();
    super::open(&data.join("library.sqlite3")).unwrap();
    // A 20 second track outlasts every scan in this test.
    write_with(
        &music.join("Long.wav"),
        &spec("Long"),
        &silent_wav_of(160_000),
    );
    write_with(
        &music.join("Gone.wav"),
        &spec("Gone"),
        &silent_wav_of(8_000),
    );
    let filler = |n: usize| {
        write_with(
            &music.join(format!("f{n}.wav")),
            &spec(&format!("Filler {n}")),
            &silent_wav_of(2_000),
        )
    };
    (0..300).for_each(filler);

    let library = Arc::new(
        LibraryService::start_with(
            &data,
            Timing {
                debounce: Duration::from_millis(200),
                min_interval: Duration::from_millis(200),
                health: Duration::from_millis(300),
                reconcile: Duration::from_secs(3600),
                tick: Duration::from_millis(50),
            },
        )
        .unwrap(),
    );
    library.add_root(music.clone()).unwrap();
    let idle = |tracks: u32| {
        let s = library.summary().unwrap();
        s.scan.is_none() && s.tracks == tracks
    };
    wait_until("first scan", || idle(302));

    let hooks = {
        let library = library.clone();
        let heard = library.clone();
        Hooks {
            unavailable: Some(Arc::new(move |track| library.report_unavailable(track))),
            listened: Some(Arc::new(move |event| heard.report_listened(event))),
            system: None,
        }
    };
    let player =
        Service::start_with_hooks(native, data.join("library.sqlite3"), Output::Null, hooks)
            .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let request = |command| runtime.block_on(player.request(command));
    let id_of = |title: &str| {
        library
            .search(title, 10)
            .unwrap()
            .tracks
            .into_iter()
            .find(|row| row.title == title)
            .unwrap()
            .id
    };
    let queue_item = |title: &str| -> Vec<Item> {
        let plan = library
            .play_plan(&PlayTarget::Track { id: id_of(title) })
            .unwrap();
        plan.items
            .into_iter()
            .map(|item| {
                (
                    item.path,
                    Some(TrackRef {
                        track_id: item.track_id,
                        title: item.title,
                        artist: item.artist,
                        artwork_id: item.artwork_id,
                    }),
                )
            })
            .collect()
    };

    let view = request(Command::AddLibrary {
        items: queue_item("Long"),
        mode: PickMode::Listen,
        start: 0,
        shuffle: None,
    })
    .unwrap();
    assert_eq!(view.tracks[0].title, "Long");
    assert_eq!(view.tracks[0].subtitle.as_deref(), Some("Band"));
    let playing = |view: &PlayerView| view.state == PlayerState::Playing && view.position_ms > 0.0;
    wait_until("playback", || playing(&request(Command::Snapshot).unwrap()));

    // Grow the library and rescan while the track plays.
    (300..600).for_each(filler);
    library.scan(None).unwrap();
    let mut last = 0.0;
    wait_until("rescan with playback", || {
        let view = request(Command::Snapshot).unwrap();
        assert_eq!(
            view.state,
            PlayerState::Playing,
            "scanning interrupted playback"
        );
        assert!(view.position_ms >= last, "position moved backwards");
        last = view.position_ms;
        idle(602)
    });

    // A file that vanishes after indexing is reported and flagged.
    std::fs::remove_file(music.join("Gone.wav")).unwrap();
    let failed = request(Command::AddLibrary {
        items: queue_item("Gone"),
        mode: PickMode::Listen,
        start: 0,
        shuffle: None,
    });
    assert!(failed.is_err());
    wait_until("missing flag", || {
        library.summary().unwrap().unavailable_tracks >= 1
    });
    assert_eq!(
        request(Command::Snapshot).unwrap().state,
        PlayerState::Error
    );

    player.shutdown().unwrap();
    library.shutdown();
}
