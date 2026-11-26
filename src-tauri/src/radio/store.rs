//! Durable station identities, with a separate expiry policy for discovery data.
use crate::{
    contract::{RadioFault, RadioSort, Station, StationEdit, StationShelf},
    library::{self, text::clean},
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use std::path::Path;

use super::source_url;

pub(super) const PAGE: u32 = 24;
const MAX_MANUAL: u32 = 1_024;
const MAX_FAVORITES: u32 = 2_048;
const COLUMNS: &str = "id, name, country, language, tags, codec, bitrate, reported_online,
    manual, favorite, artwork_url IS NOT NULL";

pub(super) struct Reference {
    pub station: Station,
    pub stream: String,
    pub artwork: Option<String>,
}

pub(super) fn database(path: &Path) -> Result<Connection, RadioFault> {
    library::open(path).map_err(|_| RadioFault::Storage)
}

pub(super) fn storage(_: rusqlite::Error) -> RadioFault {
    RadioFault::Storage
}

fn row_station(row: &Row<'_>) -> rusqlite::Result<Station> {
    Ok(Station {
        id: row.get(0)?,
        name: row.get(1)?,
        country: row.get(2)?,
        language: row.get(3)?,
        tags: row.get(4)?,
        codec: row.get(5)?,
        bitrate: row.get(6)?,
        reported_online: row.get(7)?,
        manual: row.get(8)?,
        favorite: row.get(9)?,
        artwork: row.get(10)?,
    })
}

pub(super) fn station(db: &Connection, id: &str) -> Result<Reference, RadioFault> {
    if id.len() > 40 {
        return Err(RadioFault::Invalid);
    }
    db.query_row(
        &format!("SELECT {COLUMNS}, stream_url, artwork_url FROM radio_stations WHERE id = ?1"),
        [id],
        |row| {
            Ok(Reference {
                station: row_station(row)?,
                stream: row.get(11)?,
                artwork: row.get(12)?,
            })
        },
    )
    .optional()
    .map_err(storage)?
    .ok_or(RadioFault::NotFound)
}

pub(super) fn shelf(
    db: &Connection,
    category: StationShelf,
    offset: u32,
) -> Result<(Vec<Station>, Option<u32>), RadioFault> {
    if offset > MAX_MANUAL + MAX_FAVORITES {
        return Err(RadioFault::Invalid);
    }
    let selection = match category {
        StationShelf::Favorites => "favorite = 1 ORDER BY name COLLATE NOCASE, id",
        StationShelf::Manual => "manual = 1 ORDER BY name COLLATE NOCASE, id",
        StationShelf::Recent => "heard_at IS NOT NULL ORDER BY heard_at DESC, id",
    };
    let mut statement = db
        .prepare(&format!(
            "SELECT {COLUMNS} FROM radio_stations WHERE {selection} LIMIT ?1 OFFSET ?2"
        ))
        .map_err(storage)?;
    let mut rows = statement
        .query_map(params![PAGE + 1, offset], row_station)
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let next = (rows.len() > PAGE as usize).then_some(offset + PAGE);
    rows.truncate(PAGE as usize);
    Ok((rows, next))
}

pub(super) fn save(db: &mut Connection, edit: StationEdit) -> Result<Station, RadioFault> {
    let name = clean(&edit.name);
    if name.is_empty()
        || edit.id.as_ref().is_some_and(|id| id.len() > 40)
        || [&edit.name, &edit.country, &edit.language, &edit.tags]
            .iter()
            .any(|value| value.len() > 2048)
    {
        return Err(RadioFault::Invalid);
    }
    let url = source_url(edit.url.trim()).map_err(|_| RadioFault::InvalidSource)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let id = if let Some(id) = edit.id {
        if !station(&tx, &id)?.station.manual {
            return Err(RadioFault::NotFound);
        }
        id
    } else {
        let count: u32 = tx
            .query_row(
                "SELECT count(*) FROM radio_stations WHERE manual = 1",
                [],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if count >= MAX_MANUAL {
            return Err(RadioFault::Invalid);
        }
        tx.query_row("SELECT 'local:' || lower(hex(randomblob(16)))", [], |row| {
            row.get(0)
        })
        .map_err(storage)?
    };
    tx.execute(
        "INSERT INTO radio_stations
         (id, name, stream_url, country, language, tags, codec, bitrate, manual, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', 0, 1, unixepoch())
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, stream_url = excluded.stream_url,
           country = excluded.country, language = excluded.language, tags = excluded.tags,
           updated_at = excluded.updated_at",
        params![
            id,
            name,
            url.as_str(),
            clean(&edit.country),
            clean(&edit.language),
            clean(&edit.tags)
        ],
    )
    .map_err(storage)?;
    let saved = station(&tx, &id)?.station;
    tx.commit().map_err(storage)?;
    Ok(saved)
}

pub(super) fn remove(db: &Connection, id: &str) -> Result<(), RadioFault> {
    if id.len() > 40 {
        return Err(RadioFault::Invalid);
    }
    let affected = db
        .execute(
            "DELETE FROM radio_stations WHERE id = ?1 AND manual = 1",
            [id],
        )
        .map_err(storage)?;
    if affected == 0 {
        return Err(RadioFault::NotFound);
    }
    Ok(())
}

pub(super) fn favorite(
    db: &mut Connection,
    id: &str,
    enabled: bool,
) -> Result<Station, RadioFault> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let previous = station(&tx, id)?.station;
    if enabled && !previous.favorite {
        let count: u32 = tx
            .query_row(
                "SELECT count(*) FROM radio_stations WHERE favorite = 1",
                [],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if count >= MAX_FAVORITES {
            return Err(RadioFault::Invalid);
        }
    }
    tx.execute(
        "UPDATE radio_stations SET favorite = ?2 WHERE id = ?1",
        params![id, enabled],
    )
    .map_err(storage)?;
    let saved = station(&tx, id)?.station;
    tx.commit().map_err(storage)?;
    Ok(saved)
}

pub(super) fn remember_directory(
    db: &mut Connection,
    key: &str,
    entries: &[Reference],
    has_next: bool,
) -> Result<(), RadioFault> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    for entry in entries {
        let station = &entry.station;
        tx.execute(
            "INSERT INTO radio_stations
             (id, name, stream_url, artwork_url, country, language, tags, codec, bitrate,
              reported_online, manual, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, unixepoch())
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, stream_url = excluded.stream_url,
             artwork_url = excluded.artwork_url, country = excluded.country, language = excluded.language,
             tags = excluded.tags, codec = excluded.codec, bitrate = excluded.bitrate,
             reported_online = excluded.reported_online, updated_at = excluded.updated_at",
            params![
                station.id, station.name, entry.stream, entry.artwork, station.country, station.language,
                station.tags, station.codec, station.bitrate, station.reported_online
            ],
        ).map_err(storage)?;
    }
    let ids: Vec<_> = entries.iter().map(|entry| &entry.station.id).collect();
    let response = serde_json::to_string(&(ids, has_next)).map_err(|_| RadioFault::Storage)?;
    tx.execute(
        "INSERT INTO radio_directory_cache VALUES (?1, ?2, unixepoch())
         ON CONFLICT(query) DO UPDATE SET response = excluded.response, fetched_at = excluded.fetched_at",
        params![key, response],
    ).map_err(storage)?;
    tx.execute_batch(
        "DELETE FROM radio_directory_cache WHERE fetched_at < unixepoch() - 86400;
         DELETE FROM radio_directory_cache WHERE query IN (
             SELECT query FROM radio_directory_cache ORDER BY fetched_at DESC, rowid DESC LIMIT -1 OFFSET 96
         );
         DELETE FROM radio_stations WHERE id IN (
             SELECT id FROM radio_stations WHERE manual = 0 AND favorite = 0 AND heard_at IS NULL
             ORDER BY updated_at DESC, rowid DESC LIMIT -1 OFFSET 4096
         );",
    ).map_err(storage)?;
    tx.commit().map_err(storage)
}

pub(super) fn cached(
    db: &Connection,
    key: &str,
) -> Result<Option<(Vec<Station>, bool, bool)>, RadioFault> {
    let data: Option<(String, bool)> = db
        .query_row(
            "SELECT response, fetched_at < unixepoch() - 600 FROM radio_directory_cache
         WHERE query = ?1 AND fetched_at >= unixepoch() - 86400",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    let Some((response, stale)) = data else {
        return Ok(None);
    };
    let (ids, next): (Vec<String>, bool) =
        serde_json::from_str(&response).map_err(|_| RadioFault::Storage)?;
    let stations = ids
        .iter()
        .map(|id| station(db, id).map(|entry| entry.station))
        .collect::<Result<Vec<_>, _>>();
    match stations {
        Ok(stations) => Ok(Some((stations, next, stale))),
        Err(RadioFault::NotFound) => Ok(None),
        Err(fault) => Err(fault),
    }
}

pub(super) fn preference(
    db: &Connection,
    sort: Option<RadioSort>,
) -> Result<RadioSort, RadioFault> {
    if let Some(sort) = sort {
        let value = match sort {
            RadioSort::Popular => "popular",
            RadioSort::Name => "name",
            RadioSort::Recent => "recent",
        };
        db.execute(
            "INSERT INTO radio_preferences VALUES (1, ?1)
            ON CONFLICT(id) DO UPDATE SET sort = excluded.sort",
            [value],
        )
        .map_err(storage)?;
        return Ok(sort);
    }
    let stored: Option<String> = db
        .query_row(
            "SELECT sort FROM radio_preferences WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    Ok(match stored.as_deref() {
        Some("recent") => RadioSort::Recent,
        Some("name") => RadioSort::Name,
        _ => RadioSort::Popular,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(id: Option<String>, name: &str, url: &str) -> StationEdit {
        StationEdit {
            id,
            name: name.into(),
            url: url.into(),
            country: "PL".into(),
            language: "Polish".into(),
            tags: "jazz".into(),
        }
    }

    #[test]
    fn station_identity_favorite_and_preferences_survive_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.sqlite3");
        let mut db = database(&path).unwrap();
        let saved = save(
            &mut db,
            edit(None, "After Dark", "https://example.invalid/live"),
        )
        .unwrap();
        let id = saved.id;
        favorite(&mut db, &id, true).unwrap();
        preference(&db, Some(RadioSort::Name)).unwrap();
        assert_eq!(
            save(&mut db, edit(Some(id.clone()), "Invalid", "http://[::1]/")).unwrap_err(),
            RadioFault::InvalidSource
        );
        let saved = save(
            &mut db,
            edit(Some(id.clone()), "Late Hour", "https://example.invalid/new"),
        )
        .unwrap();
        assert_eq!(saved.id, id);
        assert!(saved.favorite);
        drop(db);

        let db = database(&path).unwrap();
        assert!(matches!(preference(&db, None).unwrap(), RadioSort::Name));
        let (favorites, _) = shelf(&db, StationShelf::Favorites, 0).unwrap();
        assert_eq!(favorites[0].name, "Late Hour");
        assert_eq!(
            station(&db, &id).unwrap().stream,
            "https://example.invalid/new"
        );
        remove(&db, &id).unwrap();
        assert!(shelf(&db, StationShelf::Manual, 0).unwrap().0.is_empty());
        assert!(shelf(&db, StationShelf::Favorites, 0).unwrap().0.is_empty());
    }

    #[test]
    fn expiry_preserves_saved_references_and_limits_cached_pages() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.sqlite3");
        let mut db = database(&path).unwrap();
        let saved = save(
            &mut db,
            edit(None, "Station", "https://example.invalid/stream"),
        )
        .unwrap();
        for i in 0..100 {
            remember_directory(&mut db, &i.to_string(), &[], true).unwrap();
        }
        assert!(cached(&db, "0").unwrap().is_none());
        assert!(cached(&db, "99").unwrap().unwrap().1);
        db.execute(
            "UPDATE radio_directory_cache SET fetched_at = unixepoch() - 700 WHERE query = '99'",
            [],
        )
        .unwrap();
        assert!(cached(&db, "99").unwrap().unwrap().2);
        db.execute(
            "UPDATE radio_directory_cache SET fetched_at = unixepoch() - 86401",
            [],
        )
        .unwrap();
        assert!(cached(&db, "99").unwrap().is_none());
        assert_eq!(
            station(&db, &saved.id).unwrap().stream,
            "https://example.invalid/stream"
        );
    }
}
