//! Transactional SQLite schema upgrades. `user_version` is the authoritative version marker.

use super::LibraryError;
use crate::domain::Track;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

const CURRENT_VERSION: i64 = 2;

pub(super) fn migrate(connection: &mut Connection, root: &Path) -> Result<i64, LibraryError> {
    let mut version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 0 {
        let has_tracks: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='tracks')",
            [],
            |row| row.get(0),
        )?;
        if has_tracks {
            let has_version_table: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
                [],
                |row| row.get(0),
            )?;
            if !has_version_table {
                return Err(LibraryError::InvalidSchema { version: 0 });
            }
            let legacy: Option<i64> = connection
                .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                    row.get(0)
                })
                .optional()?;
            if legacy != Some(1) {
                return Err(LibraryError::InvalidSchema {
                    version: legacy.unwrap_or(0),
                });
            }
            let transaction = connection.transaction()?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction.commit()?;
        } else {
            create_v1(connection)?;
        }
        version = 1;
    }
    if !(1..=CURRENT_VERSION).contains(&version) {
        return Err(LibraryError::InvalidSchema { version });
    }
    if version == 1 {
        migrate_v1_to_v2(connection, root)?;
    }
    connection.execute(
        "INSERT INTO libraries(root) VALUES (?1) ON CONFLICT(root) DO NOTHING",
        [root.to_string_lossy().as_ref()],
    )?;
    connection
        .query_row(
            "SELECT id FROM libraries WHERE root = ?1",
            [root.to_string_lossy().as_ref()],
            |row| row.get(0),
        )
        .map_err(LibraryError::from)
}

fn create_v1(connection: &mut Connection) -> Result<(), LibraryError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE schema_version (version INTEGER NOT NULL);
         INSERT INTO schema_version(version) VALUES (1);
         CREATE TABLE tracks (
           id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, size INTEGER NOT NULL,
           modified_ns TEXT NOT NULL, generation INTEGER NOT NULL, data TEXT NOT NULL
         );
         CREATE INDEX tracks_generation ON tracks(generation);",
    )?;
    transaction.pragma_update(None, "user_version", 1)?;
    transaction.commit()?;
    Ok(())
}

struct LegacyRow {
    id: i64,
    path: String,
    size: i64,
    mtime_ns: String,
    generation: i64,
    data: String,
}

fn migrate_v1_to_v2(connection: &mut Connection, root: &Path) -> Result<(), LibraryError> {
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "CREATE TABLE libraries (
           id INTEGER PRIMARY KEY, root TEXT NOT NULL UNIQUE
         );
         CREATE TABLE tracks_v2 (
           id INTEGER PRIMARY KEY,
           library_id INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
           path TEXT NOT NULL,
           format TEXT NOT NULL,
           size INTEGER NOT NULL CHECK(size >= 0),
           mtime_ns TEXT NOT NULL,
           fingerprint TEXT,
           generation INTEGER NOT NULL,
           writable INTEGER NOT NULL CHECK(writable IN (0, 1)),
           title TEXT, artist TEXT, album_artist TEXT, album TEXT,
           disc_number INTEGER, track_number INTEGER, date TEXT, release_id TEXT,
           data TEXT NOT NULL,
           UNIQUE(library_id, path)
         );
         CREATE TABLE track_genres (
           track_id INTEGER NOT NULL REFERENCES tracks_v2(id) ON DELETE CASCADE,
           genre TEXT NOT NULL COLLATE NOCASE,
           PRIMARY KEY(track_id, genre)
         );",
    )?;
    transaction.execute(
        "INSERT INTO libraries(root) VALUES (?1)",
        [root.to_string_lossy().as_ref()],
    )?;
    let library_id = transaction.last_insert_rowid();
    let mut statement = transaction
        .prepare("SELECT id,path,size,modified_ns,generation,data FROM tracks ORDER BY id")?;
    let rows = statement.query_map([], |row| {
        Ok(LegacyRow {
            id: row.get(0)?,
            path: row.get(1)?,
            size: row.get(2)?,
            mtime_ns: row.get(3)?,
            generation: row.get(4)?,
            data: row.get(5)?,
        })
    })?;
    for row in rows {
        let row = row?;
        let track: Track = serde_json::from_str(&row.data)
            .map_err(|source| LibraryError::InvalidLegacyTrack { id: row.id, source })?;
        let mtime: u128 = row
            .mtime_ns
            .parse()
            .map_err(|_| LibraryError::InvalidLegacyTime { id: row.id })?;
        transaction.execute(
            "INSERT INTO tracks_v2 (
               id,library_id,path,format,size,mtime_ns,fingerprint,generation,writable,
               title,artist,album_artist,album,disc_number,track_number,date,release_id,data
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![
                row.id,
                library_id,
                row.path,
                track.format.as_str(),
                row.size,
                padded_time(mtime),
                track.snapshot.sha256,
                row.generation,
                track.writable as i64,
                track.metadata.title,
                track.metadata.artist,
                track.metadata.album_artist,
                track.metadata.album,
                track.metadata.disc.as_ref().map(|pair| pair.number),
                track.metadata.track.as_ref().map(|pair| pair.number),
                track.metadata.date,
                track.metadata.release_id,
                row.data,
            ],
        )?;
        for genre in track.metadata.genres {
            transaction.execute(
                "INSERT OR IGNORE INTO track_genres(track_id, genre) VALUES (?1, ?2)",
                params![row.id, genre],
            )?;
        }
    }
    drop(statement);
    transaction.execute_batch(
        "DROP TABLE tracks;
         ALTER TABLE tracks_v2 RENAME TO tracks;
         CREATE INDEX tracks_generation ON tracks(library_id, generation);
         CREATE INDEX tracks_grouping ON tracks(
           library_id,
           COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), '<unknown>') COLLATE NOCASE,
           album COLLATE NOCASE, disc_number, track_number, title COLLATE NOCASE
         );
         CREATE INDEX tracks_album ON tracks(library_id, album COLLATE NOCASE);
         CREATE INDEX tracks_title ON tracks(library_id, title COLLATE NOCASE);
         CREATE INDEX tracks_format_writable ON tracks(library_id, format, writable);
         CREATE INDEX track_genres_lookup ON track_genres(genre, track_id);
         UPDATE schema_version SET version = 2;",
    )?;
    transaction.pragma_update(None, "user_version", CURRENT_VERSION)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn padded_time(value: u128) -> String {
    format!("{value:039}")
}
