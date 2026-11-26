use super::*;
use crate::library::{
    self,
    index::{self, Stamp, Upsert, fixtures::meta},
    meta::TrackMeta,
};

struct Lab {
    dir: tempfile::TempDir,
    db: Connection,
}

impl Lab {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db = library::open(&dir.path().join("u.sqlite3")).unwrap();
        Self { dir, db }
    }

    fn root(&self, path: &str) -> i64 {
        index::add_root(&self.db, path, 1).unwrap()
    }

    fn add_as(&mut self, root: i64, rel: &str, meta: &TrackMeta, stamp: Stamp) -> u32 {
        let tx = self.db.transaction().unwrap();
        let id = index::upsert_track(
            &tx,
            &Upsert {
                root_id: root,
                rel_path: rel,
                stamp,
                meta,
                artwork: None,
                folder_key: "f",
                scan_id: 1,
                moved_from: None,
                now: 1,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id32(id)
    }

    fn add(&mut self, root: i64, rel: &str) -> u32 {
        let size = 1000 + rel.bytes().map(u64::from).sum::<u64>();
        self.add_as(
            root,
            rel,
            &meta(rel, &["Band"], Some("LP")),
            Stamp { size, mtime_ns: 77 },
        )
    }

    fn act(&mut self, action: PlaylistAct) -> Result<PlaylistOutcome, UserError> {
        act(&mut self.db, &action, 100)
    }

    fn create(&mut self, tracks: &[u32]) -> u32 {
        self.act(PlaylistAct::Create {
            name: "Mix".into(),
            description: String::new(),
            track_ids: tracks.to_vec(),
        })
        .unwrap()
        .id
    }

    fn entries(&self, id: u32) -> Vec<PlaylistEntry> {
        playlist_entries(&self.db, id, 0, 200).unwrap().rows
    }

    fn titles(&self, id: u32) -> Vec<String> {
        self.entries(id)
            .into_iter()
            .map(|e| e.media.title)
            .collect()
    }
}

fn move_to(id: u32, entry_id: u32, before: Option<u32>) -> PlaylistAct {
    PlaylistAct::Move {
        id,
        entry_id,
        before,
    }
}

#[test]
fn playlists_keep_duplicates_with_their_own_identity_and_order() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let (a, b) = (lab.add(root, "a"), lab.add(root, "b"));
    let list = lab.create(&[a, b, a, a]);
    let rows = lab.entries(list);
    assert_eq!(lab.titles(list), ["a", "b", "a", "a"]);
    let ids: std::collections::HashSet<_> = rows.iter().map(|e| e.entry_id).collect();
    assert_eq!(ids.len(), 4);
    assert_eq!(rows[0].media.source_id, rows[2].media.source_id);
    // Removing one occurrence leaves the others.
    let removed = lab
        .act(PlaylistAct::Remove {
            id: list,
            entry_ids: vec![rows[2].entry_id],
        })
        .unwrap();
    assert_eq!(removed.changed, 1);
    assert_eq!(lab.titles(list), ["a", "b", "a"]);
    lab.act(move_to(list, rows[1].entry_id, Some(rows[0].entry_id)))
        .unwrap();
    assert_eq!(lab.titles(list), ["b", "a", "a"]);
    lab.act(move_to(list, rows[1].entry_id, None)).unwrap();
    assert_eq!(lab.titles(list), ["a", "a", "b"]);
    let card = playlist(&lab.db, list).unwrap().unwrap();
    assert_eq!((card.tracks, card.duration_ms), (3, 540_000.0));
}

#[test]
fn repeated_moves_renumber_without_losing_or_reordering_anything() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let tracks: Vec<u32> = (0..12).map(|n| lab.add(root, &format!("{n:02}"))).collect();
    let list = lab.create(&tracks);
    // Always squeezing into the first gap exhausts it and forces renumbering.
    let mut model: Vec<u32> = lab.entries(list).iter().map(|e| e.entry_id).collect();
    for round in 0..400 {
        let moved = model.remove(2 + (round * 7) % (model.len() - 2));
        model.insert(1, moved);
        lab.act(move_to(list, moved, Some(model[2]))).unwrap();
    }
    let actual: Vec<u32> = lab.entries(list).iter().map(|e| e.entry_id).collect();
    assert_eq!(actual, model);
}

