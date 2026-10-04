//! Favorites: the channels, movies and series the user keeps, across
//! providers, in the user's order.
//!
//! Kept in SQLite in the data directory. Each favorite records its title,
//! poster and year as they were when it was added, so lists show without
//! loading the providers' shelves, and a channel a provider drops can still
//! be named.

use std::collections::HashSet;

use camino::Utf8Path;
use rusqlite::{Connection, OptionalExtension, Row, params};

/// Tables and indexes.
const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS favorite (
        provider TEXT NOT NULL,
        kind INTEGER NOT NULL,
        id TEXT NOT NULL,
        title TEXT NOT NULL,
        poster TEXT NOT NULL,
        year INTEGER,
        position INTEGER NOT NULL,
        added INTEGER NOT NULL,
        PRIMARY KEY (provider, kind, id)
    );
    CREATE INDEX IF NOT EXISTS favorite_by_position ON favorite (kind, position);
";

/// The favorites store.
pub struct Favorites {
    conn: Connection,
}

/// What kind of thing a favorite is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A live channel, by stream id.
    Channel,
    /// A movie, by stream id.
    Movie,
    /// A series, by series id.
    Series,
}

/// One favorite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    /// The provider's id.
    pub provider: String,
    /// Channel, movie or series.
    pub kind: Kind,
    /// The provider's id for it.
    pub id: String,
    /// Its name when it was added.
    pub title: String,
    /// Its logo or poster URL when it was added; empty without one.
    pub poster: String,
    /// Its year, for a movie or series.
    pub year: Option<u16>,
}

// Public API
impl Favorites {
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

    /// Adds `favorite` at the end of its kind's list; one already there
    /// keeps its place.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written.
    pub fn add(&self, favorite: &Favorite, now: i64) -> rusqlite::Result<()> {
        let kind = kind_code(favorite.kind);
        self.conn.execute(
            "INSERT OR IGNORE INTO favorite (provider, kind, id, title, poster, year, position, added)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                     (SELECT COALESCE(MAX(position), 0) + 1 FROM favorite WHERE kind = ?2), ?7)",
            params![
                favorite.provider,
                kind,
                favorite.id,
                favorite.title,
                favorite.poster,
                favorite.year,
                now
            ],
        )?;
        Ok(())
    }

