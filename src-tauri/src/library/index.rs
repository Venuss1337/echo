//! The only writer of library rows.
//!
//! Everything here runs inside a caller-owned transaction so a batch of files,
//! their artists and albums, and the full-text rows commit together or not at
//! all. No function deletes a track because a file was not seen: absence only
//! ever sets `state = 1`, and only after a complete scan (see [`finalize`]).
//! Tracks are deleted solely when the user removes their library root.

use super::{
    meta::{METADATA_VERSION, TrackMeta},
    text::{clean, key},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub size: u64,
    pub mtime_ns: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct ArtworkRef {
    pub hash: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

pub(crate) struct Upsert<'a> {
    pub root_id: i64,
    /// Path below the root, original case, `\` separated.
    pub rel_path: &'a str,
    pub stamp: Stamp,
    pub meta: &'a TrackMeta,
    pub artwork: Option<&'a ArtworkRef>,
    /// Lowercase absolute parent directory. Scopes albums that carry no album
    /// artist tag so a compilation folder stays one album.
    pub folder_key: &'a str,
    pub scan_id: i64,
    /// Existing row to keep when the file was renamed or moved.
    pub moved_from: Option<i64>,
    pub now: i64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RootError {
    Duplicate,
    Overlapping,
    Storage,
}

pub(crate) fn path_key(rel_path: &str) -> String {
    rel_path.to_lowercase()
}

fn ensure_artist(tx: &Transaction, name: &str) -> rusqlite::Result<i64> {
    let artist_key = key(name);
    // The no-op update makes RETURNING yield the existing id on conflict.
    let id: i64 = tx.query_row(
        "INSERT INTO artists (name, key) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET key = key RETURNING id",
        params![name, artist_key],
        |row| row.get(0),
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO search_artists (rowid, name) VALUES (?1, ?2)",
        params![id, name],
    )?;
    Ok(id)
}

fn ensure_artwork(tx: &Transaction, art: &ArtworkRef, now: i64) -> rusqlite::Result<i64> {
    tx.query_row(
        "INSERT INTO artwork (hash, width, height, bytes, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(hash) DO UPDATE SET hash = hash RETURNING id",
        params![art.hash, art.width, art.height, art.bytes as i64, now],
        |row| row.get(0),
    )
}

/// Groups tracks into albums by album artist plus title when an album artist
/// tag exists, and by containing folder plus title otherwise. Matching on the
/// title alone would merge unrelated releases that share a common name.
fn ensure_album(
    tx: &Transaction,
    title: &str,
    album_artist: Option<&str>,
    first_artist: Option<(i64, &str)>,
    folder_key: &str,
    year: Option<u32>,
    artwork_id: Option<i64>,
) -> rusqlite::Result<i64> {
    let title_key = key(title);
    let (album_key, artist_id) = match album_artist {
        Some(name) => (
            format!("a\u{1f}{}\u{1f}{title_key}", key(name)),
            Some(ensure_artist(tx, name)?),
        ),
        None => (
            format!("d\u{1f}{folder_key}\u{1f}{title_key}"),
            first_artist.map(|(id, _)| id),
        ),
    };
    let existing: Option<(i64, Option<i64>)> = tx
        .query_row(
            "SELECT id, artist_id FROM albums WHERE key = ?1",
            [&album_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let id = match existing {
        Some((id, current_artist)) => {
            // Two different primary artists in one untagged folder: this is a
            // compilation, so stop attributing it to whoever was seen first.
            if album_artist.is_none() && current_artist != artist_id {
                let various = ensure_artist(tx, "Various Artists")?;
                tx.execute(
                    "UPDATE albums SET artist_id = ?1 WHERE id = ?2",
                    params![various, id],
                )?;
                index_album(tx, id, title, Some("Various Artists"))?;
            }
            tx.execute(
                "UPDATE albums SET year = COALESCE(year, ?1),
                                   artwork_id = COALESCE(artwork_id, ?2) WHERE id = ?3",
                params![year, artwork_id, id],
            )?;
            id
        }
        None => {
            let id: i64 = tx.query_row(
                "INSERT INTO albums (title, title_key, artist_id, key, year, artwork_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING id",
                params![title, title_key, artist_id, album_key, year, artwork_id],
                |row| row.get(0),
            )?;
            let display = album_artist.or(first_artist.map(|(_, name)| name));
            index_album(tx, id, title, display)?;
            id
        }
    };
    Ok(id)
}

fn index_album(
    tx: &Transaction,
    id: i64,
    title: &str,
    artist: Option<&str>,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO search_albums (rowid, title, artist) VALUES (?1, ?2, ?3)",
        params![id, title, artist.unwrap_or_default()],
    )
    .map(drop)
}

/// Inserts or updates one file's row, its artist links and its search rows.
/// Returns the stable track id.
pub(crate) fn upsert_track(tx: &Transaction, input: &Upsert) -> rusqlite::Result<i64> {
    let meta = input.meta;
    let key_of_path = path_key(input.rel_path);
    let artwork_id = input
        .artwork
        .map(|art| ensure_artwork(tx, art, input.now))
        .transpose()?;

    let mut artist_ids = Vec::with_capacity(meta.artists.len());
    for name in &meta.artists {
        artist_ids.push((ensure_artist(tx, name)?, name.as_str()));
    }
    let first_artist = artist_ids.first().copied();
    let artist_text: String = {
        let joined = meta.artists.join(", ");
        clean(&joined)
    };
    let album_id = meta
        .album
        .as_deref()
        .map(|title| {
            ensure_album(
                tx,
                title,
                meta.album_artist.as_deref(),
                first_artist,
                input.folder_key,
                meta.year,
                artwork_id,
            )
        })
        .transpose()?;
    let album_key = meta.album.as_deref().map(key).unwrap_or_default();

    // A moved file keeps its id by taking over its old row, unless the new
    // path is already occupied by a different row (then the upsert below wins).
    if let Some(old) = input.moved_from {
        tx.execute(
            "UPDATE tracks SET rel_path = ?1, path_key = ?2
             WHERE id = ?3 AND root_id = ?4
               AND NOT EXISTS (SELECT 1 FROM tracks WHERE root_id = ?4 AND path_key = ?2 AND id <> ?3)",
            params![input.rel_path, key_of_path, old, input.root_id],
        )?;
    }

    let id: i64 = tx.query_row(
        "INSERT INTO tracks (root_id, rel_path, path_key, size, mtime_ns, state, seen_scan,
             title, title_key, artist_id, artist_text, artist_key, album_id, album_key,
             track_no, disc_no, year, genre, duration_ms, codec, artwork_id, metadata_version)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
         ON CONFLICT(root_id, path_key) DO UPDATE SET
             rel_path = excluded.rel_path, size = excluded.size, mtime_ns = excluded.mtime_ns,
             state = 0, seen_scan = excluded.seen_scan, title = excluded.title,
             title_key = excluded.title_key, artist_id = excluded.artist_id,
             artist_text = excluded.artist_text, artist_key = excluded.artist_key,
             album_id = excluded.album_id, album_key = excluded.album_key,
             track_no = excluded.track_no, disc_no = excluded.disc_no, year = excluded.year,
             genre = excluded.genre, duration_ms = excluded.duration_ms, codec = excluded.codec,
             artwork_id = excluded.artwork_id, metadata_version = excluded.metadata_version
         RETURNING id",
        params![
            input.root_id,
            input.rel_path,
            key_of_path,
            input.stamp.size as i64,
            input.stamp.mtime_ns,
            input.scan_id,
            meta.title,
            key(&meta.title),
            first_artist.map(|(id, _)| id),
            artist_text,
            key(&artist_text),
            album_id,
            album_key,
            meta.track_no,
            meta.disc_no,
            meta.year,
            meta.genre,
            meta.duration_ms as i64,
            meta.codec,
            artwork_id,
            METADATA_VERSION,
        ],
        |row| row.get(0),
    )?;

    tx.execute("DELETE FROM track_artists WHERE track_id = ?1", [id])?;
    for (position, (artist_id, _)) in artist_ids.iter().enumerate() {
        tx.execute(
            "INSERT OR IGNORE INTO track_artists (track_id, artist_id, position) VALUES (?1, ?2, ?3)",
            params![id, artist_id, position as i64],
        )?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO search_tracks (rowid, title, artist, album) VALUES (?1, ?2, ?3, ?4)",
        params![id, meta.title, artist_text, meta.album.as_deref().unwrap_or_default()],
    )?;
    super::user::reconnect(tx, id)?;
    Ok(id)
}

/// Stamps unchanged files as seen in this scan and clears any missing flag.
pub(crate) fn mark_seen(tx: &Transaction, ids: &[i64], scan_id: i64) -> rusqlite::Result<()> {
    let mut statement =
        tx.prepare_cached("UPDATE tracks SET seen_scan = ?1, state = 0 WHERE id = ?2")?;
    for id in ids {
        statement.execute(params![scan_id, id])?;
    }
    Ok(())
}

/// Result of closing a scan that walked the whole root.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Finalized {
    /// `missing` tracks were flagged.
    Marked { missing: usize },
    /// The root had indexed tracks and none were seen. That pattern is how an
    /// emptied or remounted drive looks, so nothing was changed.
    SuspiciouslyEmpty,
}

/// Flags tracks missing after a complete walk of an online root. Tracks below
/// directories that could not be listed are never flagged, and an empty walk
/// of a previously populated root flags nothing.
pub(crate) fn finalize(
    tx: &Transaction,
    root_id: i64,
    scan_id: i64,
    unreadable_dirs: &[String],
) -> rusqlite::Result<Finalized> {
    let seen: i64 = tx.query_row(
        "SELECT COUNT(*) FROM tracks WHERE root_id = ?1 AND seen_scan = ?2",
        params![root_id, scan_id],
        |row| row.get(0),
    )?;
    let known: i64 = tx.query_row(
        "SELECT COUNT(*) FROM tracks WHERE root_id = ?1 AND state = 0",
        [root_id],
        |row| row.get(0),
    )?;
    if seen == 0 && known > 0 {
        return Ok(Finalized::SuspiciouslyEmpty);
    }
    let candidates: Vec<(i64, String)> = {
        let mut statement = tx.prepare(
            "SELECT id, path_key FROM tracks WHERE root_id = ?1 AND state = 0 AND seen_scan <> ?2",
        )?;
        statement
            .query_map(params![root_id, scan_id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?
    };
    let mut missing = 0;
    let mut flag = tx.prepare("UPDATE tracks SET state = 1 WHERE id = ?1")?;
    for (id, path) in candidates {
        let under_unreadable = unreadable_dirs
            .iter()
            .any(|dir| path.starts_with(dir.as_str()));
        if !under_unreadable {
            flag.execute([id])?;
            missing += 1;
        }
    }
    Ok(Finalized::Marked { missing })
}

/// Removes artists, albums and artwork nothing references any more and drops
/// their search rows. Returns artwork hashes whose cache files can be deleted
/// after the transaction commits.
pub(crate) fn collect_garbage(tx: &Transaction) -> rusqlite::Result<Vec<String>> {
    let albums: Vec<i64> = {
        let mut statement = tx.prepare(
            "DELETE FROM albums WHERE id NOT IN (SELECT album_id FROM tracks WHERE album_id IS NOT NULL)
             RETURNING id",
        )?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?
    };
    for id in albums {
        tx.execute("DELETE FROM search_albums WHERE rowid = ?1", [id])?;
    }
    let artists: Vec<i64> = {
        let mut statement = tx.prepare(
            "DELETE FROM artists WHERE id NOT IN (SELECT artist_id FROM track_artists)
               AND id NOT IN (SELECT artist_id FROM tracks WHERE artist_id IS NOT NULL)
               AND id NOT IN (SELECT artist_id FROM albums WHERE artist_id IS NOT NULL)
             RETURNING id",
        )?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?
    };
    for id in artists {
        tx.execute("DELETE FROM search_artists WHERE rowid = ?1", [id])?;
    }
    let mut statement = tx.prepare(
        "DELETE FROM artwork WHERE id NOT IN (SELECT artwork_id FROM tracks WHERE artwork_id IS NOT NULL)
           AND id NOT IN (SELECT artwork_id FROM albums WHERE artwork_id IS NOT NULL)
           AND id NOT IN (SELECT artwork_id FROM media_sources WHERE artwork_id IS NOT NULL)
           AND id NOT IN (SELECT artwork_id FROM favorite_albums WHERE artwork_id IS NOT NULL)
         RETURNING hash",
    )?;
    let hashes = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(hashes)
}

fn comparable(path: &str) -> String {
    let mut value = path.replace('/', "\\").to_lowercase();
    while value.ends_with('\\') && value.len() > 3 {
        value.pop();
    }
    value
}

fn contains_dir(parent: &str, child: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| parent.ends_with('\\') || rest.starts_with('\\'))
}

pub(crate) fn add_root(db: &Connection, path: &str, now: i64) -> Result<i64, RootError> {
    let new = comparable(path);
    let mut statement = db
        .prepare("SELECT path FROM library_roots")
        .map_err(|_| RootError::Storage)?;
    let existing: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .and_then(|rows| rows.collect())
        .map_err(|_| RootError::Storage)?;
    for other in existing {
        let other = comparable(&other);
        if other == new {
            return Err(RootError::Duplicate);
        }
        // Nested roots would index the same file twice under two identities.
        if contains_dir(&other, &new) || contains_dir(&new, &other) {
            return Err(RootError::Overlapping);
        }
    }
    db.query_row(
        "INSERT INTO library_roots (path, added_at) VALUES (?1, ?2) RETURNING id",
        params![path, now],
        |row| row.get(0),
    )
    .map_err(|_| RootError::Storage)
}

/// Forgets a root and its index rows. The files on disk are never touched.
pub(crate) fn remove_root(db: &mut Connection, root_id: i64) -> rusqlite::Result<Vec<String>> {
    let tx = db.transaction()?;
    // Playlists, favorites and history outlive the index: refresh their
    // snapshots while the rows still exist, then let them become tombstones.
    super::user::snapshot_root(&tx, root_id)?;
    tx.execute(
        "DELETE FROM search_tracks WHERE rowid IN (SELECT id FROM tracks WHERE root_id = ?1)",
        [root_id],
    )?;
    tx.execute("DELETE FROM library_roots WHERE id = ?1", [root_id])?;
    super::user::prune_sources(&tx)?;
    let hashes = collect_garbage(&tx)?;
    tx.commit()?;
    Ok(hashes)
}

pub(crate) fn begin_scan(db: &Connection, root_id: i64, now: i64) -> rusqlite::Result<i64> {
    db.query_row(
        "INSERT INTO scans (root_id, started_at, status) VALUES (?1, ?2, 'running') RETURNING id",
        params![root_id, now],
        |row| row.get(0),
    )
}

pub(crate) struct ScanTotals {
    pub discovered: u32,
    pub indexed: u32,
    pub unchanged: u32,
    pub skipped: u32,
    pub failed: u32,
    pub missing: u32,
}

pub(crate) fn finish_scan(
    db: &Connection,
    scan_id: i64,
    status: &str,
    totals: &ScanTotals,
    now: i64,
) -> rusqlite::Result<()> {
    db.execute(
        "UPDATE scans SET status = ?2, finished_at = ?3, discovered = ?4, indexed = ?5,
                          unchanged = ?6, skipped = ?7, failed = ?8, missing = ?9 WHERE id = ?1",
        params![
            scan_id,
            status,
            now,
            totals.discovered,
            totals.indexed,
            totals.unchanged,
            totals.skipped,
            totals.failed,
            totals.missing
        ],
    )?;
    // History is for status display only; keep the last few per root.
    db.execute(
        "DELETE FROM scans WHERE id IN (
             SELECT id FROM scans WHERE root_id = (SELECT root_id FROM scans WHERE id = ?1)
             ORDER BY id DESC LIMIT -1 OFFSET 8)",
        [scan_id],
    )?;
    Ok(())
}

/// A crash or kill leaves 'running' rows behind. Recording them as interrupted
/// is the whole repair: no track row depends on a scan having finished.
pub(crate) fn repair_interrupted(db: &Connection, now: i64) -> rusqlite::Result<usize> {
    db.execute(
        "UPDATE scans SET status = 'interrupted', finished_at = ?1 WHERE status = 'running'",
        [now],
    )
}

pub(crate) fn set_root_online(
    db: &Connection,
    root_id: i64,
    online: bool,
) -> rusqlite::Result<bool> {
    Ok(db.execute(
        "UPDATE library_roots SET online = ?2 WHERE id = ?1 AND online <> ?2",
        params![root_id, online],
    )? > 0)
}

/// Marks a library track as missing after playback found its file gone.
/// Returns whether the row changed.
pub(crate) fn mark_track_missing(db: &Connection, track_id: i64) -> rusqlite::Result<bool> {
    Ok(db.execute(
        "UPDATE tracks SET state = 1 WHERE id = ?1 AND state = 0",
        [track_id],
    )? > 0)
}

/// Rebuilds all three full-text indexes from the relational tables.
pub(crate) fn rebuild_search(db: &mut Connection) -> rusqlite::Result<()> {
    let tx = db.transaction()?;
    tx.execute_batch(
        "DELETE FROM search_tracks;
         INSERT INTO search_tracks (rowid, title, artist, album)
             SELECT t.id, t.title, t.artist_text, COALESCE(a.title, '')
             FROM tracks t LEFT JOIN albums a ON a.id = t.album_id;
         DELETE FROM search_albums;
         INSERT INTO search_albums (rowid, title, artist)
             SELECT a.id, a.title, COALESCE(r.name, '')
             FROM albums a LEFT JOIN artists r ON r.id = a.artist_id;
         DELETE FROM search_artists;
         INSERT INTO search_artists (rowid, name) SELECT id, name FROM artists;",
    )?;
    tx.commit()
}

/// Whether each full-text index matches its source table and passes FTS5's own
/// structural integrity check.
pub(crate) fn search_is_consistent(db: &Connection) -> bool {
    let counts_match = |index: &str, table: &str| -> bool {
        let rows = |sql: String| db.query_row(&sql, [], |row| row.get::<_, i64>(0)).ok();
        let indexed = rows(format!("SELECT COUNT(*) FROM {index}"));
        indexed.is_some() && indexed == rows(format!("SELECT COUNT(*) FROM {table}"))
    };
    let integrity = |index: &str| {
        db.execute(
            &format!("INSERT INTO {index}({index}, rank) VALUES ('integrity-check', 0)"),
            [],
        )
        .is_ok()
    };
    // The integrity-check command writes nothing but needs a writable handle,
    // so only the count comparison applies to read-only connections.
    counts_match("search_tracks", "tracks")
        && counts_match("search_albums", "albums")
        && counts_match("search_artists", "artists")
        && ["search_tracks", "search_albums", "search_artists"]
            .into_iter()
            .all(integrity)
}

/// Ids present in `tracks` that no root references; used by integrity tests.
#[cfg(test)]
pub(crate) fn orphan_track_count(db: &Connection) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM tracks WHERE root_id NOT IN (SELECT id FROM library_roots)",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::library::meta::TrackMeta;

    pub(crate) fn meta(title: &str, artists: &[&str], album: Option<&str>) -> TrackMeta {
        TrackMeta {
            title: title.into(),
            artists: artists.iter().map(|name| (*name).into()).collect(),
            album: album.map(Into::into),
            codec: "FLAC".into(),
            duration_ms: 180_000,
            ..TrackMeta::default()
        }
    }

    pub(crate) fn add(
        db: &mut Connection,
        root: i64,
        rel: &str,
        meta: &TrackMeta,
        scan: i64,
    ) -> i64 {
        let tx = db.transaction().unwrap();
        let folder = rel
            .rsplit_once('\\')
            .map_or("", |(dir, _)| dir)
            .to_lowercase();
        let id = upsert_track(
            &tx,
            &Upsert {
                root_id: root,
                rel_path: rel,
                stamp: Stamp {
                    size: 1000 + rel.len() as u64,
                    mtime_ns: 5,
                },
                meta,
                artwork: None,
                folder_key: &folder,
                scan_id: scan,
                moved_from: None,
                now: 1,
            },
        )
        .unwrap();
        tx.commit().unwrap();
        id
    }
}