#[test]
fn invalid_mutations_change_nothing() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let a = lab.add(root, "a");
    let list = lab.create(&[a]);
    let other = lab.create(&[a]);
    let foreign = lab.entries(other)[0].entry_id;
    assert_eq!(
        lab.act(PlaylistAct::AddTracks {
            id: list,
            track_ids: vec![a, 9999]
        }),
        Err(UserError::NotFound),
        "one unknown id rolls back the whole batch"
    );
    assert_eq!(lab.entries(list).len(), 1);
    let own = lab.entries(list)[0].entry_id;
    assert_eq!(
        lab.act(move_to(list, foreign, None)),
        Err(UserError::NotFound)
    );
    assert_eq!(
        lab.act(move_to(list, own, Some(foreign))),
        Err(UserError::NotFound)
    );
    let removed = lab
        .act(PlaylistAct::Remove {
            id: list,
            entry_ids: vec![foreign],
        })
        .unwrap();
    assert_eq!(removed.changed, 0);
    for name in ["", "   ", "\u{202e}\u{0}"] {
        assert_eq!(
            lab.act(PlaylistAct::Edit {
                id: list,
                name: name.into(),
                description: String::new()
            }),
            Err(UserError::Invalid)
        );
    }
    assert_eq!(
        lab.act(PlaylistAct::Delete { id: 999 }),
        Err(UserError::NotFound)
    );
    assert_eq!(
        lab.act(PlaylistAct::AddTracks {
            id: list,
            track_ids: vec![a; MAX_BATCH + 1]
        }),
        Err(UserError::Invalid)
    );
    assert_eq!(lab.titles(list), ["a"]);
    assert_eq!(playlist(&lab.db, list).unwrap().unwrap().name, "Mix");
}

#[test]
fn an_interrupted_transaction_leaves_the_previous_order() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let tracks: Vec<u32> = (0..5).map(|n| lab.add(root, &format!("{n}"))).collect();
    let list = lab.create(&tracks);
    let path = lab.dir.path().join("u.sqlite3");
    {
        let mut second = library::open(&path).unwrap();
        let tx = immediate(&mut second).unwrap();
        renumber(&tx, i64::from(list)).unwrap();
        tx.execute(
            "DELETE FROM playlist_entries WHERE playlist_id = ?1",
            [list],
        )
        .unwrap();
        // Dropped without commit, as a crash would.
    }
    assert_eq!(lab.titles(list), ["0", "1", "2", "3", "4"]);
}

#[test]
fn concurrent_mutations_serialize_without_lost_entries() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let a = lab.add(root, "a");
    let list = lab.create(&[]);
    let path = lab.dir.path().join("u.sqlite3");
    std::thread::scope(|scope| {
        for _ in 0..4 {
            let path = path.clone();
            scope.spawn(move || {
                let mut db = library::open(&path).unwrap();
                for _ in 0..25 {
                    act(
                        &mut db,
                        &PlaylistAct::AddTracks {
                            id: list,
                            track_ids: vec![a],
                        },
                        1,
                    )
                    .unwrap();
                }
            });
        }
    });
    assert_eq!(lab.entries(list).len(), 100);
}

#[test]
fn removing_a_root_keeps_user_data_as_readable_tombstones() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let (a, b) = (lab.add(root, "a"), lab.add(root, "b"));
    let list = lab.create(&[a, b, a]);
    set_favorite(&mut lab.db, FavoriteTarget::Track { id: a }, true, 5).unwrap();
    record_play(&mut lab.db, a, 50, 31_000).unwrap().unwrap();
    let album = query::track(&lab.db, a).unwrap().unwrap().album_id.unwrap();
    set_favorite(&mut lab.db, FavoriteTarget::Album { id: album }, true, 5).unwrap();
    // Metadata that changed after the source was created is snapshotted too.
    lab.db
        .execute("UPDATE tracks SET title = 'A renamed' WHERE id = ?1", [a])
        .unwrap();
    index::remove_root(&mut lab.db, root).unwrap();
    let rows = lab.entries(list);
    assert_eq!(lab.titles(list), ["A renamed", "b", "A renamed"]);
    assert!(rows.iter().all(|e| !e.media.available
        && e.media.track_id.is_none()
        && e.media.album == "LP"
        && e.media.artist == "Band"));
    let favorites = favorite_tracks(&lab.db, 0, 10).unwrap();
    assert_eq!(
        (favorites.total, favorites.rows[0].title.as_str()),
        (1, "A renamed")
    );
    assert_eq!(history(&lab.db, 0, 10).unwrap().total, 1);
    let albums = favorite_albums(&lab.db, 0, 10).unwrap();
    assert_eq!(
        (albums[0].title.as_str(), albums[0].album.is_none()),
        ("LP", true)
    );
    // Nothing unavailable is queued, and a stale queue id still resolves.
    let plan = query::play_plan(
        &lab.db,
        &crate::contract::PlayTarget::Playlist {
            id: list,
            start_entry: None,
        },
    )
    .unwrap();
    assert!(plan.items.is_empty());
    assert!(record_play(&mut lab.db, a, 60, 31_000).unwrap().is_some());
    set_favorite(
        &mut lab.db,
        FavoriteTarget::Source {
            id: favorites.rows[0].source_id,
        },
        false,
        6,
    )
    .unwrap();
    assert_eq!(favorite_tracks(&lab.db, 0, 10).unwrap().total, 0);
}

