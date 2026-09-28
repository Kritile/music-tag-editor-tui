mod migrations;
mod query;

pub use query::{TrackQuery, TrackSummary};

use crate::domain::{Track, TrackId};
use crate::tags;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct Index {
    connection: Connection,
    library_id: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("cannot open library index: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("cannot access application data directory: {0:#}")]
    DataDirectory(#[from] anyhow::Error),
    #[error("unsupported or inconsistent library database schema version {version}")]
    InvalidSchema { version: i64 },
    #[error("legacy track {id} has invalid JSON: {source}")]
    InvalidLegacyTrack { id: i64, source: serde_json::Error },
    #[error("legacy track {id} has invalid modification time")]
    InvalidLegacyTime { id: i64 },
}

#[derive(Default, Debug)]
pub struct ScanReport {
    pub tracks: usize,
    pub reused: usize,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

pub fn data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "music-tag-editor", "music-tag-editor")
        .context("cannot determine application data directory")?;
    let path = dirs.data_local_dir().to_path_buf();
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn root_key(root: &Path) -> String {
    let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
    format!("{:x}", digest)[..16].to_string()
}

impl Index {
    pub fn open(root: &Path) -> std::result::Result<Self, LibraryError> {
        let path = data_dir()?.join(format!("{}.sqlite", root_key(root)));
        Self::open_at(&path, root)
    }

    #[cfg(test)]
    fn open_path(path: &Path) -> std::result::Result<Self, LibraryError> {
        Self::open_at(path, path.parent().unwrap_or(Path::new(".")))
    }

    fn open_at(path: &Path, root: &Path) -> std::result::Result<Self, LibraryError> {
        let mut connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let library_id = migrations::migrate(&mut connection, root)?;
        Ok(Self {
            connection,
            library_id,
        })
    }

    pub fn scan(
        &mut self,
        root: &Path,
        progress: impl FnMut(usize, &Path) -> bool,
    ) -> Result<ScanReport> {
        self.scan_with_updates(root, progress, |_| {})
    }

    pub fn scan_with_updates(
        &mut self,
        root: &Path,
        mut progress: impl FnMut(usize, &Path) -> bool,
        mut on_track: impl FnMut(Track),
    ) -> Result<ScanReport> {
        let generation: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(generation), 0) + 1 FROM tracks WHERE library_id = ?1",
            [self.library_id],
            |row| row.get(0),
        )?;
        let mut report = ScanReport::default();
        let quarantine = root.join(crate::quarantine::FOLDER);
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| entry.path() != quarantine)
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    report.errors.push(err.to_string());
                    continue;
                }
            };
            if !entry.file_type().is_file()
                || entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".music-tui-")
                || tags::format(entry.path()).is_none()
            {
                continue;
            }
            if !progress(report.tracks + report.errors.len(), entry.path()) {
                report.cancelled = true;
                break;
            }
            let path = entry.path();
            let path_text = path.to_string_lossy().to_string();
            let current = match tags::snapshot(path, false) {
                Ok(value) => value,
                Err(err) => {
                    report.errors.push(err.to_string());
                    continue;
                }
            };
            let previous: Option<(i64, String, i64)> = self
                .connection
                .query_row(
                    "SELECT size, mtime_ns, id FROM tracks WHERE library_id = ?1 AND path = ?2",
                    params![self.library_id, path_text],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            if previous.as_ref().is_some_and(|(size, time, _)| {
                *size == current.size as i64
                    && *time == migrations::padded_time(current.modified_ns)
            }) {
                let (_, _, id) = previous.context("missing cached track")?;
                self.connection.execute(
                    "UPDATE tracks SET generation = ?1 WHERE id = ?2",
                    params![generation, id],
                )?;
                report.reused += 1;
                report.tracks += 1;
                let data: String = self.connection.query_row(
                    "SELECT data FROM tracks WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )?;
                let mut track: Track = serde_json::from_str(&data)?;
                track.id = TrackId::indexed(id).context("invalid indexed track ID")?;
                on_track(track);
                continue;
            }
            match tags::read_track(path) {
                Ok(track) => {
                    let id = self.upsert_track(&track, generation)?;
                    report.tracks += 1;
                    let mut track = track;
                    track.id = TrackId::indexed(id).context("invalid indexed track ID")?;
                    on_track(track);
                }
                Err(err) => report.errors.push(format!("{}: {err:#}", path.display())),
            }
        }
        // Preserve prior rows when cancelled or when traversal had errors; otherwise remove vanished files.
        if report.errors.is_empty() && !report.cancelled {
            self.connection.execute(
                "DELETE FROM tracks WHERE library_id = ?1 AND generation != ?2",
                params![self.library_id, generation],
            )?;
        }
        Ok(report)
    }

    pub fn tracks(&self) -> Result<Vec<Track>> {
        let mut stmt = self
            .connection
            .prepare("SELECT id,data FROM tracks WHERE library_id = ?1 ORDER BY path")?;
        let rows = stmt.query_map([self.library_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (id, data) = row?;
            let mut track: Track = serde_json::from_str(&data)?;
            track.id = TrackId::indexed(id).context("invalid indexed track ID")?;
            Ok(track)
        })
        .collect()
    }

    pub fn refresh(&mut self, path: &Path) -> Result<()> {
        let track = tags::read_track(path)?;
        let generation: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(generation), 0) FROM tracks WHERE library_id = ?1",
            [self.library_id],
            |row| row.get(0),
        )?;
        self.upsert_track(&track, generation)?;
        Ok(())
    }

    fn upsert_track(&mut self, track: &Track, generation: i64) -> Result<i64> {
        let size = i64::try_from(track.snapshot.size).context("file size exceeds SQLite range")?;
        let data = serde_json::to_string(track)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO tracks (
               library_id,path,format,size,mtime_ns,fingerprint,generation,writable,
               title,artist,album_artist,album,disc_number,track_number,date,release_id,data
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
             ON CONFLICT(library_id,path) DO UPDATE SET
               format=excluded.format,size=excluded.size,mtime_ns=excluded.mtime_ns,
               fingerprint=excluded.fingerprint,generation=excluded.generation,
               writable=excluded.writable,title=excluded.title,artist=excluded.artist,
               album_artist=excluded.album_artist,album=excluded.album,
               disc_number=excluded.disc_number,track_number=excluded.track_number,
               date=excluded.date,release_id=excluded.release_id,data=excluded.data",
            params![
                self.library_id,
                track.snapshot.path.to_string_lossy(),
                track.format.as_str(),
                size,
                migrations::padded_time(track.snapshot.modified_ns),
                track.snapshot.sha256,
                generation,
                track.writable as i64,
                track.metadata.title,
                track.metadata.artist,
                track.metadata.album_artist,
                track.metadata.album,
                track.metadata.disc.as_ref().map(|pair| pair.number),
                track.metadata.track.as_ref().map(|pair| pair.number),
                track.metadata.date,
                track.metadata.release_id,
                data,
            ],
        )?;
        let id: i64 = transaction.query_row(
            "SELECT id FROM tracks WHERE library_id = ?1 AND path = ?2",
            params![self.library_id, track.snapshot.path.to_string_lossy()],
            |row| row.get(0),
        )?;
        transaction.execute("DELETE FROM track_genres WHERE track_id = ?1", [id])?;
        for genre in &track.metadata.genres {
            transaction.execute(
                "INSERT OR IGNORE INTO track_genres(track_id,genre) VALUES (?1,?2)",
                params![id, genre],
            )?;
        }
        transaction.commit()?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests;
