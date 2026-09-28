use crate::domain::{Track, TrackId};
use crate::tags;
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct Index {
    connection: Connection,
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
    pub fn open(root: &Path) -> Result<Self> {
        let path = data_dir()?.join(format!("{}.sqlite", root_key(root)));
        Self::open_path(&path)
    }

    fn open_path(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);\
            INSERT INTO schema_version(version) SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version);\
            CREATE TABLE IF NOT EXISTS tracks (\
              id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, size INTEGER NOT NULL,\
              modified_ns TEXT NOT NULL, generation INTEGER NOT NULL, data TEXT NOT NULL\
            );\
            CREATE INDEX IF NOT EXISTS tracks_generation ON tracks(generation);")?;
        Ok(Self { connection })
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
            "SELECT COALESCE(MAX(generation), 0) + 1 FROM tracks",
            [],
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
            let previous: Option<(i64, String)> = self
                .connection
                .query_row(
                    "SELECT size, modified_ns FROM tracks WHERE path = ?1",
                    [&path_text],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if previous.as_ref().is_some_and(|(size, time)| {
                *size == current.size as i64 && *time == current.modified_ns.to_string()
            }) {
                self.connection.execute(
                    "UPDATE tracks SET generation = ?1 WHERE path = ?2",
                    params![generation, path_text],
                )?;
                report.reused += 1;
                report.tracks += 1;
                let data: String = self.connection.query_row(
                    "SELECT data FROM tracks WHERE path = ?1",
                    [&path_text],
                    |row| row.get(0),
                )?;
                let mut track: Track = serde_json::from_str(&data)?;
                let id: i64 = self.connection.query_row(
                    "SELECT id FROM tracks WHERE path = ?1",
                    [&path_text],
                    |row| row.get(0),
                )?;
                track.id = TrackId::indexed(id).context("invalid indexed track ID")?;
                on_track(track);
                continue;
            }
            match tags::read_track(path) {
                Ok(track) => {
                    self.connection.execute(
                        "INSERT INTO tracks(path,size,modified_ns,generation,data) VALUES (?1,?2,?3,?4,?5) \
                         ON CONFLICT(path) DO UPDATE SET size=excluded.size, modified_ns=excluded.modified_ns, \
                         generation=excluded.generation, data=excluded.data",
                        params![path_text, current.size as i64, current.modified_ns.to_string(), generation, serde_json::to_string(&track)?],
                    )?;
                    report.tracks += 1;
                    let id: i64 = self.connection.query_row(
                        "SELECT id FROM tracks WHERE path = ?1",
                        [&path_text],
                        |row| row.get(0),
                    )?;
                    let mut track = track;
                    track.id = TrackId::indexed(id).context("invalid indexed track ID")?;
                    on_track(track);
                }
                Err(err) => report.errors.push(format!("{}: {err:#}", path.display())),
            }
        }
        // Preserve prior rows when cancelled or when traversal had errors; otherwise remove vanished files.
        if report.errors.is_empty() && !report.cancelled {
            self.connection
                .execute("DELETE FROM tracks WHERE generation != ?1", [generation])?;
        }
        Ok(report)
    }

    pub fn tracks(&self) -> Result<Vec<Track>> {
        let mut stmt = self
            .connection
            .prepare("SELECT id,data FROM tracks ORDER BY path")?;
        let rows = stmt.query_map([], |row| {
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
            "SELECT COALESCE(MAX(generation), 0) FROM tracks",
            [],
            |row| row.get(0),
        )?;
        self.connection.execute(
            "INSERT INTO tracks(path,size,modified_ns,generation,data) VALUES (?1,?2,?3,?4,?5) \
             ON CONFLICT(path) DO UPDATE SET size=excluded.size, modified_ns=excluded.modified_ns, \
             generation=excluded.generation, data=excluded.data",
            params![
                path.to_string_lossy().to_string(),
                track.snapshot.size as i64,
                track.snapshot.modified_ns.to_string(),
                generation,
                serde_json::to_string(&track)?
            ],
        )?;
        Ok(())
    }
}

use rusqlite::OptionalExtension;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_emits_tracks_during_background_load() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
            temp.path().join("one.mp3"),
        )
        .expect("fixture");
        let mut index = Index::open_path(&temp.path().join("index.sqlite")).expect("index");
        let mut arrived = Vec::new();
        index
            .scan_with_updates(
                temp.path(),
                |_, _| true,
                |track| arrived.push(track.snapshot.path),
            )
            .expect("scan");
        assert_eq!(arrived, vec![temp.path().join("one.mp3")]);
    }

    #[test]
    fn repeated_scan_preserves_indexed_track_identity() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3"),
            temp.path().join("one.mp3"),
        )
        .expect("fixture");
        let mut index = Index::open_path(&temp.path().join("index.sqlite")).expect("index");
        let first = index.scan(temp.path(), |_, _| true).expect("initial scan");
        assert_eq!(first.reused, 0);
        let original = index.tracks().expect("indexed tracks")[0].id;
        assert!(original.get() > 0);

        let second = index.scan(temp.path(), |_, _| true).expect("repeat scan");
        assert_eq!(second.reused, 1);
        assert_eq!(index.tracks().expect("reused tracks")[0].id, original);
    }

    #[test]
    fn scan_skips_quarantined_subtree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/problem.mp3");
        std::fs::copy(&fixture, temp.path().join("live.mp3")).expect("copy live");
        let hidden = temp.path().join(crate::quarantine::FOLDER).join("entry");
        std::fs::create_dir_all(&hidden).expect("mkdir");
        std::fs::copy(&fixture, hidden.join("hidden.mp3")).expect("copy hidden");
        let mut index = Index::open_path(&temp.path().join("index.sqlite")).expect("index");
        let report = index.scan(temp.path(), |_, _| true).expect("scan");
        assert_eq!(report.tracks, 1);
        assert_eq!(index.tracks().expect("tracks").len(), 1);
    }
}