#[cfg(test)]
mod tests {
    use super::{fixtures::*, *};
    use crate::library;

    fn count(db: &Connection, sql: &str) -> i64 {
        db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn database() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = library::open(&dir.path().join("lib.sqlite3")).unwrap();
        (dir, db)
    }

    #[test]
    fn identity_groups_artists_by_case_and_albums_by_artist_and_title() {
        let (_dir, mut db) = database();
        let root = add_root(&db, r"C:\Music", 1).unwrap();
        let mut a = meta("One", &["Björk"], Some("Homogenic"));
        a.album_artist = Some("Björk".into());
        let mut b = meta("Two", &["BJÖRK"], Some("homogenic"));
        b.album_artist = Some("BJÖRK".into());
        let mut other = meta("Three", &["Someone"], Some("Homogenic"));
        other.album_artist = Some("Someone".into());
        let first = add(&mut db, root, r"a\1.flac", &a, 1);
        add(&mut db, root, r"a\2.flac", &b, 1);
        add(&mut db, root, r"b\3.flac", &other, 1);
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) FROM artists WHERE name LIKE '%j_rk' OR name LIKE '%J_RK'"
            ),
            1
        );
        assert_eq!(count(&db, "SELECT COUNT(*) FROM albums"), 2);
        assert!(search_is_consistent(&db));
        // Re-inserting the same path keeps the id and does not duplicate rows.
        assert_eq!(add(&mut db, root, r"A\1.flac", &a, 2), first);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM tracks"), 3);
    }

    #[test]
    fn untagged_folders_are_one_album_and_mixed_artists_become_various() {
        let (_dir, mut db) = database();
        let root = add_root(&db, r"C:\Music", 1).unwrap();
        add(
            &mut db,
            root,
            r"mix\1.mp3",
            &meta("A", &["First"], Some("Mix")),
            1,
        );
        add(
            &mut db,
            root,
            r"mix\2.mp3",
            &meta("B", &["Second"], Some("Mix")),
            1,
        );
        add(
            &mut db,
            root,
            r"elsewhere\3.mp3",
            &meta("C", &["First"], Some("Mix")),
            1,
        );
        let (albums, artist): (i64, String) = db
            .query_row(
                "SELECT COUNT(*), (SELECT r.name FROM albums a JOIN artists r ON r.id = a.artist_id
                                   WHERE a.key LIKE 'd%mix%' ORDER BY a.id LIMIT 1) FROM albums",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(albums, 2, "same title in another folder stays separate");
        assert_eq!(artist, "Various Artists");
        assert!(search_is_consistent(&db));
    }

    #[test]
    fn roots_reject_duplicates_and_nesting_but_allow_siblings() {
        let (_dir, db) = database();
        add_root(&db, r"D:\Music", 1).unwrap();
        assert_eq!(add_root(&db, r"d:\music\", 1), Err(RootError::Duplicate));
        assert_eq!(
            add_root(&db, r"D:\Music\Jazz", 1),
            Err(RootError::Overlapping)
        );
        assert_eq!(add_root(&db, r"D:\", 1), Err(RootError::Overlapping));
        add_root(&db, r"D:\Music2", 1).unwrap();
        add_root(&db, r"E:\Music", 1).unwrap();
    }

    #[test]
    fn finalize_flags_only_unseen_tracks_outside_unreadable_directories() {
        let (_dir, mut db) = database();
        let root = add_root(&db, r"C:\Music", 1).unwrap();
        let ids: Vec<i64> = ["keep.flac", "gone.flac", r"locked\inside.flac"]
            .iter()
            .map(|rel| add(&mut db, root, rel, &meta(rel, &[], None), 1))
            .collect();
        let tx = db.transaction().unwrap();
        mark_seen(&tx, &ids[..1], 2).unwrap();
        let result = finalize(&tx, root, 2, &["locked\\".to_owned()]).unwrap();
        tx.commit().unwrap();
        assert_eq!(result, Finalized::Marked { missing: 1 });
        let states: Vec<i64> = ids
            .iter()
            .map(|id| {
                db.query_row("SELECT state FROM tracks WHERE id=?1", [id], |r| r.get(0))
                    .unwrap()
            })
            .collect();
        assert_eq!(states, [0, 1, 0]);
        // Seeing a file again clears the flag without changing its identity.
        let tx = db.transaction().unwrap();
        mark_seen(&tx, &ids[1..2], 3).unwrap();
        tx.commit().unwrap();
        let back: i64 = db
            .query_row("SELECT state FROM tracks WHERE id=?1", [ids[1]], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(back, 0);
    }

    #[test]
    fn an_empty_walk_of_a_populated_root_changes_nothing() {
        let (_dir, mut db) = database();
        let root = add_root(&db, r"C:\Music", 1).unwrap();
        add(&mut db, root, "a.flac", &meta("a", &[], None), 1);
        let tx = db.transaction().unwrap();
        assert_eq!(
            finalize(&tx, root, 2, &[]).unwrap(),
            Finalized::SuspiciouslyEmpty
        );
        tx.commit().unwrap();
        let state: i64 = db
            .query_row("SELECT state FROM tracks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, 0);
    }

    #[test]
    fn removing_a_root_cascades_index_rows_and_garbage() {
        let (_dir, mut db) = database();
        let keep = add_root(&db, r"C:\Keep", 1).unwrap();
        let drop_root = add_root(&db, r"C:\Drop", 1).unwrap();
        let mut shared = meta("S", &["Shared"], Some("Both"));
        shared.album_artist = Some("Shared".into());
        add(&mut db, keep, "s.flac", &shared, 1);
        add(&mut db, drop_root, "s.flac", &shared, 1);
        add(
            &mut db,
            drop_root,
            "only.flac",
            &meta("O", &["Lonely"], Some("Solo")),
            1,
        );
        remove_root(&mut db, drop_root).unwrap();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM tracks"), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM artists"), 1);
        assert_eq!(count(&db, "SELECT COUNT(*) FROM albums"), 1);
        assert_eq!(orphan_track_count(&db), 0);
        assert!(search_is_consistent(&db));
    }

    #[test]
    fn search_rebuild_recovers_from_a_damaged_index() {
        let (_dir, mut db) = database();
        let root = add_root(&db, r"C:\Music", 1).unwrap();
        for n in 0..50 {
            add(
                &mut db,
                root,
                &format!("t{n}.flac"),
                &meta(&format!("Song {n}"), &["Band"], Some("LP")),
                1,
            );
        }
        db.execute("DELETE FROM search_tracks WHERE rowid % 2 = 0", [])
            .unwrap();
        db.execute("DELETE FROM search_artists", []).unwrap();
        assert!(!search_is_consistent(&db));
        rebuild_search(&mut db).unwrap();
        assert!(search_is_consistent(&db));
        let hits: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM search_tracks WHERE search_tracks MATCH '\"song\"*'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hits, 50);
    }

    #[test]
    fn interrupted_scans_are_recorded_without_touching_tracks() {
        let (_dir, mut db) = database();
        let root = add_root(&db, r"C:\Music", 1).unwrap();
        add(&mut db, root, "a.flac", &meta("a", &[], None), 1);
        let scan = begin_scan(&db, root, 10).unwrap();
        assert_eq!(repair_interrupted(&db, 20).unwrap(), 1);
        let status: String = db
            .query_row("SELECT status FROM scans WHERE id=?1", [scan], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "interrupted");
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM tracks WHERE state = 0", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}
