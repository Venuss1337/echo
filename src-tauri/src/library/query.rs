//! Read-only, paginated library queries.
//!
//! Every function takes a connection and returns at most one bounded page, so
//! the renderer never receives the whole library. SQL text is assembled only
//! from fixed fragments chosen by enums; all values, including search text, are
//! bound parameters.

use super::text::fts_query;
use crate::contract::{
    AlbumPage, AlbumQuery, AlbumRow, AlbumSort, ArtistPage, ArtistQuery, ArtistRole, ArtistRow,
    LibraryRootView, PlayTarget, ScanReport, ScanStatus, SearchResults, TrackPage, TrackQuery,
    TrackRow, TrackSort,
};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Value};
use std::path::PathBuf;

/// Longest playlist a play request considers before windowing.
const MAX_PLAY_SCAN: i64 = 20_000;
pub(crate) const MAX_PAGE: u32 = 200;
pub(crate) const MAX_SEARCH: u32 = 40;
/// Matches the playback queue capacity.
pub(crate) const MAX_PLAY_ITEMS: usize = 1024;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum QueryError {
    Invalid,
    Storage,
}

impl From<rusqlite::Error> for QueryError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

pub(crate) fn id32(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn opt32(value: Option<i64>) -> Option<u32> {
    value.map(id32)
}

fn page_bounds(offset: u32, limit: u32) -> Result<(i64, i64), QueryError> {
    if limit == 0 || limit > MAX_PAGE {
        return Err(QueryError::Invalid);
    }
    Ok((i64::from(offset), i64::from(limit)))
}

const TRACK_COLUMNS: &str =
    "t.id, t.title, t.artist_text, COALESCE(a.title, ''), t.album_id, t.artist_id,
    t.track_no, t.disc_no, t.year, t.duration_ms, (t.state = 0 AND r.online = 1),
    COALESCE(t.artwork_id, a.artwork_id), t.codec,
    EXISTS (SELECT 1 FROM media_sources ms JOIN favorite_tracks ft ON ft.source_id = ms.id
            WHERE ms.track_id = t.id)";
/// Reading order inside an album. Missing disc numbers mean disc one, and
/// unnumbered tracks follow numbered ones.
pub(crate) const ALBUM_ORDER: &str =
    "COALESCE(t.disc_no, 1), t.track_no IS NULL, t.track_no, t.title_key, t.id";
const TRACK_FROM: &str = "tracks t JOIN library_roots r ON r.id = t.root_id
    LEFT JOIN albums a ON a.id = t.album_id";

fn track_row(row: &Row) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: id32(row.get(0)?),
        title: row.get(1)?,
        artist: row.get(2)?,
        album: row.get(3)?,
        album_id: opt32(row.get(4)?),
        artist_id: opt32(row.get(5)?),
        track_no: opt32(row.get(6)?),
        disc_no: opt32(row.get(7)?),
        year: opt32(row.get(8)?),
        duration_ms: id32(row.get(9)?),
        available: row.get(10)?,
        artwork_id: opt32(row.get(11)?),
        codec: row.get(12)?,
        favorite: row.get(13)?,
    })
}

