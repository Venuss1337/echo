//! Single transactional checkpoint. Restored entries remain inert until
//! separately reauthorized by the local-file intake boundary.
use super::{Playback, queue::Queue};
use crate::library;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum CheckpointError {
    Open,
    Storage,
    Invalid,
}

#[derive(Serialize, Deserialize)]
struct Session {
    version: u8,
    queue: Queue,
    position: u64,
    volume: u8,
    muted: bool,
}

pub struct Checkpoints(Connection);

impl Checkpoints {
    pub fn open(path: &Path) -> Result<Self, CheckpointError> {
        library::open(path)
            .map(Self)
            .map_err(|_| CheckpointError::Open)
    }

    /// The owner calls this at lifecycle boundaries or on a bounded timer,
    /// independently of native position event frequency.
    pub fn save(&self, playback: &Playback) -> Result<(), CheckpointError> {
        let payload = serde_json::to_string(&Session {
            version: 1,
            queue: playback.queue.clone(),
            position: if matches!(playback.state, super::State::Ended | super::State::Error) {
                0
            } else {
                playback.position
            },
            volume: playback.volume,
            muted: playback.muted,
        })
        .map_err(|_| CheckpointError::Invalid)?;
        self.0
            .execute(
                "INSERT INTO playback_session (id, payload) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
                [payload],
            )
            .map_err(|_| CheckpointError::Storage)?;
        Ok(())
    }

    pub(super) fn heard_station(&mut self, id: &str) -> Result<(), CheckpointError> {
        let tx = self.0.transaction().map_err(|_| CheckpointError::Storage)?;
        tx.execute(
            "UPDATE radio_stations SET heard_at = unixepoch() WHERE id = ?1",
            [id],
        )
        .map_err(|_| CheckpointError::Storage)?;
        tx.execute(
            "UPDATE radio_stations SET heard_at = NULL WHERE id IN (
               SELECT id FROM radio_stations WHERE heard_at IS NOT NULL
               ORDER BY heard_at DESC, id LIMIT -1 OFFSET 100
             )",
            [],
        )
        .map_err(|_| CheckpointError::Storage)?;
        tx.commit().map_err(|_| CheckpointError::Storage)
    }

    /// Reopening restores the queue and preferences in Idle. It never opens a media path.
    pub fn restore(&self) -> Result<Playback, CheckpointError> {
        let payload: Option<String> = self
            .0
            .query_row(
                "SELECT payload FROM playback_session WHERE id=1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| CheckpointError::Storage)?;
        let Some(payload) = payload else {
            return Ok(Playback::default());
        };
        let session: Session =
            serde_json::from_str(&payload).map_err(|_| CheckpointError::Invalid)?;
        if session.version != 1 || session.volume > 100 || !session.queue.valid() {
            return Err(CheckpointError::Invalid);
        }
        Ok(Playback {
            queue: session.queue,
            position: session.position,
            volume: session.volume,
            muted: session.muted,
            ..Playback::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::queue::Repeat;
    use crate::playback::{Event, State};

    #[test]
    fn reopens_inert_queue_and_preferences_even_if_media_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.sqlite3");
        let store = Checkpoints::open(&path).unwrap();
        let missing = temp.path().join("formerly present 日本.flac");
        let mut player = Playback::default();
        let first = player.queue.append(missing.clone()).unwrap();
        let duplicate = player.queue.append(missing.clone()).unwrap();
        player.load(first).unwrap();
        player.event(
            1,
            Event::Position {
                milliseconds: 1500,
                duration: Some(5000),
                seekable: true,
            },
        );
        player.settings(Repeat::All, Some(5));
        player.mixer_confirmed(27, true);
        store.save(&player).unwrap();
        drop(store);
        let restored = Checkpoints::open(&path).unwrap().restore().unwrap();
        assert_eq!(restored.snapshot().state, State::Idle);
        assert_eq!(restored.snapshot().position, 1500);
        assert_eq!(restored.snapshot().volume, 27);
        assert!(restored.snapshot().muted);
        assert_eq!(restored.queue.current(), Some(first));
        assert_eq!(restored.queue.order(), &[first, duplicate]);
        assert_eq!(restored.queue.entries()[0].path(), missing);
        assert_eq!(restored.queue.repeat, Repeat::All);
        assert!(!missing.exists());
    }

    #[test]
    fn rejects_corrupt_checkpoint_without_resetting_other_tables() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.sqlite3");
        let store = Checkpoints::open(&path).unwrap();
        store.0.execute_batch("INSERT INTO library_roots(path) VALUES ('keep'); INSERT INTO playback_session VALUES(1, 'invalid');").unwrap();
        assert!(matches!(store.restore(), Err(CheckpointError::Invalid)));
        let saved: String = store
            .0
            .query_row("SELECT path FROM library_roots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(saved, "keep");
    }
}
