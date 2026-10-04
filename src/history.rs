//! Watch history: how far each movie and episode was watched, for resume,
//! progress bars and watched marks.
//!
//! Kept in SQLite in the data directory, not the cache, so clearing the
//! cache keeps it. Titles are keyed by provider, kind and the provider's
//! id; episodes also record their series, for "Resume S2 E4".

use std::collections::{HashMap, HashSet};

use camino::Utf8Path;
use rusqlite::{Connection, OptionalExtension, Row, params};

/// Past this fraction of its length a title counts as watched.
pub const WATCHED: f64 = 0.95;
/// Positions before this many seconds are not worth resuming.
const MIN_RESUME: f64 = 30.0;

/// Tables and indexes.
const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS progress (
        provider TEXT NOT NULL,
        kind INTEGER NOT NULL,
        id TEXT NOT NULL,
        series INTEGER,
        title TEXT NOT NULL,
        position REAL NOT NULL,
        duration REAL NOT NULL,
        watched INTEGER NOT NULL,
        updated INTEGER NOT NULL,
        PRIMARY KEY (provider, kind, id)
    );
    CREATE INDEX IF NOT EXISTS progress_by_series ON progress (provider, series, updated);
";

/// The watch history store.
pub struct History {
    conn: Connection,
}

/// What kind of title an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A movie, by stream id.
    Movie,
    /// An episode, by episode id.
    Episode,
}

/// A title being watched.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// The provider's id.
    pub provider: String,
    /// Movie or episode.
    pub kind: Kind,
    /// The provider's id for the title.
    pub id: String,
    /// For an episode, its series' id.
    pub series: Option<u64>,
    /// Display name, for lists of what was watched.
    pub title: String,
}

/// How far a title was watched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// Seconds in.
    pub position: f64,
    /// Length in seconds; 0 when unknown.
    pub duration: f64,
    /// Whether it was watched to the end.
    pub watched: bool,
    /// When it was last watched, as Unix seconds.
    pub updated: i64,
}

// Public API
impl History {
    /// Opens (or creates) the store at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be opened or migrated.
    pub fn open(path: &Utf8Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            // A missing directory surfaces as the open error below.
            let _ = std::fs::create_dir_all(dir);
        }
        Self::with(Connection::open(path)?)
    }

    /// An empty store in memory, for tests.
    ///
    /// # Errors
    ///
    /// Returns an error when SQLite cannot create it.
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::with(Connection::open_in_memory()?)
    }

    /// Records that `entry` is at `position` of `duration` seconds.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written.
    pub fn save(
        &self,
        entry: &Entry,
        position: f64,
        duration: f64,
        now: i64,
    ) -> rusqlite::Result<()> {
        let watched = duration > 0.0 && position >= duration * WATCHED;
        self.conn.execute(
            "INSERT INTO progress (provider, kind, id, series, title, position, duration, watched, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (provider, kind, id) DO UPDATE SET
                series = excluded.series, title = excluded.title, position = excluded.position,
                duration = excluded.duration, watched = excluded.watched, updated = excluded.updated",
            params![
                entry.provider,
                kind_code(entry.kind),
                entry.id,
                entry.series.and_then(|s| i64::try_from(s).ok()),
                entry.title,
                position,
                duration,
                watched,
                now
            ],
        )?;
        Ok(())
    }

    /// How far `provider`'s title `id` of `kind` was watched.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn progress(
        &self,
        provider: &str,
        kind: Kind,
        id: &str,
    ) -> rusqlite::Result<Option<Progress>> {
        self.conn
            .query_row(
                "SELECT position, duration, watched, updated FROM progress
                 WHERE provider = ?1 AND kind = ?2 AND id = ?3",
                params![provider, kind_code(kind), id],
                progress,
            )
            .optional()
    }

    /// Every title of `kind` watched from `provider`, by id.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn all(&self, provider: &str, kind: Kind) -> rusqlite::Result<HashMap<String, Progress>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT id, position, duration, watched, updated FROM progress
             WHERE provider = ?1 AND kind = ?2",
        )?;
        let rows = statement.query_map(params![provider, kind_code(kind)], |row| {
            Ok((row.get::<_, String>(0)?, progress_at(row, 1)?))
        })?;
        rows.collect()
    }

    /// The episodes of `provider`'s `series` watched, by episode id, and
    /// the one watched last.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn episodes(
        &self,
        provider: &str,
        series: u64,
    ) -> rusqlite::Result<(HashMap<String, Progress>, Option<String>)> {
        let Ok(series) = i64::try_from(series) else {
            return Ok((HashMap::new(), None));
        };
        let mut statement = self.conn.prepare_cached(
            "SELECT id, position, duration, watched, updated FROM progress
             WHERE provider = ?1 AND kind = ?2 AND series = ?3 ORDER BY updated",
        )?;
        let rows = statement
            .query_map(params![provider, kind_code(Kind::Episode), series], |row| {
                Ok((row.get::<_, String>(0)?, progress_at(row, 1)?))
            })?;
        let mut episodes = HashMap::new();
        let mut last = None;
        for row in rows {
            let (id, progress) = row?;
            last = Some(id.clone());
            episodes.insert(id, progress);
        }
        Ok((episodes, last))
    }

    /// Titles started and not finished, latest first, at most `limit`; a
    /// series once, by the episode watched last.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn unfinished(&self, limit: usize) -> rusqlite::Result<Vec<(Entry, Progress)>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT provider, kind, id, series, title, position, duration, watched, updated
             FROM progress ORDER BY updated DESC",
        )?;
        let rows = statement.query_map([], |row| {
            let entry = Entry {
                provider: row.get(0)?,
                kind: match row.get::<_, i64>(1)? {
                    1 => Kind::Episode,
                    _ => Kind::Movie,
                },
                id: row.get(2)?,
                series: row
                    .get::<_, Option<i64>>(3)?
                    .and_then(|s| u64::try_from(s).ok()),
                title: row.get(4)?,
            };
            Ok((entry, progress_at(row, 5)?))
        })?;
        let mut seen = HashSet::new();
        let mut unfinished = Vec::new();
        for row in rows {
            let (entry, progress) = row?;
            // A series counts once, by its latest episode, finished or not.
            let title = (entry.provider.clone(), entry.series, entry.id.clone());
            let key = match entry.series {
                Some(_) => (title.0, title.1, String::new()),
                None => title,
            };
            if !seen.insert(key) || progress.resume_at().is_none() {
                continue;
            }
            unfinished.push((entry, progress));
            if unfinished.len() == limit {
                break;
            }
        }
        Ok(unfinished)
    }

    /// Forgets everything watched from `provider`.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written.
    pub fn remove(&self, provider: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM progress WHERE provider = ?1", [provider])?;
        Ok(())
    }
}