    /// Removes a favorite; one that is not there is not an error.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written.
    pub fn remove(&self, provider: &str, kind: Kind, id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM favorite WHERE provider = ?1 AND kind = ?2 AND id = ?3",
            params![provider, kind_code(kind), id],
        )?;
        Ok(())
    }

    /// Whether `provider`'s `id` of `kind` is a favorite.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn contains(&self, provider: &str, kind: Kind, id: &str) -> rusqlite::Result<bool> {
        self.conn
            .query_row(
                "SELECT 1 FROM favorite WHERE provider = ?1 AND kind = ?2 AND id = ?3",
                params![provider, kind_code(kind), id],
                |_| Ok(()),
            )
            .optional()
            .map(|found| found.is_some())
    }

    /// The favorites of `kind`, in the user's order.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn list(&self, kind: Kind) -> rusqlite::Result<Vec<Favorite>> {
        let mut statement = self.conn.prepare_cached(
            "SELECT provider, kind, id, title, poster, year FROM favorite
             WHERE kind = ?1 ORDER BY position",
        )?;
        let rows = statement.query_map([kind_code(kind)], favorite)?;
        rows.collect()
    }

    /// The ids of `provider`'s favorites of `kind`.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read.
    pub fn ids(&self, provider: &str, kind: Kind) -> rusqlite::Result<HashSet<String>> {
        let mut statement = self
            .conn
            .prepare_cached("SELECT id FROM favorite WHERE provider = ?1 AND kind = ?2")?;
        let rows = statement.query_map(params![provider, kind_code(kind)], |row| row.get(0))?;
        rows.collect()
    }

    /// Moves a favorite one place earlier (`delta` below 0) or later in its
    /// kind's list; `false` when it is at that end or not there.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be read or written.
    pub fn move_by(
        &mut self,
        provider: &str,
        kind: Kind,
        id: &str,
        delta: i32,
    ) -> rusqlite::Result<bool> {
        let kind = kind_code(kind);
        let tx = self.conn.transaction()?;
        let Some(position) = tx
            .query_row(
                "SELECT position FROM favorite WHERE provider = ?1 AND kind = ?2 AND id = ?3",
                params![provider, kind, id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        else {
            return Ok(false);
        };
        let neighbour = if delta < 0 {
            "SELECT position FROM favorite WHERE kind = ?1 AND position < ?2 ORDER BY position DESC LIMIT 1"
        } else {
            "SELECT position FROM favorite WHERE kind = ?1 AND position > ?2 ORDER BY position LIMIT 1"
        };
        let Some(other) = tx
            .query_row(neighbour, params![kind, position], |row| {
                row.get::<_, i64>(0)
            })
            .optional()?
        else {
            return Ok(false);
        };
        // Swap through a free position: (kind, position) has no uniqueness
        // constraint, but a free slot keeps the swap obvious.
        tx.execute(
            "UPDATE favorite SET position = -1 WHERE kind = ?1 AND position = ?2",
            params![kind, other],
        )?;
        tx.execute(
            "UPDATE favorite SET position = ?1 WHERE kind = ?2 AND position = ?3",
            params![other, kind, position],
        )?;
        tx.execute(
            "UPDATE favorite SET position = ?1 WHERE kind = ?2 AND position = -1",
            params![position, kind],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Forgets every favorite from `provider`.
    ///
    /// # Errors
    ///
    /// Returns an error when the store cannot be written.
    pub fn remove_provider(&self, provider: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM favorite WHERE provider = ?1", [provider])?;
        Ok(())
    }
}

// Private API
impl Favorites {
    fn with(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }
}

const fn kind_code(kind: Kind) -> i64 {
    match kind {
        Kind::Channel => 0,
        Kind::Movie => 1,
        Kind::Series => 2,
    }
}

fn favorite(row: &Row) -> rusqlite::Result<Favorite> {
    Ok(Favorite {
        provider: row.get(0)?,
        kind: match row.get::<_, i64>(1)? {
            1 => Kind::Movie,
            2 => Kind::Series,
            _ => Kind::Channel,
        },
        id: row.get(2)?,
        title: row.get(3)?,
        poster: row.get(4)?,
        year: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(provider: &str, id: &str) -> Favorite {
        Favorite {
            provider: provider.into(),
            kind: Kind::Channel,
            id: id.into(),
            title: format!("Channel {id}"),
            poster: String::new(),
            year: None,
        }
    }

    fn order(store: &Favorites) -> Vec<String> {
        store
            .list(Kind::Channel)
            .unwrap()
            .into_iter()
            .map(|f| format!("{}/{}", f.provider, f.id))
            .collect()
    }

    #[test]
    fn favorites_keep_the_order_they_were_added_in() {
        let store = Favorites::in_memory().unwrap();
        store.add(&channel("north", "1"), 10).unwrap();
        store.add(&channel("south", "1"), 20).unwrap();
        store.add(&channel("north", "2"), 30).unwrap();
        store.add(&channel("north", "1"), 40).unwrap();
        assert_eq!(
            order(&store),
            ["north/1", "south/1", "north/2"],
            "adding again keeps the place"
        );
        assert!(store.contains("south", Kind::Channel, "1").unwrap());
        assert!(
            !store.contains("south", Kind::Movie, "1").unwrap(),
            "kinds are apart"
        );
        assert_eq!(store.ids("north", Kind::Channel).unwrap().len(), 2);

        store.remove("south", Kind::Channel, "1").unwrap();
        assert_eq!(order(&store), ["north/1", "north/2"]);
        store.remove("south", Kind::Channel, "1").unwrap();
    }

    #[test]
    fn move_by_swaps_with_the_neighbour() {
        let mut store = Favorites::in_memory().unwrap();
        for (provider, id) in [("a", "1"), ("b", "1"), ("a", "2")] {
            store.add(&channel(provider, id), 0).unwrap();
        }
        assert!(store.move_by("a", Kind::Channel, "2", -1).unwrap());
        assert_eq!(order(&store), ["a/1", "a/2", "b/1"]);
        assert!(store.move_by("a", Kind::Channel, "1", 1).unwrap());
        assert_eq!(order(&store), ["a/2", "a/1", "b/1"]);
        assert!(
            !store.move_by("a", Kind::Channel, "2", -1).unwrap(),
            "already first"
        );
        assert!(
            !store.move_by("b", Kind::Channel, "1", 1).unwrap(),
            "already last"
        );
        assert!(
            !store.move_by("c", Kind::Channel, "1", 1).unwrap(),
            "not there"
        );
    }

    #[test]
    fn titles_are_kept_and_providers_forgotten() {
        let store = Favorites::in_memory().unwrap();
        let movie = Favorite {
            provider: "a".into(),
            kind: Kind::Movie,
            id: "501".into(),
            title: "Cottontail".into(),
            poster: "http://p/501.jpg".into(),
            year: Some(2026),
        };
        store.add(&movie, 0).unwrap();
        store.add(&channel("a", "7"), 0).unwrap();
        store.add(&channel("b", "7"), 0).unwrap();
        assert_eq!(store.list(Kind::Movie).unwrap(), [movie]);
        store.remove_provider("a").unwrap();
        assert!(store.list(Kind::Movie).unwrap().is_empty());
        assert_eq!(order(&store), ["b/7"]);
    }
}