pub(crate) fn tracks_page(db: &Connection, query: &TrackQuery) -> Result<TrackPage, QueryError> {
    let (offset, limit) = page_bounds(query.offset, query.limit)?;
    let mut filters = Vec::new();
    let mut values: Vec<Value> = Vec::new();
    if let Some(album) = query.album_id {
        filters.push("t.album_id = ?");
        values.push(i64::from(album).into());
    }
    if let Some(artist) = query.artist_id {
        filters.push(
            "EXISTS (SELECT 1 FROM track_artists ta WHERE ta.track_id = t.id AND ta.artist_id = ?)",
        );
        values.push(i64::from(artist).into());
    }
    let condition = if filters.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", filters.join(" AND "))
    };
    let order = match query.sort {
        TrackSort::Title => "t.title_key, t.id",
        TrackSort::Artist => "t.artist_key, t.album_key, t.disc_no, t.track_no, t.id",
        TrackSort::Album if query.album_id.is_some() => ALBUM_ORDER,
        TrackSort::Album => "t.album_key, t.album_id, t.disc_no, t.track_no, t.id",
        TrackSort::Recent => "t.id DESC",
    };
    let total: i64 = db.query_row(
        &format!("SELECT COUNT(*) FROM tracks t {condition}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    values.push(limit.into());
    values.push(offset.into());
    let mut statement = db.prepare_cached(&format!(
        "SELECT {TRACK_COLUMNS} FROM {TRACK_FROM} {condition} ORDER BY {order} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), track_row)?
        .collect::<Result<_, _>>()?;
    Ok(TrackPage {
        total: id32(total),
        offset: query.offset,
        rows,
    })
}

const ALBUM_COLUMNS: &str =
    "al.id, al.title, COALESCE(ar.name, ''), al.artist_id, al.year, al.artwork_id,
    (SELECT COUNT(*) FROM tracks t WHERE t.album_id = al.id),
    (SELECT COUNT(*) FROM tracks t JOIN library_roots r ON r.id = t.root_id
        WHERE t.album_id = al.id AND t.state = 0 AND r.online = 1),
    (SELECT COALESCE(SUM(t.duration_ms), 0) FROM tracks t WHERE t.album_id = al.id),
    EXISTS (SELECT 1 FROM favorite_albums fa WHERE fa.album_key = al.key)";
const ALBUM_FROM: &str = "albums al LEFT JOIN artists ar ON ar.id = al.artist_id";

fn album_row(row: &Row) -> rusqlite::Result<AlbumRow> {
    Ok(AlbumRow {
        id: id32(row.get(0)?),
        title: row.get(1)?,
        artist: row.get(2)?,
        artist_id: opt32(row.get(3)?),
        year: opt32(row.get(4)?),
        artwork_id: opt32(row.get(5)?),
        tracks: id32(row.get(6)?),
        available_tracks: id32(row.get(7)?),
        duration_ms: row.get::<_, i64>(8)? as f64,
        favorite: row.get(9)?,
    })
}

pub(crate) fn albums_page(db: &Connection, query: &AlbumQuery) -> Result<AlbumPage, QueryError> {
    let (offset, limit) = page_bounds(query.offset, query.limit)?;
    let mut values: Vec<Value> = Vec::new();
    const APPEARS: &str = "EXISTS (SELECT 1 FROM tracks t
        JOIN track_artists ta ON ta.track_id = t.id WHERE t.album_id = al.id AND ta.artist_id = ?)";
    let condition = match query.artist_id.map(i64::from) {
        None => String::new(),
        Some(artist) => {
            values.push(artist.into());
            match query.role {
                Some(ArtistRole::Main) => "WHERE al.artist_id = ?".to_owned(),
                Some(ArtistRole::Appears) => {
                    values.push(artist.into());
                    format!("WHERE al.artist_id IS NOT ? AND {APPEARS}")
                }
                // Every album they lead or contribute to.
                None => {
                    values.push(artist.into());
                    format!("WHERE al.artist_id = ? OR {APPEARS}")
                }
            }
        }
    };
    let order = match query.sort {
        AlbumSort::Title => "al.title_key, al.id",
        AlbumSort::Artist => "COALESCE(ar.key, ''), al.year, al.title_key, al.id",
        AlbumSort::Year => "al.year IS NULL, al.year DESC, al.title_key, al.id",
        AlbumSort::Recent => "al.id DESC",
    };
    let total: i64 = db.query_row(
        &format!("SELECT COUNT(*) FROM {ALBUM_FROM} {condition}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    values.push(limit.into());
    values.push(offset.into());
    let mut statement = db.prepare_cached(&format!(
        "SELECT {ALBUM_COLUMNS} FROM {ALBUM_FROM} {condition} ORDER BY {order} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), album_row)?
        .collect::<Result<_, _>>()?;
    Ok(AlbumPage {
        total: id32(total),
        offset: query.offset,
        rows,
    })
}

const ARTIST_COLUMNS: &str = "ar.id, ar.name,
    (SELECT COUNT(DISTINCT t.album_id) FROM track_artists ta JOIN tracks t ON t.id = ta.track_id
        WHERE ta.artist_id = ar.id AND t.album_id IS NOT NULL),
    (SELECT COUNT(*) FROM track_artists ta WHERE ta.artist_id = ar.id),
    (SELECT t.artwork_id FROM track_artists ta JOIN tracks t ON t.id = ta.track_id
        WHERE ta.artist_id = ar.id AND t.artwork_id IS NOT NULL ORDER BY t.id LIMIT 1)";

fn artist_row(row: &Row) -> rusqlite::Result<ArtistRow> {
    Ok(ArtistRow {
        id: id32(row.get(0)?),
        name: row.get(1)?,
        albums: id32(row.get(2)?),
        tracks: id32(row.get(3)?),
        artwork_id: opt32(row.get(4)?),
    })
}

pub(crate) fn artists_page(db: &Connection, query: &ArtistQuery) -> Result<ArtistPage, QueryError> {
    let (offset, limit) = page_bounds(query.offset, query.limit)?;
    let total: i64 = db.query_row("SELECT COUNT(*) FROM artists", [], |row| row.get(0))?;
    let mut statement = db.prepare_cached(&format!(
        "SELECT {ARTIST_COLUMNS} FROM artists ar ORDER BY ar.key, ar.id LIMIT ?1 OFFSET ?2"
    ))?;
    let rows = statement
        .query_map(params![limit, offset], artist_row)?
        .collect::<Result<_, _>>()?;
    Ok(ArtistPage {
        total: id32(total),
        offset: query.offset,
        rows,
    })
}

pub(crate) fn track(db: &Connection, id: u32) -> Result<Option<TrackRow>, QueryError> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {TRACK_COLUMNS} FROM {TRACK_FROM} WHERE t.id = ?1"
    ))?;
    Ok(statement.query_row([i64::from(id)], track_row).optional()?)
}

pub(crate) fn album(db: &Connection, id: u32) -> Result<Option<AlbumRow>, QueryError> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {ALBUM_COLUMNS} FROM {ALBUM_FROM} WHERE al.id = ?1"
    ))?;
    Ok(statement.query_row([i64::from(id)], album_row).optional()?)
}

