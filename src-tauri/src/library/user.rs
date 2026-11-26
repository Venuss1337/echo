//! Playlists, favorites and listening history.
//!
//! User data refers to `media_sources`, never to `tracks`. A source links to
//! the live index row while one exists. When a library root is removed its
//! tracks disappear, the link becomes null and the source stays behind as a
//! tombstone with the last known metadata (see ADR 0008). Every mutation is
//! one immediate transaction.

use super::{
    index::path_key,
    query::{self, MAX_PAGE, QueryError, id32},
    text::{clean, key},
};
use crate::contract::{
    AlbumQuery, AlbumRow, AlbumSort, FavoriteAlbum, FavoriteTarget, HistoryPage, HistoryRow,
    HistoryScope, HomeView, MediaPage, MediaRow, PlaylistAct, PlaylistCard, PlaylistEntry,
    PlaylistOutcome, PlaylistPage,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};

/// Distance between neighbours after an append or renumber.
const GAP: i64 = 1 << 20;
const MAX_ENTRIES: i64 = 20_000;
const MAX_BATCH: usize = 1_000;
const MAX_PLAYLISTS: i64 = 2_000;
const MAX_PLAYS: i64 = 50_000;
const DURATION_TOLERANCE_MS: i64 = 2_000;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UserError {
    Invalid,
    NotFound,
    Storage,
}

impl From<rusqlite::Error> for UserError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}

fn absolute_key(root: &str, rel: &str) -> String {
    format!("{}\\{rel}", root.trim_end_matches('\\')).to_lowercase()
}

