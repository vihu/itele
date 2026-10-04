//! The programme guide: each provider's XMLTV, parsed as a stream into a
//! local SQLite store, then read for now and next, the guide grid, and
//! search.
//!
//! Programmes are keyed by provider and by the XMLTV channel id, which a
//! live stream names as its `epg_channel_id`; ids compare lowercased.
//! Times are Unix seconds.

use std::collections::HashSet;
use std::io::BufRead;
use std::ops::Range;

use camino::Utf8Path;
use quick_xml::events::{BytesStart, Event};
use rusqlite::{Connection, OptionalExtension, params};

/// Result type for this module.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Tables, indexes, and the title search index.
const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS programme (
        id INTEGER PRIMARY KEY,
        provider TEXT NOT NULL,
        channel TEXT NOT NULL,
        start INTEGER NOT NULL,
        stop INTEGER NOT NULL,
        title TEXT NOT NULL,
        description TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS programme_by_channel ON programme (provider, channel, start);
    CREATE VIRTUAL TABLE IF NOT EXISTS programme_search USING fts5 (title);
    CREATE TABLE IF NOT EXISTS import (provider TEXT PRIMARY KEY, at INTEGER NOT NULL);
";

/// The local guide store.
pub struct Store {
    conn: Connection,
}

/// One programme on one channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Programme {
    /// Start, Unix seconds.
    pub start: i64,
    /// End, Unix seconds.
    pub stop: i64,
    /// Title.
    pub title: String,
    /// Description; empty when the guide has none.
    pub description: String,
}

/// A search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The provider whose guide lists the programme.
    pub provider: String,
    /// XMLTV channel id, lowercased.
    pub channel: String,
    /// The programme.
    pub programme: Programme,
}

/// What can go wrong reading or storing the guide.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The SQLite store failed.
    Sql(rusqlite::Error),
    /// The XMLTV document is not well-formed.
    Xml(quick_xml::Error),
}