pub(crate) fn artist(db: &Connection, id: u32) -> Result<Option<ArtistRow>, QueryError> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {ARTIST_COLUMNS} FROM artists ar WHERE ar.id = ?1"
    ))?;
    Ok(statement
        .query_row([i64::from(id)], artist_row)
        .optional()?)
}

/// Cached palette JSON, if it was computed before.
pub(crate) fn artwork_palette(db: &Connection, id: u32) -> Result<Option<String>, QueryError> {
    Ok(db
        .query_row(
            "SELECT palette FROM artwork WHERE id = ?1",
            [i64::from(id)],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

/// Files that embed this artwork, for regenerating a larger variant.
pub(crate) fn artwork_files(db: &Connection, id: u32) -> Result<Vec<PathBuf>, QueryError> {
    let mut statement = db.prepare_cached(
        "SELECT r.path, t.rel_path FROM tracks t JOIN library_roots r ON r.id = t.root_id
         WHERE t.artwork_id = ?1 AND t.state = 0 AND r.online = 1 ORDER BY t.id LIMIT 4",
    )?;
    Ok(statement
        .query_map([i64::from(id)], |row| {
            Ok(PathBuf::from(row.get::<_, String>(0)?).join(row.get::<_, String>(1)?))
        })?
        .collect::<Result<_, _>>()?)
}

/// Ranked prefix search across tracks, albums and artists. Empty or
/// unsearchable text yields empty results rather than an error.
pub(crate) fn search(db: &Connection, text: &str, limit: u32) -> Result<SearchResults, QueryError> {
    if limit == 0 || limit > MAX_SEARCH {
        return Err(QueryError::Invalid);
    }
    let Some(expression) = fts_query(text) else {
        return Ok(SearchResults {
            tracks: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
        });
    };
    let limit = i64::from(limit);
    let tracks = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {TRACK_COLUMNS} FROM (
                 SELECT rowid AS id, bm25(search_tracks, 10.0, 4.0, 3.0) AS rank
                 FROM search_tracks WHERE search_tracks MATCH ?1 ORDER BY rank, rowid LIMIT ?2
             ) m JOIN tracks t ON t.id = m.id
             JOIN library_roots r ON r.id = t.root_id LEFT JOIN albums a ON a.id = t.album_id
             ORDER BY m.rank, t.id"
        ))?;
        statement
            .query_map(params![expression, limit], track_row)?
            .collect::<Result<_, _>>()?
    };
    let albums = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {ALBUM_COLUMNS} FROM (
                 SELECT rowid AS id, bm25(search_albums, 10.0, 4.0) AS rank
                 FROM search_albums WHERE search_albums MATCH ?1 ORDER BY rank, rowid LIMIT ?2
             ) m JOIN albums al ON al.id = m.id
             LEFT JOIN artists ar ON ar.id = al.artist_id ORDER BY m.rank, al.id"
        ))?;
        statement
            .query_map(params![expression, limit], album_row)?
            .collect::<Result<_, _>>()?
    };
    let artists = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {ARTIST_COLUMNS} FROM (
                 SELECT rowid AS id, bm25(search_artists) AS rank
                 FROM search_artists WHERE search_artists MATCH ?1 ORDER BY rank, rowid LIMIT ?2
             ) m JOIN artists ar ON ar.id = m.id ORDER BY m.rank, ar.id"
        ))?;
        statement
            .query_map(params![expression, limit], artist_row)?
            .collect::<Result<_, _>>()?
    };
    Ok(SearchResults {
        tracks,
        albums,
        artists,
    })
}