/// Returns the durable source for an index row, creating it on first use. An
/// id whose row is gone resolves to the tombstone that replaced it.
pub(crate) fn ensure_source(tx: &Transaction, track_id: i64) -> rusqlite::Result<Option<i64>> {
    let existing = tx
        .query_row(
            "SELECT id FROM media_sources WHERE track_id = ?1 OR last_track_id = ?1
             ORDER BY track_id IS NULL LIMIT 1",
            [track_id],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Ok(existing);
    }
    let Some((root, rel)) = tx
        .query_row(
            "SELECT r.path, t.rel_path FROM tracks t
             JOIN library_roots r ON r.id = t.root_id WHERE t.id = ?1",
            [track_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    tx.query_row(
        "INSERT INTO media_sources (track_id, last_track_id, abs_key, rel_key, size, mtime_ns,
             title, artist, album, duration_ms, artwork_id)
         SELECT t.id, t.id, ?2, ?3, t.size, t.mtime_ns, t.title, t.artist_text,
             COALESCE(a.title, ''), t.duration_ms, COALESCE(t.artwork_id, a.artwork_id)
         FROM tracks t LEFT JOIN albums a ON a.id = t.album_id WHERE t.id = ?1 RETURNING id",
        params![track_id, absolute_key(&root, &rel), path_key(&rel)],
        |row| row.get(0),
    )
    .optional()
}

/// Copies current metadata and location into the sources of a root that is
/// about to be removed.
pub(crate) fn snapshot_root(tx: &Transaction, root_id: i64) -> rusqlite::Result<()> {
    let linked: Vec<(i64, i64, String, String)> = {
        let mut statement = tx.prepare(
            "SELECT m.id, t.id, r.path, t.rel_path FROM media_sources m
             JOIN tracks t ON t.id = m.track_id
             JOIN library_roots r ON r.id = t.root_id WHERE r.id = ?1",
        )?;
        statement
            .query_map([root_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<_, _>>()?
    };
    let mut update = tx.prepare(
        "UPDATE media_sources SET (size, mtime_ns, title, artist, album, duration_ms, artwork_id) =
             (SELECT t.size, t.mtime_ns, t.title, t.artist_text, COALESCE(a.title, ''),
                     t.duration_ms, COALESCE(t.artwork_id, a.artwork_id)
              FROM tracks t LEFT JOIN albums a ON a.id = t.album_id WHERE t.id = ?2),
             abs_key = ?3, rel_key = ?4 WHERE id = ?1",
    )?;
    for (source, track, root, rel) in linked {
        update.execute(params![
            source,
            track,
            absolute_key(&root, &rel),
            path_key(&rel)
        ])?;
    }
    // Favorite albums refresh their snapshot too.
    tx.execute(
        "UPDATE favorite_albums SET (title, artist, artwork_id) =
             (SELECT al.title, COALESCE(ar.name, ''), al.artwork_id FROM albums al
              LEFT JOIN artists ar ON ar.id = al.artist_id WHERE al.key = album_key)
         WHERE album_key IN (SELECT key FROM albums)",
        [],
    )?;
    Ok(())
}

/// Drops sources no playlist, favorite or play refers to.
pub(crate) fn prune_sources(tx: &Transaction) -> rusqlite::Result<usize> {
    tx.execute(
        "DELETE FROM media_sources WHERE
             NOT EXISTS (SELECT 1 FROM playlist_entries WHERE source_id = media_sources.id)
             AND NOT EXISTS (SELECT 1 FROM favorite_tracks WHERE source_id = media_sources.id)
             AND NOT EXISTS (SELECT 1 FROM plays WHERE source_id = media_sources.id)",
        [],
    )
}

/// Relinks a tombstone to a newly indexed file, only on strong evidence:
///
/// 1. the same absolute path, and either the same size and timestamp or the
///    same title with a duration within two seconds (a re-tagged file); or
/// 2. the same path below its root, size, timestamp and duration (a moved
///    folder or changed drive letter), with exactly one such tombstone.
///
/// A matching title is never enough by itself.
pub(crate) fn reconnect(tx: &Transaction, track_id: i64) -> rusqlite::Result<bool> {
    // Almost always empty, and indexed, so ordinary scans pay one lookup.
    let any: bool = tx
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM media_sources WHERE track_id IS NULL)")?
        .query_row([], |row| row.get(0))?;
    if !any {
        return Ok(false);
    }
    if tx
        .prepare_cached("SELECT 1 FROM media_sources WHERE track_id = ?1")?
        .exists([track_id])?
    {
        return Ok(false);
    }
    let (root, rel, size, mtime, title, duration): (String, String, i64, i64, String, i64) = tx
        .query_row(
            "SELECT r.path, t.rel_path, t.size, t.mtime_ns, t.title, t.duration_ms
             FROM tracks t JOIN library_roots r ON r.id = t.root_id WHERE t.id = ?1",
            [track_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
    let same_path: Vec<(i64, i64, i64, String, i64)> = {
        let mut statement = tx.prepare_cached(
            "SELECT id, size, mtime_ns, title, duration_ms FROM media_sources
             WHERE track_id IS NULL AND abs_key = ?1 ORDER BY id",
        )?;
        statement
            .query_map([absolute_key(&root, &rel)], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })?
            .collect::<Result<_, _>>()?
    };
    let by_path = same_path.iter().find(|(_, s, m, name, length)| {
        (*s == size && *m == mtime)
            || (key(name) == key(&title) && (length - duration).abs() <= DURATION_TOLERANCE_MS)
    });
    let found = match by_path {
        Some((id, ..)) => Some(*id),
        None => {
            let mut statement = tx.prepare_cached(
                "SELECT id FROM media_sources WHERE track_id IS NULL AND size = ?1
                 AND mtime_ns = ?2 AND rel_key = ?3 AND duration_ms = ?4 LIMIT 2",
            )?;
            let matches: Vec<i64> = statement
                .query_map(params![size, mtime, path_key(&rel), duration], |row| {
                    row.get(0)
                })?
                .collect::<Result<_, _>>()?;
            (matches.len() == 1).then(|| matches[0])
        }
    };
    let Some(source) = found else {
        return Ok(false);
    };
    tx.execute(
        "UPDATE media_sources SET track_id = ?2, last_track_id = ?2 WHERE id = ?1",
        params![source, track_id],
    )?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

pub(crate) const MEDIA_COLUMNS: &str =
    "m.id, t.id, COALESCE(t.title, m.title), COALESCE(t.artist_text, m.artist),
    CASE WHEN t.id IS NULL THEN m.album ELSE COALESCE(a.title, '') END, t.album_id,
    COALESCE(t.artwork_id, a.artwork_id, m.artwork_id), COALESCE(t.duration_ms, m.duration_ms),
    COALESCE(t.state = 0 AND r.online = 1, 0),
    EXISTS (SELECT 1 FROM favorite_tracks ft WHERE ft.source_id = m.id)";
const MEDIA_JOINS: &str = "LEFT JOIN tracks t ON t.id = m.track_id
    LEFT JOIN library_roots r ON r.id = t.root_id LEFT JOIN albums a ON a.id = t.album_id";

fn media_row(row: &Row) -> rusqlite::Result<MediaRow> {
    Ok(MediaRow {
        source_id: id32(row.get(0)?),
        track_id: row.get::<_, Option<i64>>(1)?.map(id32),
        title: row.get(2)?,
        artist: row.get(3)?,
        album: row.get(4)?,
        album_id: row.get::<_, Option<i64>>(5)?.map(id32),
        artwork_id: row.get::<_, Option<i64>>(6)?.map(id32),
        duration_ms: id32(row.get(7)?),
        available: row.get(8)?,
        favorite: row.get(9)?,
    })
}

fn bounds(offset: u32, limit: u32) -> Result<(i64, i64), QueryError> {
    if limit == 0 || limit > MAX_PAGE {
        return Err(QueryError::Invalid);
    }
    Ok((i64::from(offset), i64::from(limit)))
}

const CARD_COLUMNS: &str = "p.id, p.name, p.description, p.updated_at,
    (SELECT COUNT(*) FROM playlist_entries e WHERE e.playlist_id = p.id),
    (SELECT COUNT(*) FROM playlist_entries e JOIN media_sources m ON m.id = e.source_id
        LEFT JOIN tracks t ON t.id = m.track_id LEFT JOIN library_roots r ON r.id = t.root_id
        WHERE e.playlist_id = p.id AND COALESCE(t.state = 0 AND r.online = 1, 0) = 0),
    (SELECT COALESCE(SUM(COALESCE(t.duration_ms, m.duration_ms)), 0) FROM playlist_entries e
        JOIN media_sources m ON m.id = e.source_id LEFT JOIN tracks t ON t.id = m.track_id
        WHERE e.playlist_id = p.id)";

fn card(db: &Connection, row: &Row) -> rusqlite::Result<PlaylistCard> {
    let id: i64 = row.get(0)?;
    let mut covers = db.prepare_cached(
        "SELECT art FROM (
             SELECT COALESCE(t.artwork_id, a.artwork_id, m.artwork_id) AS art, MIN(e.position) AS first
             FROM playlist_entries e JOIN media_sources m ON m.id = e.source_id
             LEFT JOIN tracks t ON t.id = m.track_id LEFT JOIN albums a ON a.id = t.album_id
             WHERE e.playlist_id = ?1 GROUP BY art)
         WHERE art IS NOT NULL ORDER BY first LIMIT 4",
    )?;
    Ok(PlaylistCard {
        id: id32(id),
        name: row.get(1)?,
        description: row.get(2)?,
        updated_at: row.get::<_, i64>(3)? as f64,
        tracks: id32(row.get(4)?),
        unavailable: id32(row.get(5)?),
        duration_ms: row.get::<_, i64>(6)? as f64,
        artwork_ids: covers
            .query_map([id], |row| row.get::<_, i64>(0).map(id32))?
            .collect::<Result<_, _>>()?,
    })
}

pub(crate) fn playlists(db: &Connection) -> Result<Vec<PlaylistCard>, QueryError> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {CARD_COLUMNS} FROM playlists p ORDER BY p.updated_at DESC, p.id DESC"
    ))?;
    Ok(statement
        .query_map([], |row| card(db, row))?
        .collect::<Result<_, _>>()?)
}