// Public API
impl Store {
    /// Opens (or creates) the store at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sql`] when the file cannot be opened or migrated.
    pub fn open(path: &Utf8Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            // A missing directory surfaces as the open error below.
            let _ = std::fs::create_dir_all(dir);
        }
        Self::with(Connection::open(path).map_err(Error::Sql)?)
    }

    /// An empty store in memory, for tests.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sql`] when SQLite lacks FTS5.
    pub fn in_memory() -> Result<Self> {
        Self::with(Connection::open_in_memory().map_err(Error::Sql)?)
    }

    /// Replaces `provider`'s programmes with the ones in `xml`.
    ///
    /// Keeps only channels in `wanted` (lowercased ids) and programmes that
    /// overlap `keep`, so a guide for thousands of unused channels or weeks
    /// ahead stays small. Returns the number of programmes stored. The old
    /// programmes stay when parsing fails.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Xml`] when the document is malformed and
    /// [`Error::Sql`] when the store cannot be written.
    pub fn import(
        &mut self,
        provider: &str,
        wanted: &HashSet<String>,
        keep: Range<i64>,
        xml: impl BufRead,
        now: i64,
    ) -> Result<usize> {
        let tx = self.conn.transaction().map_err(Error::Sql)?;
        tx.execute(
            "DELETE FROM programme_search WHERE rowid IN \
             (SELECT id FROM programme WHERE provider = ?1)",
            [provider],
        )
        .map_err(Error::Sql)?;
        tx.execute("DELETE FROM programme WHERE provider = ?1", [provider])
            .map_err(Error::Sql)?;
        let mut stored = 0;
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO programme (provider, channel, start, stop, title, description) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(Error::Sql)?;
            let mut index = tx
                .prepare("INSERT INTO programme_search (rowid, title) VALUES (?1, ?2)")
                .map_err(Error::Sql)?;
            parse(xml, |channel, programme| {
                let wanted = wanted.contains(&channel)
                    && programme.stop > keep.start
                    && programme.start < keep.end
                    && programme.stop > programme.start;
                if !wanted {
                    return Ok(());
                }
                insert
                    .execute(params![
                        provider,
                        channel,
                        programme.start,
                        programme.stop,
                        programme.title,
                        programme.description
                    ])
                    .map_err(Error::Sql)?;
                index
                    .execute(params![tx.last_insert_rowid(), programme.title])
                    .map_err(Error::Sql)?;
                stored += 1;
                Ok(())
            })?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO import (provider, at) VALUES (?1, ?2)",
            params![provider, now],
        )
        .map_err(Error::Sql)?;
        tx.commit().map_err(Error::Sql)?;
        Ok(stored)
    }

    /// When `provider`'s guide was last imported, Unix seconds.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sql`] when the store cannot be read.
    pub fn imported_at(&self, provider: &str) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT at FROM import WHERE provider = ?1",
                [provider],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::Sql)
    }

    /// Deletes `provider`'s programmes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sql`] when the store cannot be written.
    pub fn remove(&mut self, provider: &str) -> Result {
        let tx = self.conn.transaction().map_err(Error::Sql)?;
        tx.execute(
            "DELETE FROM programme_search WHERE rowid IN \
             (SELECT id FROM programme WHERE provider = ?1)",
            [provider],
        )
        .map_err(Error::Sql)?;
        tx.execute("DELETE FROM programme WHERE provider = ?1", [provider])
            .map_err(Error::Sql)?;
        tx.execute("DELETE FROM import WHERE provider = ?1", [provider])
            .map_err(Error::Sql)?;
        tx.commit().map_err(Error::Sql)
    }

    /// The programmes on `channel` that overlap `from..to`, in order.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sql`] when the store cannot be read.
    pub fn between(
        &self,
        provider: &str,
        channel: &str,
        from: i64,
        to: i64,
    ) -> Result<Vec<Programme>> {
        let mut query = self
            .conn
            .prepare_cached(
                "SELECT start, stop, title, description FROM programme \
                 WHERE provider = ?1 AND channel = ?2 AND stop > ?3 AND start < ?4 \
                 ORDER BY start",
            )
            .map_err(Error::Sql)?;
        let rows = query
            .query_map(
                params![provider, channel.to_lowercase(), from, to],
                programme,
            )
            .map_err(Error::Sql)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Error::Sql)
    }

    /// The programme on `channel` at `at`, and the one after it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sql`] when the store cannot be read.
    pub fn now_next(
        &self,
        provider: &str,
        channel: &str,
        at: i64,
    ) -> Result<(Option<Programme>, Option<Programme>)> {
        let mut query = self
            .conn
            .prepare_cached(
                "SELECT start, stop, title, description FROM programme \
                 WHERE provider = ?1 AND channel = ?2 AND stop > ?3 \
                 ORDER BY start LIMIT 2",
            )
            .map_err(Error::Sql)?;
        let mut rows: Vec<Programme> = query
            .query_map(params![provider, channel.to_lowercase(), at], programme)
            .map_err(Error::Sql)?
            .collect::<rusqlite::Result<_>>()
            .map_err(Error::Sql)?;
        let next = (rows.len() > 1).then(|| rows.remove(1));
        let first = rows.pop();
        Ok(match first {
            Some(p) if p.start <= at => (Some(p), next),
            // Nothing airs now: the first row is already the next one.
            first => (None, first),
        })
    }

    /// Programmes whose title matches every word of `query` (prefixes
    /// count), across all providers: airing or upcoming first, then the
    /// most recent past ones.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sql`] when the store cannot be read.
    pub fn search(&self, query: &str, at: i64, limit: usize) -> Result<Vec<Hit>> {
        let Some(expression) = match_expression(query) else {
            return Ok(Vec::new());
        };
        let mut statement = self
            .conn
            .prepare_cached(
                "SELECT p.provider, p.channel, p.start, p.stop, p.title, p.description \
                 FROM programme_search s JOIN programme p ON p.id = s.rowid \
                 WHERE programme_search MATCH ?1 \
                 ORDER BY (p.stop <= ?2), CASE WHEN p.stop > ?2 THEN p.start ELSE -p.start END \
                 LIMIT ?3",
            )
            .map_err(Error::Sql)?;
        let rows = statement
            .query_map(params![expression, at, limit as i64], |row| {
                Ok(Hit {
                    provider: row.get(0)?,
                    channel: row.get(1)?,
                    programme: Programme {
                        start: row.get(2)?,
                        stop: row.get(3)?,
                        title: row.get(4)?,
                        description: row.get(5)?,
                    },
                })
            })
            .map_err(Error::Sql)?;
        rows.collect::<rusqlite::Result<_>>().map_err(Error::Sql)
    }
}

