//! App-owned SQLite bootstrap. Migrations and the schema version advance atomically.
//!
//! The submodules split the library by responsibility so each can be tested
//! from the command line without a window: `text` normalizes names, `meta` and
//! `artwork` read untrusted files, `index` is the only writer of library rows,
//! `query` serves paginated reads, and the Windows-only `scan` and `service`
//! walk the filesystem and coordinate background work.

pub(crate) mod artwork;
#[cfg(all(test, windows))]
mod bench;
#[cfg(windows)]
pub(crate) mod desktop;
pub(crate) mod index;
pub(crate) mod meta;
#[cfg(all(test, windows))]
mod native_tests;
pub(crate) mod palette;
pub(crate) mod query;
#[cfg(windows)]
pub(crate) mod scan;
#[cfg(windows)]
pub(crate) mod service;
pub(crate) mod text;
pub(crate) mod user;
#[cfg(windows)]
pub(crate) mod watch;

use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::{fs, path::Path, time::Duration};

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_library_roots.sql"),
    include_str!("../migrations/002_playback_session.sql"),
    include_str!("../migrations/003_library_index.sql"),
    include_str!("../migrations/004_user_data.sql"),
    include_str!("../migrations/005_radio.sql"),
];

#[derive(Debug, PartialEq)]
pub(super) enum LibraryError {
    Directory,
    UnsupportedVersion,
    Sqlite(rusqlite::ErrorCode),
    InvalidSchema,
}

impl From<rusqlite::Error> for LibraryError {
    fn from(error: rusqlite::Error) -> Self {
        error
            .sqlite_error_code()
            .map_or(Self::InvalidSchema, Self::Sqlite)
    }
}

pub(super) fn initialize(directory: &Path) -> Result<(), LibraryError> {
    fs::create_dir_all(directory).map_err(|_| LibraryError::Directory)?;
    open(&directory.join("library.sqlite3")).map(drop)
}

/// Opens the read-write connection and applies pending migrations. WAL lets
/// scan transactions and playback checkpoints proceed while readers query.
pub(crate) fn open(path: &Path) -> Result<Connection, LibraryError> {
    let mut db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(5))?;
    // journal_mode returns a row, and cannot change inside a transaction.
    db.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
    db.pragma_update(None, "synchronous", "NORMAL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&mut db, MIGRATIONS)?;
    Ok(db)
}

/// A migrated database opened for queries only. It cannot write even if a
/// query bug tried to, and never contends for the migration lock.
pub(crate) fn open_reader(path: &Path) -> Result<Connection, LibraryError> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.pragma_update(None, "query_only", "ON")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    Ok(db)
}

fn migrate(db: &mut Connection, migrations: &[&str]) -> Result<(), LibraryError> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let installed = usize::try_from(version).map_err(|_| LibraryError::UnsupportedVersion)?;
    if installed > migrations.len() {
        return Err(LibraryError::UnsupportedVersion);
    }
    for (index, sql) in migrations.iter().enumerate().skip(installed) {
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
    }
    // Preparing against the installed schema detects an incorrect version marker
    // or a missing column without mutating the existing data.
    tx.prepare("SELECT id, path FROM library_roots LIMIT 0")?;
    if migrations.len() >= 2 {
        tx.prepare("SELECT id, payload FROM playback_session LIMIT 0")?;
    }
    if migrations.len() >= 3 {
        tx.prepare("SELECT id, root_id, path_key, size, mtime_ns, state FROM tracks LIMIT 0")?;
        tx.prepare("SELECT rowid, title FROM search_tracks LIMIT 0")?;
    }
    if migrations.len() >= 4 {
        tx.prepare("SELECT id, track_id, abs_key FROM media_sources LIMIT 0")?;
        tx.prepare("SELECT id, position FROM playlist_entries LIMIT 0")?;
        tx.prepare("SELECT source_id, listened_ms FROM plays LIMIT 0")?;
        tx.prepare("SELECT palette FROM artwork LIMIT 0")?;
    }
    if migrations.len() >= 5 {
        tx.prepare("SELECT id, stream_url, favorite, heard_at FROM radio_stations LIMIT 0")?;
        tx.prepare("SELECT query, response, fetched_at FROM radio_directory_cache LIMIT 0")?;
        tx.prepare("SELECT id, raster, palette FROM radio_artwork_cache LIMIT 0")?;
        tx.prepare("SELECT id, sort FROM radio_preferences LIMIT 0")?;
    }
    tx.commit()?;
    Ok(())
}

