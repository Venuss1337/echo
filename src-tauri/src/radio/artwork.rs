//! The renderer can request one known station's re-encoded thumbnail.
use super::{RequestKind, bounded_body, source_url, store};
use crate::{
    contract::RadioFault,
    library::{artwork, palette},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::path::PathBuf;

const MAX_REMOTE_BYTES: usize = 768 * 1024;
const MAX_RASTER_BYTES: usize = 128 * 1024;

/// Reads the current station's bounded, already re-encoded artwork for SMTC.
pub(crate) fn system_thumbnail(db: &rusqlite::Connection, id: &str) -> Option<Vec<u8>> {
    db.query_row(
        "SELECT c.raster FROM radio_artwork_cache c
         JOIN radio_stations s ON s.id = c.id AND s.artwork_url = c.source
         WHERE c.id = ?1 AND length(c.raster) BETWEEN 1 AND ?2
         AND c.fetched_at >= unixepoch() - 86400",
        params![id, MAX_RASTER_BYTES as i64],
        |row| row.get(0),
    )
    .ok()
}

pub(super) async fn image(path: PathBuf, id: String) -> Result<Vec<u8>, RadioFault> {
    let read_path = path.clone();
    let read_id = id.clone();
    let (source, cached) = tauri::async_runtime::spawn_blocking(move || {
        let db = store::database(&read_path)?;
        let source = store::station(&db, &read_id)?.artwork;
        let cached = db
            .query_row(
                "SELECT raster, palette FROM radio_artwork_cache
             WHERE id = ?1 AND source = ?2
             AND fetched_at >= unixepoch() - CASE WHEN length(raster) = 0 THEN 600 ELSE 86400 END
             AND length(raster) <= ?3",
                params![read_id, source, MAX_RASTER_BYTES as i64],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(store::storage)?;
        Ok::<_, RadioFault>((source, cached))
    })
    .await
    .map_err(|_| RadioFault::Unavailable)??;
    if let Some((raster, palette)) = cached {
        return Ok(pack(raster, palette));
    }
    let Some(source) = source else {
        return Ok(Vec::new());
    };
    let url = source_url(&source).map_err(|_| RadioFault::InvalidSource)?;
    let bytes = bounded_body(url, RequestKind::Artwork, MAX_REMOTE_BYTES)
        .await
        .ok();
    tauri::async_runtime::spawn_blocking(move || {
        let (raster, palette) = bytes.as_deref()
            .and_then(|bytes| artwork::make_station_thumbnail(bytes).ok())
            .filter(|image| image.jpeg.len() <= MAX_RASTER_BYTES)
            .and_then(|image| {
                let colors = palette::extract(&image.jpeg)?;
                Some((image.jpeg, format!("{}{}{}", colors.ink, colors.glow, colors.accent)))
            }).unwrap_or_default();
        let mut db = store::database(&path)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(store::storage)?;
        // A refreshed directory row may have changed its source during decode.
        tx.execute(
            "INSERT INTO radio_artwork_cache
             SELECT id, ?2, ?3, ?4, unixepoch() FROM radio_stations WHERE id = ?1 AND artwork_url = ?2
             ON CONFLICT(id) DO UPDATE SET source = excluded.source, raster = excluded.raster,
             palette = excluded.palette, fetched_at = excluded.fetched_at",
            params![id, source, raster, palette],
        ).map_err(store::storage)?;
        tx.execute(
            "DELETE FROM radio_artwork_cache WHERE id IN (
                SELECT id FROM radio_artwork_cache ORDER BY fetched_at DESC, rowid DESC LIMIT -1 OFFSET 96
            )",
            [],
        ).map_err(store::storage)?;
        tx.commit().map_err(store::storage)?;
        Ok(pack(raster, palette))
    }).await.map_err(|_| RadioFault::Unavailable)?
}

fn pack(raster: Vec<u8>, palette: String) -> Vec<u8> {
    if raster.is_empty() || palette.len() != 21 {
        return Vec::new();
    }
    let mut output = palette.into_bytes();
    output.extend(raster);
    output
}