pub(crate) fn playlist(db: &Connection, id: u32) -> Result<Option<PlaylistCard>, QueryError> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {CARD_COLUMNS} FROM playlists p WHERE p.id = ?1"
    ))?;
    Ok(statement
        .query_row([i64::from(id)], |row| card(db, row))
        .optional()?)
}

pub(crate) fn playlist_entries(
    db: &Connection,
    id: u32,
    offset: u32,
    limit: u32,
) -> Result<PlaylistPage, QueryError> {
    let (from, count) = bounds(offset, limit)?;
    let total: i64 = db.query_row(
        "SELECT COUNT(*) FROM playlist_entries WHERE playlist_id = ?1",
        [i64::from(id)],
        |row| row.get(0),
    )?;
    let mut statement = db.prepare_cached(&format!(
        "SELECT {MEDIA_COLUMNS}, e.id FROM playlist_entries e
         JOIN media_sources m ON m.id = e.source_id {MEDIA_JOINS}
         WHERE e.playlist_id = ?1 ORDER BY e.position LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = statement
        .query_map(params![i64::from(id), count, from], |row| {
            Ok(PlaylistEntry {
                entry_id: id32(row.get(10)?),
                media: media_row(row)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(PlaylistPage {
        total: id32(total),
        offset,
        rows,
    })
}

pub(crate) fn favorite_tracks(
    db: &Connection,
    offset: u32,
    limit: u32,
) -> Result<MediaPage, QueryError> {
    let (from, count) = bounds(offset, limit)?;
    let total: i64 = db.query_row("SELECT COUNT(*) FROM favorite_tracks", [], |row| row.get(0))?;
    let mut statement = db.prepare_cached(&format!(
        "SELECT {MEDIA_COLUMNS} FROM favorite_tracks f
         JOIN media_sources m ON m.id = f.source_id {MEDIA_JOINS}
         ORDER BY f.created_at DESC, f.source_id DESC LIMIT ?1 OFFSET ?2"
    ))?;
    let rows = statement
        .query_map(params![count, from], media_row)?
        .collect::<Result<_, _>>()?;
    Ok(MediaPage {
        total: id32(total),
        offset,
        rows,
    })
}

pub(crate) fn favorite_albums(
    db: &Connection,
    offset: u32,
    limit: u32,
) -> Result<Vec<FavoriteAlbum>, QueryError> {
    let (from, count) = bounds(offset, limit)?;
    let mut statement = db.prepare_cached(
        "SELECT f.id, f.title, f.artist, f.artwork_id, al.id FROM favorite_albums f
         LEFT JOIN albums al ON al.key = f.album_key
         ORDER BY f.created_at DESC, f.id DESC LIMIT ?1 OFFSET ?2",
    )?;
    type Snapshot = (i64, String, String, Option<i64>, Option<i64>);
    let rows: Vec<Snapshot> = statement
        .query_map(params![count, from], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    rows.into_iter()
        .map(|(id, title, artist, art, album)| {
            let album = match album {
                Some(album) => query::album(db, id32(album))?,
                None => None,
            };
            Ok(FavoriteAlbum {
                favorite_id: id32(id),
                title,
                artist,
                artwork_id: art.map(id32),
                album,
            })
        })
        .collect()
}

pub(crate) fn history(db: &Connection, offset: u32, limit: u32) -> Result<HistoryPage, QueryError> {
    let (from, count) = bounds(offset, limit)?;
    let total: i64 = db.query_row("SELECT COUNT(*) FROM plays", [], |row| row.get(0))?;
    let mut statement = db.prepare_cached(&format!(
        "SELECT {MEDIA_COLUMNS}, p.id, p.started_at, p.listened_ms FROM plays p
         JOIN media_sources m ON m.id = p.source_id {MEDIA_JOINS}
         ORDER BY p.started_at DESC, p.id DESC LIMIT ?1 OFFSET ?2"
    ))?;
    let rows = statement
        .query_map(params![count, from], |row| {
            Ok(HistoryRow {
                play_id: id32(row.get(10)?),
                started_at: row.get::<_, i64>(11)? as f64,
                listened_ms: row.get::<_, i64>(12)? as f64,
                media: media_row(row)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(HistoryPage {
        total: id32(total),
        offset,
        rows,
    })
}

fn recent_albums(db: &Connection, limit: i64) -> Result<Vec<AlbumRow>, QueryError> {
    let mut statement = db.prepare_cached(
        "SELECT al.id FROM (SELECT album_key, MAX(started_at) AS last FROM plays
             WHERE album_key IS NOT NULL GROUP BY album_key) p
         JOIN albums al ON al.key = p.album_key ORDER BY p.last DESC, al.id LIMIT ?1",
    )?;
    let ids: Vec<i64> = statement
        .query_map([limit], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let mut albums = Vec::with_capacity(ids.len());
    for id in ids {
        albums.extend(query::album(db, id32(id))?);
    }
    Ok(albums)
}

pub(crate) fn home(db: &Connection) -> Result<HomeView, QueryError> {
    let frequent = {
        let mut statement = db.prepare_cached(&format!(
            "SELECT {MEDIA_COLUMNS} FROM (SELECT source_id, COUNT(*) AS n, MAX(started_at) AS last
                 FROM plays GROUP BY source_id HAVING n >= 2 ORDER BY n DESC, last DESC LIMIT 6) f
             JOIN media_sources m ON m.id = f.source_id {MEDIA_JOINS} ORDER BY f.n DESC, f.last DESC"
        ))?;
        statement
            .query_map([], media_row)?
            .collect::<Result<_, _>>()?
    };
    let mut playlists = playlists(db)?;
    playlists.truncate(6);
    Ok(HomeView {
        recent_albums: recent_albums(db, 10)?,
        added_albums: query::albums_page(
            db,
            &AlbumQuery {
                sort: AlbumSort::Recent,
                artist_id: None,
                role: None,
                offset: 0,
                limit: 10,
            },
        )?
        .rows,
        favorite_albums: favorite_albums(db, 0, 10)?,
        recent_tracks: history(db, 0, 6)?.rows,
        frequent,
        playlists,
    })
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

fn immediate(db: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    db.transaction_with_behavior(TransactionBehavior::Immediate)
}

fn playlist_name(raw: &str) -> Result<String, UserError> {
    let name: String = clean(raw).chars().take(120).collect();
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(UserError::Invalid);
    }
    Ok(name)
}

fn description(raw: &str) -> String {
    clean(raw).chars().take(500).collect()
}

fn touch(tx: &Transaction, id: i64, now: i64) -> Result<(), UserError> {
    if tx.execute(
        "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
        params![id, now],
    )? == 0
    {
        return Err(UserError::NotFound);
    }
    Ok(())
}

fn append(tx: &Transaction, playlist: i64, tracks: &[i64], now: i64) -> Result<u32, UserError> {
    let (count, last): (i64, i64) = tx.query_row(
        "SELECT COUNT(*), COALESCE(MAX(position), 0) FROM playlist_entries WHERE playlist_id = ?1",
        [playlist],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if tracks.len() > MAX_BATCH || count + tracks.len() as i64 > MAX_ENTRIES {
        return Err(UserError::Invalid);
    }
    let mut position = last;
    for track in tracks {
        // All or nothing: one unknown id rolls the whole batch back.
        let source = ensure_source(tx, *track)?.ok_or(UserError::NotFound)?;
        position = position.checked_add(GAP).ok_or(UserError::Invalid)?;
        tx.execute(
            "INSERT INTO playlist_entries (playlist_id, source_id, position, added_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![playlist, source, position, now],
        )?;
    }
    Ok(tracks.len() as u32)
}

/// Rewrites positions with full gaps. Two passes through negative space keep
/// the uniqueness constraint satisfied at every statement.
fn renumber(tx: &Transaction, playlist: i64) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE playlist_entries SET position = -n.rank FROM (
             SELECT id, ROW_NUMBER() OVER (ORDER BY position) AS rank
             FROM playlist_entries WHERE playlist_id = ?1) n
         WHERE playlist_entries.id = n.id",
        [playlist],
    )?;
    tx.execute(
        "UPDATE playlist_entries SET position = -position * ?2 WHERE playlist_id = ?1",
        params![playlist, GAP],
    )?;
    Ok(())
}

fn move_entry(
    tx: &Transaction,
    playlist: i64,
    entry: i64,
    before: Option<i64>,
) -> Result<(), UserError> {
    if before == Some(entry) {
        return Ok(());
    }
    let place = |tx: &Transaction, id: i64| -> Result<i64, UserError> {
        tx.query_row(
            "SELECT position FROM playlist_entries WHERE id = ?1 AND playlist_id = ?2",
            params![id, playlist],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(UserError::NotFound)
    };
    place(tx, entry)?;
    for attempt in 0..2 {
        let upper = before.map(|id| place(tx, id)).transpose()?;
        let lower: i64 = tx.query_row(
            "SELECT COALESCE(MAX(position), 0) FROM playlist_entries
             WHERE playlist_id = ?1 AND id <> ?2 AND (?3 IS NULL OR position < ?3)",
            params![playlist, entry, upper],
            |row| row.get(0),
        )?;
        let target = match upper {
            Some(upper) => lower + (upper - lower) / 2,
            None => lower.checked_add(GAP).ok_or(UserError::Invalid)?,
        };
        if target > lower {
            tx.execute(
                "UPDATE playlist_entries SET position = ?2 WHERE id = ?1",
                params![entry, target],
            )?;
            return Ok(());
        }
        if attempt == 0 {
            renumber(tx, playlist)?;
        }
    }
    Err(UserError::Invalid)
}

pub(crate) fn act(
    db: &mut Connection,
    action: &PlaylistAct,
    now: i64,
) -> Result<PlaylistOutcome, UserError> {
    let tx = immediate(db)?;
    let ids = |values: &[u32]| values.iter().map(|id| i64::from(*id)).collect::<Vec<_>>();
    let outcome = match action {
        PlaylistAct::Create {
            name,
            description: text,
            track_ids,
        } => {
            let count: i64 =
                tx.query_row("SELECT COUNT(*) FROM playlists", [], |row| row.get(0))?;
            if count >= MAX_PLAYLISTS {
                return Err(UserError::Invalid);
            }
            let id: i64 = tx.query_row(
                "INSERT INTO playlists (name, description, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3) RETURNING id",
                params![playlist_name(name)?, description(text), now],
                |row| row.get(0),
            )?;
            PlaylistOutcome {
                id: id32(id),
                changed: append(&tx, id, &ids(track_ids), now)?,
            }
        }
        PlaylistAct::Edit {
            id,
            name,
            description: text,
        } => {
            touch(&tx, i64::from(*id), now)?;
            tx.execute(
                "UPDATE playlists SET name = ?2, description = ?3 WHERE id = ?1",
                params![i64::from(*id), playlist_name(name)?, description(text)],
            )?;
            PlaylistOutcome {
                id: *id,
                changed: 0,
            }
        }
        PlaylistAct::Delete { id } => {
            let entries: i64 = tx.query_row(
                "SELECT COUNT(*) FROM playlist_entries WHERE playlist_id = ?1",
                [i64::from(*id)],
                |row| row.get(0),
            )?;
            if tx.execute("DELETE FROM playlists WHERE id = ?1", [i64::from(*id)])? == 0 {
                return Err(UserError::NotFound);
            }
            PlaylistOutcome {
                id: *id,
                changed: id32(entries),
            }
        }
        PlaylistAct::AddTracks { id, track_ids } => {
            touch(&tx, i64::from(*id), now)?;
            PlaylistOutcome {
                id: *id,
                changed: append(&tx, i64::from(*id), &ids(track_ids), now)?,
            }
        }
        PlaylistAct::AddAlbum { id, album_id } => {
            touch(&tx, i64::from(*id), now)?;
            let tracks: Vec<i64> = {
                let mut statement = tx.prepare(&format!(
                    "SELECT t.id FROM tracks t WHERE t.album_id = ?1 ORDER BY {}",
                    query::ALBUM_ORDER
                ))?;
                statement
                    .query_map([i64::from(*album_id)], |row| row.get(0))?
                    .collect::<Result<_, _>>()?
            };
            if tracks.is_empty() {
                return Err(UserError::NotFound);
            }
            PlaylistOutcome {
                id: *id,
                changed: append(&tx, i64::from(*id), &tracks, now)?,
            }
        }
        PlaylistAct::Remove { id, entry_ids } => {
            if entry_ids.len() > MAX_BATCH {
                return Err(UserError::Invalid);
            }
            touch(&tx, i64::from(*id), now)?;
            let mut changed = 0;
            for entry in entry_ids {
                changed += tx.execute(
                    "DELETE FROM playlist_entries WHERE id = ?1 AND playlist_id = ?2",
                    params![i64::from(*entry), i64::from(*id)],
                )?;
            }
            PlaylistOutcome {
                id: *id,
                changed: changed as u32,
            }
        }
        PlaylistAct::Move {
            id,
            entry_id,
            before,
        } => {
            touch(&tx, i64::from(*id), now)?;
            move_entry(
                &tx,
                i64::from(*id),
                i64::from(*entry_id),
                before.map(i64::from),
            )?;
            PlaylistOutcome {
                id: *id,
                changed: 1,
            }
        }
    };
    tx.commit()?;
    Ok(outcome)
}

pub(crate) fn set_favorite(
    db: &mut Connection,
    target: FavoriteTarget,
    on: bool,
    now: i64,
) -> Result<(), UserError> {
    let tx = immediate(db)?;
    let set_source = |source: i64| -> Result<(), UserError> {
        if on {
            tx.execute(
                "INSERT OR IGNORE INTO favorite_tracks (source_id, created_at) VALUES (?1, ?2)",
                params![source, now],
            )?;
        } else {
            tx.execute("DELETE FROM favorite_tracks WHERE source_id = ?1", [source])?;
        }
        Ok(())
    };
    match target {
        FavoriteTarget::Track { id } => {
            let source = ensure_source(&tx, i64::from(id))?.ok_or(UserError::NotFound)?;
            set_source(source)?;
        }
        FavoriteTarget::Source { id } => {
            let known = tx
                .prepare("SELECT 1 FROM media_sources WHERE id = ?1")?
                .exists([i64::from(id)])?;
            if !known {
                return Err(UserError::NotFound);
            }
            set_source(i64::from(id))?;
        }
        FavoriteTarget::Album { id } => {
            let changed = if on {
                tx.execute(
                    "INSERT INTO favorite_albums (album_key, title, artist, artwork_id, created_at)
                     SELECT al.key, al.title, COALESCE(ar.name, ''), al.artwork_id, ?2
                     FROM albums al LEFT JOIN artists ar ON ar.id = al.artist_id WHERE al.id = ?1
                     ON CONFLICT(album_key) DO UPDATE SET title = excluded.title",
                    params![i64::from(id), now],
                )?
            } else {
                tx.execute(
                    "DELETE FROM favorite_albums
                     WHERE album_key = (SELECT key FROM albums WHERE id = ?1)",
                    [i64::from(id)],
                )?
            };
            if on && changed == 0 {
                return Err(UserError::NotFound);
            }
        }
        FavoriteTarget::FavoriteAlbum { id } => {
            // Only removal makes sense for a favorite without an album.
            if on {
                return Err(UserError::Invalid);
            }
            tx.execute("DELETE FROM favorite_albums WHERE id = ?1", [i64::from(id)])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Starts a history row. Returns its id, or `None` when the track is unknown.
pub(crate) fn record_play(
    db: &mut Connection,
    track_id: u32,
    started_at: i64,
    listened_ms: u64,
) -> Result<Option<i64>, UserError> {
    let tx = immediate(db)?;
    let Some(source) = ensure_source(&tx, i64::from(track_id))? else {
        return Ok(None);
    };
    let id: i64 = tx.query_row(
        "INSERT INTO plays (source_id, album_key, started_at, listened_ms)
         VALUES (?1, (SELECT al.key FROM tracks t JOIN albums al ON al.id = t.album_id
                      WHERE t.id = ?2), ?3, ?4) RETURNING id",
        params![source, i64::from(track_id), started_at, listened_ms as i64],
        |row| row.get(0),
    )?;
    tx.execute(
        "DELETE FROM plays WHERE id <= (SELECT MAX(id) FROM plays) - ?1",
        [MAX_PLAYS],
    )?;
    tx.commit()?;
    Ok(Some(id))
}

pub(crate) fn update_play(db: &Connection, id: i64, listened_ms: u64) -> Result<(), UserError> {
    db.execute(
        "UPDATE plays SET listened_ms = MAX(listened_ms, ?2) WHERE id = ?1",
        params![id, listened_ms as i64],
    )?;
    Ok(())
}

/// Clears history only. Playlists and favorites are untouched.
pub(crate) fn clear_history(
    db: &mut Connection,
    scope: HistoryScope,
    now: i64,
) -> Result<u32, UserError> {
    let since = match scope {
        HistoryScope::LastHour => now - 3_600,
        HistoryScope::LastDay => now - 86_400,
        HistoryScope::All => i64::MIN,
    };
    let tx = immediate(db)?;
    let removed = tx.execute("DELETE FROM plays WHERE started_at >= ?1", [since])?;
    prune_sources(&tx)?;
    tx.commit()?;
    Ok(removed as u32)
}

#[cfg(test)]
mod tests;