/// A database exactly as an earlier release left it.
#[cfg(test)]
pub(crate) fn create_at_version(path: &Path, version: usize) {
    let mut db = Connection::open(path).unwrap();
    migrate(&mut db, &MIGRATIONS[..version]).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_new_database_and_preserves_rows_on_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.sqlite3");
        let db = open(&path).unwrap();
        db.execute(
            "INSERT INTO library_roots (path) VALUES (?1)",
            ["test-root"],
        )
        .unwrap();
        drop(db);

        let db = open(&path).unwrap();
        assert_eq!(
            db.query_row("SELECT path FROM library_roots", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "test-root"
        );
    }

    #[test]
    fn upgrading_a_version_two_database_preserves_roots_and_the_playback_session() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("v2.sqlite3");
        {
            // Build exactly what the previous release left on disk.
            let mut old = Connection::open(&path).unwrap();
            migrate(&mut old, &MIGRATIONS[..2]).unwrap();
            old.execute(
                "INSERT INTO library_roots (path) VALUES (?1)",
                [r"D:\Music"],
            )
            .unwrap();
            old.execute(
                "INSERT INTO playback_session (id, payload) VALUES (1, '{\"saved\":true}')",
                [],
            )
            .unwrap();
        }
        let db = open(&path).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            5
        );
        let (root, online, added): (String, i64, i64) = db
            .query_row(
                "SELECT path, online, added_at FROM library_roots",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((root.as_str(), online, added), (r"D:\Music", 1, 0));
        let session: String = db
            .query_row("SELECT payload FROM playback_session", [], |row| row.get(0))
            .unwrap();
        assert_eq!(session, "{\"saved\":true}");
        // The new tables exist, are empty and enforce their constraints.
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM tracks", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            db.execute(
                "INSERT INTO tracks (root_id, rel_path, path_key, size, mtime_ns, title, title_key, metadata_version)
                 VALUES (999, 'a', 'a', 1, 1, 't', 't', 1)",
                [],
            )
            .is_err(),
            "foreign keys are enforced"
        );
    }

    #[test]
    fn readers_cannot_write_and_see_committed_data_while_a_writer_is_open() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rw.sqlite3");
        let writer = open(&path).unwrap();
        writer
            .execute("INSERT INTO library_roots (path) VALUES ('x')", [])
            .unwrap();
        let reader = open_reader(&path).unwrap();
        assert!(
            reader
                .execute("INSERT INTO library_roots (path) VALUES ('y')", [])
                .is_err()
        );
        let uncommitted = writer.unchecked_transaction().unwrap();
        uncommitted
            .execute("INSERT INTO library_roots (path) VALUES ('pending')", [])
            .unwrap();
        // WAL readers see the last commit, not the open transaction.
        assert_eq!(
            reader
                .query_row("SELECT COUNT(*) FROM library_roots", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn refuses_database_from_a_newer_release() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.sqlite3");
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", 999).unwrap();
        assert!(open(&path).is_err());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            999
        );
    }

    #[test]
    fn a_conflicting_migration_preserves_existing_data_and_version() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.sqlite3");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE library_roots (path TEXT); INSERT INTO library_roots VALUES ('keep');",
        )
        .unwrap();
        assert!(open(&path).is_err());
        let stored: String = db
            .query_row("SELECT path FROM library_roots", [], |row| row.get(0))
            .unwrap();
        assert_eq!(stored, "keep");
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn refuses_a_versioned_database_missing_its_schema() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.sqlite3");
        Connection::open(&path)
            .unwrap()
            .pragma_update(None, "user_version", 1)
            .unwrap();
        assert!(open(&path).is_err());
    }

    #[test]
    fn refuses_corruption_without_replacing_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("corrupt.sqlite3");
        let original = b"an existing file that is not SQLite";
        fs::write(&path, original).unwrap();
        assert!(open(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn rolls_back_applied_statements_when_a_later_migration_fails() {
        let mut db = Connection::open_in_memory().unwrap();
        assert!(migrate(&mut db, &[MIGRATIONS[0], MIGRATIONS[0]]).is_err());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(db.prepare("SELECT id FROM library_roots").is_err());
        migrate(&mut db, MIGRATIONS).unwrap();
    }
}