pub(crate) fn artwork_hash(db: &Connection, id: u32) -> Result<Option<String>, QueryError> {
    Ok(db
        .query_row(
            "SELECT hash FROM artwork WHERE id = ?1",
            [i64::from(id)],
            |row| row.get(0),
        )
        .optional()?)
}

pub(crate) struct Counts {
    pub tracks: u32,
    pub albums: u32,
    pub artists: u32,
    pub unavailable: u32,
}

pub(crate) fn counts(db: &Connection) -> Result<Counts, QueryError> {
    let one = |sql: &str| db.query_row(sql, [], |row| row.get::<_, i64>(0)).map(id32);
    Ok(Counts {
        tracks: one("SELECT COUNT(*) FROM tracks")?,
        albums: one("SELECT COUNT(*) FROM albums")?,
        artists: one("SELECT COUNT(*) FROM artists")?,
        unavailable: one(
            "SELECT COUNT(*) FROM tracks t JOIN library_roots r ON r.id = t.root_id
             WHERE t.state <> 0 OR r.online = 0",
        )?,
    })
}

fn status_of(text: &str) -> ScanStatus {
    match text {
        "complete" => ScanStatus::Complete,
        "partial" => ScanStatus::Partial,
        "cancelled" => ScanStatus::Cancelled,
        "offline" => ScanStatus::Offline,
        "interrupted" | "running" => ScanStatus::Interrupted,
        _ => ScanStatus::Failed,
    }
}