#[test]
fn sources_return_only_on_strong_evidence() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let stamp = |size, mtime_ns| Stamp { size, mtime_ns };
    let song = |title: &str| meta(title, &["Band"], Some("LP"));
    let same = lab.add_as(root, "same.flac", &song("Same"), stamp(10, 1));
    let tagged = lab.add_as(root, "tagged.flac", &song("Tagged"), stamp(20, 2));
    let replaced = lab.add_as(root, "replaced.flac", &song("Replaced"), stamp(30, 3));
    let moved = lab.add_as(root, r"d\moved.flac", &song("Moved"), stamp(40, 4));
    let titled = lab.add_as(root, "titled.flac", &song("Common title"), stamp(50, 5));
    let list = lab.create(&[same, tagged, replaced, moved, titled]);
    index::remove_root(&mut lab.db, root).unwrap();

    let again = lab.root(r"C:\Music");
    lab.add_as(again, "same.flac", &song("Same"), stamp(10, 1));
    // Re-tagged in place: new size and time, same title and duration.
    lab.add_as(again, "tagged.flac", &song("tagged"), stamp(21, 9));
    // A different recording now occupies the path.
    let mut impostor = song("Something else");
    impostor.duration_ms = 5_000;
    lab.add_as(again, "replaced.flac", &impostor, stamp(31, 9));
    // Same title and artist elsewhere is not evidence.
    lab.add_as(
        again,
        r"elsewhere\x.flac",
        &song("Common title"),
        stamp(51, 9),
    );
    let drive = lab.root(r"E:\Music");
    lab.add_as(drive, r"d\moved.flac", &song("Moved"), stamp(40, 4));

    let available: Vec<(String, bool)> = lab
        .entries(list)
        .into_iter()
        .map(|e| (e.media.title, e.media.available))
        .collect();
    let expected = [
        ("Same", true),
        ("tagged", true),
        ("Replaced", false),
        ("Moved", true),
        ("Common title", false),
    ];
    for ((title, on), (want, want_on)) in available.iter().zip(expected) {
        assert_eq!((title.as_str(), *on), (want, want_on));
    }
}

#[test]
fn ambiguous_moves_stay_unavailable() {
    let mut lab = Lab::new();
    let (one, two) = (lab.root(r"C:\A"), lab.root(r"C:\B"));
    let stamp = Stamp {
        size: 9,
        mtime_ns: 9,
    };
    let m = meta("Twin", &["Band"], None);
    let a = lab.add_as(one, "t.flac", &m, stamp);
    let b = lab.add_as(two, "t.flac", &m, stamp);
    let list = lab.create(&[a, b]);
    index::remove_root(&mut lab.db, one).unwrap();
    index::remove_root(&mut lab.db, two).unwrap();
    let third = lab.root(r"D:\C");
    lab.add_as(third, "t.flac", &m, stamp);
    assert!(lab.entries(list).iter().all(|e| !e.media.available));
}

#[test]
fn favorites_are_idempotent_and_visible_on_library_rows() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let a = lab.add(root, "a");
    let album = query::track(&lab.db, a).unwrap().unwrap().album_id.unwrap();
    for _ in 0..2 {
        set_favorite(&mut lab.db, FavoriteTarget::Track { id: a }, true, 5).unwrap();
        set_favorite(&mut lab.db, FavoriteTarget::Album { id: album }, true, 5).unwrap();
    }
    assert!(query::track(&lab.db, a).unwrap().unwrap().favorite);
    assert!(query::album(&lab.db, album).unwrap().unwrap().favorite);
    assert_eq!(favorite_tracks(&lab.db, 0, 10).unwrap().total, 1);
    assert_eq!(
        set_favorite(&mut lab.db, FavoriteTarget::Track { id: 404 }, true, 5),
        Err(UserError::NotFound)
    );
    set_favorite(&mut lab.db, FavoriteTarget::Album { id: album }, false, 5).unwrap();
    assert!(!query::album(&lab.db, album).unwrap().unwrap().favorite);
}

