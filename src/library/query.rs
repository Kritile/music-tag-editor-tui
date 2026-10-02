//! Indexed SQL projections for browser queries. Raw tag JSON stays in the index for full reads.

use super::Index;
use crate::domain::{AudioFormat, TrackId};
use anyhow::{Context, Result, bail};
use rusqlite::{params_from_iter, types::Value};
use std::path::PathBuf;

/// A bounded result set and the number of rows matching its filters.
#[derive(Clone, Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

/// Stable ordering for successive pages. Every order has a unique ID tie breaker.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackSort {
    #[default]
    Album,
    Title,
    Artist,
    Path,
    ModifiedNewest,
}

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

/// Filters and paging options for library browsing.
#[derive(Clone, Debug)]
pub struct TrackQuery<'a> {
    /// Case-insensitive substring in title, artist, album artist, album, or path.
    pub search: Option<&'a str>,
    pub format: Option<AudioFormat>,
    pub writable: Option<bool>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub title_prefix: Option<&'a str>,
    /// Filters per-track tag and Echo Mini issues. Cohort issues are excluded.
    pub issues: Option<bool>,
    pub sort: TrackSort,
    pub limit: usize,
    pub offset: usize,
}

impl Default for TrackQuery<'_> {
    fn default() -> Self {
        Self {
            search: None,
            format: None,
            writable: None,
            artist: None,
            album: None,
            genre: None,
            title_prefix: None,
            issues: None,
            sort: TrackSort::default(),
            limit: 100,
            offset: 0,
        }
    }
}

impl Index {
    /// Returns a page of SQL projections without deserializing full tracks.
    pub fn query_tracks(&self, query: &TrackQuery<'_>) -> Result<Page<TrackSummary>> {
        let mut filter = String::from(" FROM tracks WHERE library_id = ?");
        let mut values = vec![Value::Integer(self.library_id)];
        if let Some(format) = query.format {
            filter.push_str(" AND format = ?");
            values.push(Value::Text(format.as_str().to_owned()));
        }
        if let Some(writable) = query.writable {
            filter.push_str(" AND writable = ?");
            values.push(Value::Integer(writable as i64));
        }
        if let Some(artist) = query.artist {
            filter.push_str(" AND COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), '<unknown>') = ? COLLATE NOCASE");
            values.push(Value::Text(artist.to_owned()));
        }
        if let Some(album) = query.album {
            filter.push_str(" AND album = ? COLLATE NOCASE");
            values.push(Value::Text(album.to_owned()));
        }
        if let Some(genre) = query.genre {
            filter.push_str(" AND EXISTS (SELECT 1 FROM track_genres WHERE track_id = tracks.id AND genre = ? COLLATE NOCASE)");
            values.push(Value::Text(genre.to_owned()));
        }
        if let Some(prefix) = query.title_prefix {
            filter.push_str(" AND title LIKE ? ESCAPE '\\'");
            let escaped = escape_like(prefix);
            values.push(Value::Text(format!("{escaped}%")));
        }
        if let Some(search) = query.search.filter(|search| !search.is_empty()) {
            filter.push_str(" AND (title LIKE ? ESCAPE '\\' OR artist LIKE ? ESCAPE '\\' OR album_artist LIKE ? ESCAPE '\\' OR album LIKE ? ESCAPE '\\' OR path LIKE ? ESCAPE '\\')");
            let pattern = Value::Text(format!("%{}%", escape_like(search)));
            values.extend(std::iter::repeat_n(pattern, 5));
        }
        if let Some(has_issues) = query.issues {
            // Per-track rules use only this row. Cohort rules from rules::inspect
            // require a separate library-wide analysis and are not indexed here.
            filter.push_str(if has_issues { " AND " } else { " AND NOT " });
            filter.push_str("(json_array_length(json_extract(data, '$.diagnostics')) > 0
                OR (album IS NOT NULL AND album_artist IS NULL)
                OR json_array_length(json_extract(data, '$.metadata.artists')) > 1
                OR (artist IS NOT NULL AND json_array_length(json_extract(data, '$.metadata.artists')) > 0
                    AND NOT EXISTS (SELECT 1 FROM json_each(data, '$.metadata.artists') WHERE value = tracks.artist))
                OR json_extract(data, '$.metadata.track.total') IS NOT NULL
                OR json_extract(data, '$.metadata.disc.total') IS NOT NULL
                OR json_extract(data, '$.metadata.artwork_count') > 0
                OR writable = 0)");
        }
        let total: i64 = self.connection.query_row(
            &format!("SELECT COUNT(*){filter}"),
            params_from_iter(values.iter()),
            |row| row.get(0),
        )?;
        let mut sql = format!(
            "SELECT id,path,format,size,mtime_ns,fingerprint,writable,title,artist,
                    album_artist,album,disc_number,track_number{filter}"
        );
        sql.push_str(match query.sort {
            TrackSort::Album => " ORDER BY COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), '<unknown>') COLLATE NOCASE, album COLLATE NOCASE, disc_number IS NULL, disc_number, track_number IS NULL, track_number, title COLLATE NOCASE, path, id",
            TrackSort::Title => " ORDER BY title COLLATE NOCASE, path, id",
            TrackSort::Artist => " ORDER BY artist COLLATE NOCASE, album COLLATE NOCASE, title COLLATE NOCASE, path, id",
            TrackSort::Path => " ORDER BY path, id",
            TrackSort::ModifiedNewest => " ORDER BY mtime_ns DESC, id",
        });
        sql.push_str(" LIMIT ? OFFSET ?");
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
        let items = rows
            .map(|row| {
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
            .collect::<Result<Vec<_>>>()?;
        Ok(Page {
            items,
            total: usize::try_from(total)?,
            offset: query.offset,
            limit: query.limit,
        })
    }
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