pub(crate) fn roots(db: &Connection) -> Result<Vec<LibraryRootView>, QueryError> {
    let mut statement = db.prepare_cached(
        "SELECT r.id, r.path, r.online, r.last_complete_scan_at,
             (SELECT COUNT(*) FROM tracks t WHERE t.root_id = r.id),
             (SELECT COUNT(*) FROM tracks t WHERE t.root_id = r.id AND (t.state <> 0 OR r.online = 0)),
             s.status, s.finished_at, s.discovered, s.indexed, s.unchanged, s.skipped, s.failed, s.missing
         FROM library_roots r
         LEFT JOIN scans s ON s.id = (SELECT MAX(id) FROM scans WHERE root_id = r.id AND status <> 'running')
         ORDER BY r.id",
    )?;
    let rows = statement
        .query_map([], |row| {
            let path: String = row.get(1)?;
            let status: Option<String> = row.get(6)?;
            Ok(LibraryRootView {
                id: id32(row.get(0)?),
                name: root_name(&path),
                path,
                online: row.get(2)?,
                last_scan_at: row.get::<_, Option<i64>>(3)?.map(|seconds| seconds as f64),
                tracks: id32(row.get(4)?),
                unavailable_tracks: id32(row.get(5)?),
                last_scan: status
                    .map(|status| -> rusqlite::Result<ScanReport> {
                        Ok(ScanReport {
                            status: status_of(&status),
                            finished_at: row
                                .get::<_, Option<i64>>(7)?
                                .map(|seconds| seconds as f64),
                            discovered: id32(row.get(8)?),
                            indexed: id32(row.get(9)?),
                            unchanged: id32(row.get(10)?),
                            skipped: id32(row.get(11)?),
                            failed: id32(row.get(12)?),
                            missing: id32(row.get(13)?),
                        })
                    })
                    .transpose()?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

/// The last path component, or the whole path for a drive root.
pub(crate) fn root_name(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    trimmed
        .rsplit(['\\', '/'])
        .next()
        .filter(|name| !name.is_empty() && !name.ends_with(':'))
        .map_or_else(|| path.to_owned(), str::to_owned)
}

/// A library track resolved to what the playback queue needs. The path is
/// rebuilt from the root and the relative path stored by the scanner.
pub(crate) struct PlayItem {
    pub track_id: u32,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub artwork_id: Option<u32>,
}

/// One index row of a play target, before availability filtering.
type PlayRow = (i64, String, String, String, String, Option<i64>, bool, i64);

pub(crate) struct PlayPlan {
    pub items: Vec<PlayItem>,
    /// Index in `items` where playback should begin.
    pub start: usize,
}

/// Resolves a renderer-supplied target into playable local files. Unavailable
/// tracks are skipped, not queued, so a disconnected drive cannot fill the
/// queue with entries that are certain to fail.
pub(crate) fn play_plan(db: &Connection, target: &PlayTarget) -> Result<PlayPlan, QueryError> {
    const ITEM: &str = "t.id, r.path, t.rel_path, t.title, t.artist_text,
        COALESCE(t.artwork_id, a.artwork_id), (t.state = 0 AND r.online = 1)";
    const JOINS: &str = "JOIN library_roots r ON r.id = t.root_id
        LEFT JOIN albums a ON a.id = t.album_id";
    let of_tracks = |condition: &str, order: &str| {
        format!(
            "SELECT {ITEM}, t.id FROM tracks t {JOINS} WHERE {condition} ORDER BY {order} LIMIT ?2"
        )
    };
    let mut wanted = None;
    let mut rows: Vec<PlayRow> = Vec::new();
    let mut load = |sql: &str, bound: i64| -> Result<(), QueryError> {
        let mut statement = db.prepare_cached(sql)?;
        for row in statement.query_map(params![bound, MAX_PLAY_SCAN], play_row)? {
            rows.push(row?);
        }
        Ok(())
    };
    match target {
        PlayTarget::Track { id } => load(&of_tracks("t.id = ?1", "t.id"), i64::from(*id))?,
        PlayTarget::Album { id, start_track } => {
            wanted = *start_track;
            load(&of_tracks("t.album_id = ?1", ALBUM_ORDER), i64::from(*id))?;
        }
        PlayTarget::Artist { id } => load(
            &of_tracks(
                "EXISTS (SELECT 1 FROM track_artists ta WHERE ta.track_id = t.id AND ta.artist_id = ?1)
                 OR a.artist_id = ?1",
                &format!("a.year IS NULL, a.year, t.album_key, t.album_id, {ALBUM_ORDER}"),
            ),
            i64::from(*id),
        )?,
        PlayTarget::Tracks { ids } => {
            if ids.is_empty() || ids.len() > MAX_PLAY_ITEMS {
                return Err(QueryError::Invalid);
            }
            for id in ids {
                load(&of_tracks("t.id = ?1", "t.id"), i64::from(*id))?;
            }
        }
        PlayTarget::Playlist { id, start_entry } => {
            wanted = *start_entry;
            load(
                &format!(
                    "SELECT {ITEM}, e.id FROM playlist_entries e
                     JOIN media_sources m ON m.id = e.source_id
                     JOIN tracks t ON t.id = m.track_id {JOINS}
                     WHERE e.playlist_id = ?1 ORDER BY e.position LIMIT ?2"
                ),
                i64::from(*id),
            )?;
        }
        PlayTarget::Favorites { start_source } => {
            wanted = *start_source;
            load(
                &format!(
                    "SELECT {ITEM}, m.id FROM favorite_tracks f
                     JOIN media_sources m ON m.id = f.source_id
                     JOIN tracks t ON t.id = m.track_id {JOINS}
                     WHERE f.source_id >= ?1 ORDER BY f.created_at DESC, f.source_id DESC LIMIT ?2"
                ),
                0,
            )?;
        }
    }
    let mut items = Vec::new();
    let mut start = None;
    let mut reached = false;
    for (id, root, rel, title, artist, artwork, available, key) in rows {
        // An unavailable starting row falls forward to the next playable one.
        reached |= wanted == Some(id32(key));
        if !available {
            continue;
        }
        if reached && start.is_none() {
            start = Some(items.len());
        }
        items.push(PlayItem {
            track_id: id32(id),
            path: PathBuf::from(root).join(rel),
            title,
            artist,
            artwork_id: opt32(artwork),
        });
    }
    let mut start = start.unwrap_or(0);
    if items.len() > MAX_PLAY_ITEMS {
        // The queue is bounded: keep the window that begins at the chosen row.
        let first = start.min(items.len() - MAX_PLAY_ITEMS);
        items = items.drain(first..first + MAX_PLAY_ITEMS).collect();
        start -= first;
    }
    Ok(PlayPlan { items, start })
}

fn play_row(row: &Row) -> rusqlite::Result<PlayRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{
        self,
        index::{self, fixtures::*},
    };

    fn seeded(count: usize) -> (tempfile::TempDir, Connection, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut db = library::open(&dir.path().join("q.sqlite3")).unwrap();
        let root = index::add_root(&db, r"C:\Music", 1).unwrap();
        for n in 0..count {
            let artist = format!("Artist {}", n % 7);
            let album = format!("Album {}", n % 13);
            let mut m = meta(&format!("Song {n:04}"), &[&artist], Some(&album));
            m.album_artist = Some(artist.clone());
            m.track_no = Some((n / 13) as u32 + 1);
            add(&mut db, root, &format!(r"{artist}\{album}\{n}.flac"), &m, 1);
        }
        (dir, db, root)
    }

    #[test]
    fn pages_are_bounded_stable_and_cover_the_library_exactly_once() {
        let (_dir, db, _) = seeded(250);
        let mut seen = std::collections::HashSet::new();
        for offset in [0, 100, 200] {
            let page = tracks_page(
                &db,
                &TrackQuery {
                    sort: TrackSort::Title,
                    album_id: None,
                    artist_id: None,
                    offset,
                    limit: 100,
                },
            )
            .unwrap();
            assert_eq!(page.total, 250);
            assert!(page.rows.len() <= 100);
            for row in page.rows {
                assert!(seen.insert(row.id), "row repeated across pages");
            }
        }
        assert_eq!(seen.len(), 250);
        let invalid = |limit| {
            tracks_page(
                &db,
                &TrackQuery {
                    sort: TrackSort::Title,
                    album_id: None,
                    artist_id: None,
                    offset: 0,
                    limit,
                },
            )
        };
        assert_eq!(invalid(0).err(), Some(QueryError::Invalid));
        assert_eq!(invalid(MAX_PAGE + 1).err(), Some(QueryError::Invalid));
    }

    #[test]
    fn filters_and_sorts_by_album_and_artist() {
        let (_dir, db, _) = seeded(130);
        let albums = albums_page(
            &db,
            &AlbumQuery {
                sort: AlbumSort::Title,
                artist_id: None,
                role: None,
                offset: 0,
                limit: 50,
            },
        )
        .unwrap();
        assert!(albums.total >= 13);
        let first = &albums.rows[0];
        let tracks = tracks_page(
            &db,
            &TrackQuery {
                sort: TrackSort::Album,
                album_id: Some(first.id),
                artist_id: None,
                offset: 0,
                limit: 100,
            },
        )
        .unwrap();
        assert_eq!(tracks.total, first.tracks);
        assert!(
            tracks
                .rows
                .windows(2)
                .all(|pair| pair[0].track_no <= pair[1].track_no)
        );
        let artists = artists_page(
            &db,
            &ArtistQuery {
                offset: 0,
                limit: 50,
            },
        )
        .unwrap();
        assert_eq!(artists.total, 7);
        let by_artist = albums_page(
            &db,
            &AlbumQuery {
                sort: AlbumSort::Artist,
                artist_id: Some(artists.rows[0].id),
                role: None,
                offset: 0,
                limit: 50,
            },
        )
        .unwrap();
        assert!(
            by_artist.total >= 1
                && by_artist
                    .rows
                    .iter()
                    .all(|a| a.artist_id == Some(artists.rows[0].id))
        );
    }

    #[test]
    fn search_finds_prefixes_across_kinds_and_ignores_hostile_input() {
        let (_dir, db, _) = seeded(60);
        let found = search(&db, "song 00", 20).unwrap();
        assert_eq!(found.tracks.len(), 20);
        let by_album = search(&db, "alb", 40).unwrap();
        assert!(!by_album.albums.is_empty() && !by_album.tracks.is_empty());
        assert!(!search(&db, "artist 3", 10).unwrap().artists.is_empty());
        for hostile in [
            "",
            "   ",
            "\"",
            "* OR *",
            "title:song NEAR(a b)",
            "a\0b",
            "'; DROP TABLE tracks;--",
        ] {
            assert!(search(&db, hostile, 10).is_ok(), "{hostile:?}");
        }
        assert_eq!(search(&db, "zzzzqq", 10).unwrap().tracks.len(), 0);
        assert_eq!(count_tracks(&db), 60);
        assert_eq!(search(&db, "x", 0).err(), Some(QueryError::Invalid));
    }

    fn count_tracks(db: &Connection) -> i64 {
        db.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn unavailable_tracks_and_offline_roots_are_reported_not_hidden() {
        let (_dir, db, root) = seeded(20);
        db.execute("UPDATE tracks SET state = 1 WHERE id <= 3", [])
            .unwrap();
        assert_eq!(counts(&db).unwrap().unavailable, 3);
        index::set_root_online(&db, root, false).unwrap();
        let c = counts(&db).unwrap();
        assert_eq!((c.tracks, c.unavailable), (20, 20));
        let page = tracks_page(
            &db,
            &TrackQuery {
                sort: TrackSort::Recent,
                album_id: None,
                artist_id: None,
                offset: 0,
                limit: 5,
            },
        )
        .unwrap();
        assert!(page.rows.iter().all(|row| !row.available));
        let roots = roots(&db).unwrap();
        assert!(!roots[0].online && roots[0].unavailable_tracks == 20);
    }

    #[test]
    fn play_plans_resolve_paths_skip_unavailable_and_keep_album_order() {
        let (_dir, db, _) = seeded(200);
        let album = albums_page(
            &db,
            &AlbumQuery {
                sort: AlbumSort::Title,
                artist_id: None,
                role: None,
                offset: 0,
                limit: 100,
            },
        )
        .unwrap()
        .rows
        .into_iter()
        .find(|album| album.tracks >= 2)
        .unwrap()
        .id;
        let full = play_plan(
            &db,
            &PlayTarget::Album {
                id: album,
                start_track: None,
            },
        )
        .unwrap();
        assert!(full.items.len() >= 2);
        assert!(full.items[0].path.starts_with(r"C:\Music"));
        let second = full.items[1].track_id;
        db.execute(
            "UPDATE tracks SET state = 1 WHERE id = ?1",
            [full.items[0].track_id],
        )
        .unwrap();
        let plan = play_plan(
            &db,
            &PlayTarget::Album {
                id: album,
                start_track: Some(second),
            },
        )
        .unwrap();
        assert_eq!(plan.items.len(), full.items.len() - 1);
        assert_eq!(plan.items[plan.start].track_id, second);
        let single = play_plan(&db, &PlayTarget::Track { id: second }).unwrap();
        assert_eq!((single.items.len(), single.start), (1, 0));
        let gone = play_plan(&db, &PlayTarget::Track { id: 999_999 }).unwrap();
        assert!(gone.items.is_empty());
    }

    #[test]
    fn root_names_use_the_last_component() {
        assert_eq!(root_name(r"D:\Music\Jazz"), "Jazz");
        assert_eq!(root_name(r"D:\Music\Jazz\"), "Jazz");
        assert_eq!(root_name(r"E:\"), r"E:\");
    }
}