#[test]
fn history_orders_by_time_groups_albums_and_clears_only_its_scope() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let (a, b) = (lab.add(root, "a"), lab.add(root, "b"));
    set_favorite(&mut lab.db, FavoriteTarget::Track { id: a }, true, 5).unwrap();
    let first = record_play(&mut lab.db, a, 1_000, 30_000).unwrap().unwrap();
    record_play(&mut lab.db, b, 90_000, 30_000).unwrap();
    record_play(&mut lab.db, a, 99_000, 30_000).unwrap();
    update_play(&lab.db, first, 45_000).unwrap();
    update_play(&lab.db, first, 10).unwrap();
    let page = history(&lab.db, 0, 10).unwrap();
    let titles: Vec<_> = page.rows.iter().map(|r| r.media.title.as_str()).collect();
    assert_eq!(titles, ["a", "b", "a"]);
    assert_eq!(page.rows[2].listened_ms, 45_000.0);
    let home = home(&lab.db).unwrap();
    assert_eq!(home.recent_albums.len(), 1);
    assert_eq!(home.frequent.len(), 1);
    assert_eq!(
        clear_history(&mut lab.db, HistoryScope::LastHour, 100_000).unwrap(),
        1
    );
    assert_eq!(
        clear_history(&mut lab.db, HistoryScope::All, 100_000).unwrap(),
        2
    );
    assert_eq!(history(&lab.db, 0, 10).unwrap().total, 0);
    assert_eq!(favorite_tracks(&lab.db, 0, 10).unwrap().total, 1);
    assert_eq!(record_play(&mut lab.db, 4040, 1, 1), Ok(None));
}

#[test]
fn deleting_a_playlist_touches_neither_tracks_nor_other_lists() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let a = lab.add(root, "a");
    let (one, two) = (lab.create(&[a]), lab.create(&[a, a]));
    lab.act(PlaylistAct::Delete { id: one }).unwrap();
    assert!(playlist(&lab.db, one).unwrap().is_none());
    assert_eq!(lab.entries(two).len(), 2);
    assert!(query::track(&lab.db, a).unwrap().is_some());
}

#[test]
fn large_playlists_page_play_and_reorder_within_budget() {
    let mut lab = Lab::new();
    let root = lab.root(r"C:\Music");
    let tracks: Vec<u32> = (0..500)
        .map(|n| lab.add(root, &format!("{n:03}")))
        .collect();
    let list = lab.create(&[]);
    for _ in 0..20 {
        lab.act(PlaylistAct::AddTracks {
            id: list,
            track_ids: tracks.clone(),
        })
        .unwrap();
    }
    let page = playlist_entries(&lab.db, list, 9_900, 100).unwrap();
    assert_eq!((page.total, page.rows.len()), (10_000, 100));
    let deep = page.rows[50].entry_id;
    let plan = query::play_plan(
        &lab.db,
        &crate::contract::PlayTarget::Playlist {
            id: list,
            start_entry: Some(deep),
        },
    )
    .unwrap();
    assert_eq!(plan.items.len(), query::MAX_PLAY_ITEMS);
    assert_eq!(plan.items[plan.start].title, page.rows[50].media.title);
    let started = std::time::Instant::now();
    lab.act(move_to(list, deep, Some(lab_first(&lab, list))))
        .unwrap();
    let moved = started.elapsed();
    assert_eq!(lab.entries(list)[0].entry_id, deep);
    assert!(moved.as_secs() < 2, "{moved:?}");
}

fn lab_first(lab: &Lab, list: u32) -> u32 {
    lab.entries(list)[0].entry_id
}

#[test]
fn version_three_databases_upgrade_in_place_and_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v3.sqlite3");
    library::create_at_version(&path, 3);
    {
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "INSERT INTO library_roots (path) VALUES ('C:\\Music');
             INSERT INTO tracks (root_id, rel_path, path_key, size, mtime_ns, title, title_key, metadata_version)
                 VALUES (1, 'a', 'a', 1, 1, 'Song', 'song', 1);
             INSERT INTO playback_session VALUES (1, 'keep');",
        )
        .unwrap();
    }
    for _ in 0..2 {
        let mut db = library::open(&path).unwrap();
        let payload: String = db
            .query_row("SELECT payload FROM playback_session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(payload, "keep");
        act(
            &mut db,
            &PlaylistAct::Create {
                name: "P".into(),
                description: String::new(),
                track_ids: vec![1],
            },
            1,
        )
        .unwrap();
    }
    let db = library::open(&path).unwrap();
    assert_eq!(playlists(&db).unwrap().len(), 2);
    assert_eq!(query::track(&db, 1).unwrap().unwrap().title, "Song");
}