/// Parses an XMLTV time: `20261004213000 +0100`, with or without the
/// offset (UTC then) or the seconds.
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let rest = text[digits.len()..].trim();
    let number = |range: Range<usize>| digits.get(range)?.parse::<i32>().ok();
    let (year, month, day) = (number(0..4)?, number(4..6)?, number(6..8)?);
    let (hour, minute) = (number(8..10)?, number(10..12)?);
    let second = if digits.len() >= 14 {
        number(12..14)?
    } else {
        0
    };
    let offset_seconds = match rest.as_bytes() {
        [] => 0,
        [sign @ (b'+' | b'-'), ..] if rest.len() >= 5 => {
            let hours: i32 = rest.get(1..3)?.parse().ok()?;
            let minutes: i32 = rest.get(3..5)?.parse().ok()?;
            let seconds = hours * 3600 + minutes * 60;
            if *sign == b'-' { -seconds } else { seconds }
        }
        _ if rest.eq_ignore_ascii_case("z") || rest.eq_ignore_ascii_case("utc") => 0,
        _ => return None,
    };
    let civil = jiff::civil::DateTime::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        0,
    )
    .ok()?;
    let offset = jiff::tz::Offset::from_seconds(offset_seconds).ok()?;
    Some(offset.to_timestamp(civil).ok()?.as_second())
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Sql(e) => Some(e),
            Error::Xml(e) => Some(e),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::Sql(e) => write!(f, "the programme guide store failed: {e}"),
            Error::Xml(e) => write!(f, "the programme guide is not valid XMLTV: {e}"),
        }
    }
}

// Private API
impl Store {
    fn with(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA).map_err(Error::Sql)?;
        Ok(Self { conn })
    }
}

fn programme(row: &rusqlite::Row) -> rusqlite::Result<Programme> {
    Ok(Programme {
        start: row.get(0)?,
        stop: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
    })
}

/// An FTS5 expression that matches every word of `query` as a prefix, or
/// `None` when the query has no words.
fn match_expression(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// Which text the parser is collecting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    None,
    Title,
    Description,
}

