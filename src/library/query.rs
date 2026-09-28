//! Indexed SQL projections for browser queries. Raw tag JSON stays in the index for full reads.

use super::Index;
use crate::domain::{AudioFormat, TrackId};
use anyhow::{Context, Result, bail};
use rusqlite::{params_from_iter, types::Value};
use std::path::PathBuf;

/// SQL projection used for browsing without decoding each track's raw metadata JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackSummary {
    pub id: TrackId,
    pub path: PathBuf,
    pub format: AudioFormat,
    pub size: u64,
    pub modified_ns: u128,
    pub fingerprint: Option<String>,
    pub writable: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub disc_number: Option<u32>,
    pub track_number: Option<u32>,
}

/// Indexed filters for library browsing. Title matching is a case-insensitive prefix.
#[derive(Clone, Debug)]
pub struct TrackQuery<'a> {
    pub format: Option<AudioFormat>,
    pub writable: Option<bool>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub title_prefix: Option<&'a str>,
    pub limit: usize,
    pub offset: usize,
}

impl Default for TrackQuery<'_> {
    fn default() -> Self {
        Self {
            format: None,
            writable: None,
            artist: None,
            album: None,
            genre: None,
            title_prefix: None,
            limit: 100,
            offset: 0,
        }
    }
}

impl Index {
    /// Returns a page of SQL projections. This never deserializes the raw tag JSON.
    pub fn query_tracks(&self, query: &TrackQuery<'_>) -> Result<Vec<TrackSummary>> {
        let mut sql = String::from(
            "SELECT id,path,format,size,mtime_ns,fingerprint,writable,title,artist,
                    album_artist,album,disc_number,track_number
             FROM tracks WHERE library_id = ?",
        );
        let mut values = vec![Value::Integer(self.library_id)];
        if let Some(format) = query.format {
            sql.push_str(" AND format = ?");
            values.push(Value::Text(format.as_str().to_owned()));
        }
        if let Some(writable) = query.writable {
            sql.push_str(" AND writable = ?");
            values.push(Value::Integer(writable as i64));
        }
        if let Some(artist) = query.artist {
            sql.push_str(" AND COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), '<unknown>') = ? COLLATE NOCASE");
            values.push(Value::Text(artist.to_owned()));
        }
        if let Some(album) = query.album {
            sql.push_str(" AND album = ? COLLATE NOCASE");
            values.push(Value::Text(album.to_owned()));
        }
        if let Some(genre) = query.genre {
            sql.push_str(" AND EXISTS (SELECT 1 FROM track_genres WHERE track_id = tracks.id AND genre = ? COLLATE NOCASE)");
            values.push(Value::Text(genre.to_owned()));
        }
        if let Some(prefix) = query.title_prefix {
            sql.push_str(" AND title LIKE ? ESCAPE '\\'");
            let escaped = prefix
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            values.push(Value::Text(format!("{escaped}%")));
        }
        sql.push_str(
            " ORDER BY COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), '<unknown>') COLLATE NOCASE,
             album COLLATE NOCASE, disc_number IS NULL, disc_number,
             track_number IS NULL, track_number, title COLLATE NOCASE, path
             LIMIT ? OFFSET ?",
        );
        values.push(Value::Integer(i64::try_from(query.limit)?));
        values.push(Value::Integer(i64::try_from(query.offset)?));
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(values), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, bool>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<u32>>(11)?,
                row.get::<_, Option<u32>>(12)?,
            ))
        })?;
        rows.map(|row| {
            let (
                id,
                path,
                format,
                size,
                mtime,
                fingerprint,
                writable,
                title,
                artist,
                album_artist,
                album,
                disc_number,
                track_number,
            ) = row?;
            let id = TrackId::indexed(id).context("invalid indexed track ID")?;
            let format = match format.as_str() {
                "MP3" => AudioFormat::Mp3,
                "FLAC" => AudioFormat::Flac,
                "MP4" => AudioFormat::Mp4,
                "Ogg" => AudioFormat::OggVorbis,
                "Opus" => AudioFormat::Opus,
                _ => bail!("unknown indexed audio format {format}"),
            };
            Ok(TrackSummary {
                id,
                path: PathBuf::from(path),
                format,
                size: u64::try_from(size)?,
                modified_ns: mtime.parse()?,
                fingerprint,
                writable,
                title,
                artist,
                album_artist,
                album,
                disc_number,
                track_number,
            })
        })
        .collect()
    }
}