impl Progress {
    /// Where to resume, when far enough in and not finished.
    pub fn resume_at(&self) -> Option<f64> {
        (!self.watched && self.position >= MIN_RESUME).then_some(self.position)
    }

    /// How far in, 0 to 1; 0 when the length is unknown.
    pub fn fraction(&self) -> f32 {
        if self.duration > 0.0 {
            (self.position / self.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }

    /// Seconds left; 0 when the length is unknown.
    pub fn left(&self) -> f64 {
        (self.duration - self.position).max(0.0)
    }
}

// Private API
impl History {
    fn with(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }
}

const fn kind_code(kind: Kind) -> i64 {
    match kind {
        Kind::Movie => 0,
        Kind::Episode => 1,
    }
}

fn progress(row: &Row) -> rusqlite::Result<Progress> {
    progress_at(row, 0)
}

/// The four progress columns starting at `first`.
fn progress_at(row: &Row, first: usize) -> rusqlite::Result<Progress> {
    Ok(Progress {
        position: row.get(first)?,
        duration: row.get(first + 1)?,
        watched: row.get(first + 2)?,
        updated: row.get(first + 3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: Kind, id: &str, series: Option<u64>) -> Entry {
        Entry {
            provider: "north".into(),
            kind,
            id: id.into(),
            series,
            title: "Cottontail".into(),
        }
    }

    #[test]
    fn progress_round_trips_and_marks_watched() {
        let history = History::in_memory().unwrap();
        let movie = entry(Kind::Movie, "501", None);
        assert_eq!(history.progress("north", Kind::Movie, "501").unwrap(), None);

        history.save(&movie, 600.0, 5400.0, 100).unwrap();
        let p = history
            .progress("north", Kind::Movie, "501")
            .unwrap()
            .unwrap();
        assert_eq!((p.position, p.watched, p.updated), (600.0, false, 100));
        assert_eq!(p.resume_at(), Some(600.0));
        assert_eq!(p.left(), 4800.0);

        history.save(&movie, 5200.0, 5400.0, 200).unwrap();
        let p = history
            .progress("north", Kind::Movie, "501")
            .unwrap()
            .unwrap();
        assert!(p.watched, "past 95% counts as watched");
        assert_eq!(p.resume_at(), None);

        history.save(&movie, 10.0, 5400.0, 300).unwrap();
        let p = history
            .progress("north", Kind::Movie, "501")
            .unwrap()
            .unwrap();
        assert_eq!(p.resume_at(), None, "too early to resume");
        assert!(
            history
                .progress("north", Kind::Episode, "501")
                .unwrap()
                .is_none(),
            "kinds are apart"
        );
    }

    #[test]
    fn episodes_list_a_series_and_the_last_watched() {
        let history = History::in_memory().unwrap();
        history
            .save(&entry(Kind::Episode, "e2", Some(7)), 900.0, 1500.0, 20)
            .unwrap();
        history
            .save(&entry(Kind::Episode, "e1", Some(7)), 1500.0, 1500.0, 10)
            .unwrap();
        history
            .save(&entry(Kind::Episode, "x1", Some(8)), 60.0, 1500.0, 30)
            .unwrap();
        let (episodes, last) = history.episodes("north", 7).unwrap();
        assert_eq!(episodes.len(), 2);
        assert!(episodes["e1"].watched);
        assert_eq!(last.as_deref(), Some("e2"));
        assert_eq!(history.all("north", Kind::Episode).unwrap().len(), 3);

        history.remove("north").unwrap();
        assert!(history.all("north", Kind::Episode).unwrap().is_empty());
    }

    #[test]
    fn unfinished_lists_latest_first_and_a_series_once() {
        let history = History::in_memory().unwrap();
        history
            .save(&entry(Kind::Movie, "501", None), 600.0, 5400.0, 10)
            .unwrap();
        history
            .save(&entry(Kind::Movie, "502", None), 5300.0, 5400.0, 20)
            .unwrap();
        history
            .save(&entry(Kind::Episode, "e1", Some(7)), 900.0, 1500.0, 30)
            .unwrap();
        history
            .save(&entry(Kind::Episode, "e2", Some(7)), 300.0, 1500.0, 40)
            .unwrap();
        let ids: Vec<_> = history
            .unfinished(10)
            .unwrap()
            .into_iter()
            .map(|(e, _)| e.id)
            .collect();
        assert_eq!(ids, ["e2", "501"], "watched 502 and older e1 left out");
        assert_eq!(history.unfinished(1).unwrap().len(), 1);
    }
}