/// Streams the `<programme>` elements of `xml` into `found` with their
/// lowercased channel id. Programmes with unreadable times are skipped.
fn parse(xml: impl BufRead, mut found: impl FnMut(String, Programme) -> Result) -> Result {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut current: Option<(String, Programme)> = None;
    let mut field = Field::None;
    let mut has_title = false;
    loop {
        let event = reader.read_event_into(&mut buf).map_err(Error::Xml)?;
        match event {
            Event::Start(e) if e.local_name().as_ref() == "programme" => {
                current = start_programme(&e);
                has_title = false;
            }
            Event::Start(e) if current.is_some() => {
                field = match e.local_name().as_ref() {
                    // The first title is the main one; others are translations.
                    "title" if !has_title => Field::Title,
                    "desc" => Field::Description,
                    _ => Field::None,
                };
            }
            Event::Text(t) => push(&mut current, field, &t.xml10_content()),
            Event::CData(c) => push(&mut current, field, &c),
            Event::GeneralRef(r) => {
                let text = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => quick_xml::escape::resolve_predefined_entity(r.as_ref())
                        .map_or_else(|| format!("&{};", r.as_ref()), str::to_owned),
                };
                push(&mut current, field, &text);
            }
            Event::End(e) => match e.local_name().as_ref() {
                "programme" => {
                    if let Some((channel, mut programme)) = current.take() {
                        programme.title = programme.title.trim().to_owned();
                        programme.description = programme.description.trim().to_owned();
                        if !programme.title.is_empty() {
                            found(channel, programme)?;
                        }
                    }
                }
                "title" if field == Field::Title => {
                    has_title = true;
                    field = Field::None;
                }
                _ => field = Field::None,
            },
            Event::Eof => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

/// A programme from its start tag, or `None` when it lacks a channel or a
/// readable start and stop.
fn start_programme(e: &BytesStart) -> Option<(String, Programme)> {
    let attribute = |name: &str| {
        e.try_get_attribute(name).ok().flatten().and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
    };
    let channel = attribute("channel")?.trim().to_lowercase();
    let start = parse_time(&attribute("start")?)?;
    let stop = parse_time(&attribute("stop")?)?;
    Some((
        channel,
        Programme {
            start,
            stop,
            title: String::new(),
            description: String::new(),
        },
    ))
}

fn push(current: &mut Option<(String, Programme)>, field: Field, text: &str) {
    if let Some((_, programme)) = current {
        match field {
            Field::Title => programme.title.push_str(text),
            Field::Description => programme.description.push_str(text),
            Field::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUIDE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE tv SYSTEM "xmltv.dtd">
<tv generator-info-name="test">
  <channel id="atlas.nature"><display-name>Atlas</display-name></channel>
  <programme start="20261004200000 +0000" stop="20261004210000 +0000" channel="Atlas.Nature">
    <title lang="en">Wild Coasts</title>
    <title lang="fr">Côtes sauvages</title>
    <desc lang="en">Tides &amp; storms, &#8220;live&#8221;.</desc>
  </programme>
  <programme start="20261004210000 +0000" stop="20261004220000 +0000" channel="atlas.nature">
    <title><![CDATA[The Meadow Year]]></title>
  </programme>
  <programme start="20261004220000 +0000" stop="20261004230000 +0000" channel="atlas.nature">
    <title>Night Shift: Owls</title>
  </programme>
  <programme start="20261004210000 +0000" stop="20261004220000 +0000" channel="other.channel">
    <title>Not wanted</title>
  </programme>
  <programme start="garbage" stop="20261004220000 +0000" channel="atlas.nature">
    <title>Bad time</title>
  </programme>
  <programme start="20261020210000 +0000" stop="20261020220000 +0000" channel="atlas.nature">
    <title>Too far ahead</title>
  </programme>
</tv>"#;

    /// 2026-10-04 21:22 UTC.
    const NOW: i64 = 1_791_148_920;

    fn store() -> Store {
        let mut store = Store::in_memory().unwrap();
        let wanted = HashSet::from(["atlas.nature".to_owned()]);
        let keep = NOW - 86_400..NOW + 7 * 86_400;
        let stored = store
            .import("north", &wanted, keep, GUIDE.as_bytes(), NOW)
            .unwrap();
        assert_eq!(stored, 3);
        store
    }

    #[test]
    fn parse_time_reads_offsets_and_short_forms() {
        assert_eq!(parse_time("20261004212200 +0000"), Some(NOW));
        assert_eq!(parse_time("20261004222200 +0100"), Some(NOW));
        assert_eq!(parse_time("20261004162200 -0500"), Some(NOW));
        assert_eq!(parse_time("20261004212200"), Some(NOW), "no offset is UTC");
        assert_eq!(parse_time("202610042122"), Some(NOW), "no seconds");
        assert_eq!(parse_time("20261004212200 Z"), Some(NOW));
        assert_eq!(parse_time("garbage"), None);
        assert_eq!(parse_time("20261304212200 +0000"), None, "month 13");
        assert_eq!(parse_time("20261004212200 +01"), None);
    }

    #[test]
    fn import_keeps_wanted_channels_in_the_window() {
        let store = store();
        let all = store.between("north", "ATLAS.NATURE", 0, i64::MAX).unwrap();
        let titles: Vec<_> = all.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Wild Coasts", "The Meadow Year", "Night Shift: Owls"]
        );
        assert_eq!(all[0].description, "Tides & storms, \u{201c}live\u{201d}.");
        assert_eq!(store.imported_at("north").unwrap(), Some(NOW));
        assert_eq!(store.imported_at("south").unwrap(), None);
    }

    #[test]
    fn now_next_finds_the_airing_programme() {
        let store = store();
        let (now, next) = store.now_next("north", "atlas.nature", NOW).unwrap();
        assert_eq!(now.unwrap().title, "The Meadow Year");
        assert_eq!(next.unwrap().title, "Night Shift: Owls");
        let gap = parse_time("20261004193000").unwrap();
        let (now, next) = store.now_next("north", "atlas.nature", gap).unwrap();
        assert_eq!((now, next.unwrap().title.as_str()), (None, "Wild Coasts"));
        let late = parse_time("20261004235900").unwrap();
        assert_eq!(
            store.now_next("north", "atlas.nature", late).unwrap(),
            (None, None)
        );
    }

    #[test]
    fn reimport_replaces_and_remove_clears() {
        let mut store = store();
        let wanted = HashSet::from(["atlas.nature".to_owned()]);
        let xml = r#"<tv><programme start="20261004210000 +0000" stop="20261004220000 +0000"
            channel="atlas.nature"><title>Replacement</title></programme></tv>"#;
        store
            .import("north", &wanted, 0..i64::MAX, xml.as_bytes(), NOW + 1)
            .unwrap();
        let titles: Vec<_> = store
            .between("north", "atlas.nature", 0, i64::MAX)
            .unwrap()
            .into_iter()
            .map(|p| p.title)
            .collect();
        assert_eq!(titles, ["Replacement"]);
        assert!(
            store.search("meadow", NOW, 10).unwrap().is_empty(),
            "old titles leave the index"
        );
        store.remove("north").unwrap();
        assert!(
            store
                .between("north", "atlas.nature", 0, i64::MAX)
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.imported_at("north").unwrap(), None);
    }

    #[test]
    fn malformed_xml_keeps_the_old_guide() {
        let mut store = store();
        let wanted = HashSet::from(["atlas.nature".to_owned()]);
        let broken = "<tv><programme start=\"20261004210000\" stop=\"20261004220000\" channel=\"atlas.nature\"><title>X</tv>";
        assert!(matches!(
            store.import("north", &wanted, 0..i64::MAX, broken.as_bytes(), NOW),
            Err(Error::Xml(_))
        ));
        assert_eq!(
            store
                .between("north", "atlas.nature", 0, i64::MAX)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn search_matches_prefixes_upcoming_first() {
        let store = store();
        let hits = store.search("nig sh", NOW, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].provider.as_str(), hits[0].channel.as_str()),
            ("north", "atlas.nature")
        );
        let titles: Vec<_> = store
            .search("the", NOW, 10)
            .unwrap()
            .into_iter()
            .map(|h| h.programme.title)
            .collect();
        assert_eq!(titles, ["The Meadow Year"]);
        let order: Vec<_> = store
            .search("w", NOW, 10)
            .unwrap()
            .into_iter()
            .map(|h| h.programme.title)
            .collect();
        assert_eq!(order, ["Wild Coasts"], "past programmes still match");
        assert!(store.search("  \"*( ", NOW, 10).unwrap().is_empty());
    }
}
