//! Movies and series: the lists, the details of one title, and stream URLs.
//!
//! Same rules as live streams: lenient field by field, entries without an
//! id dropped, and every stream URL carries the password.

use percent_encoding::utf8_percent_encode;

use super::raw::{RawEpisode, RawSeries, RawSeriesInfo, RawVodInfo, RawVodStream};
use super::{CategoryId, Credentials, Error, PATH_SEGMENT, Result, StreamId};

/// Container assumed when the provider names none.
const DEFAULT_EXTENSION: &str = "mp4";

/// A provider's id for a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SeriesId(pub u64);

/// A provider's id for an episode, used in its stream URL.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EpisodeId(pub String);

/// One movie in the provider's list.
#[derive(Clone, Debug, PartialEq)]
pub struct Movie {
    /// The provider's id, used in the stream URL.
    pub id: StreamId,
    /// Display name.
    pub name: String,
    /// Category the movie is listed under, if any.
    pub category_id: Option<CategoryId>,
    /// Poster URL, if any.
    pub poster: Option<String>,
    /// Rating out of 10, if rated.
    pub rating: Option<f32>,
    /// Release year: the provider's, or a `(2024)` ending the name.
    pub year: Option<u16>,
    /// When the provider added it, as Unix seconds.
    pub added: Option<u64>,
    /// Container, for example `mkv`, used in the stream URL.
    pub extension: String,
}

/// One series in the provider's list.
#[derive(Clone, Debug, PartialEq)]
pub struct Show {
    /// The provider's id.
    pub id: SeriesId,
    /// Display name.
    pub name: String,
    /// Category the series is listed under, if any.
    pub category_id: Option<CategoryId>,
    /// Poster URL, if any.
    pub poster: Option<String>,
    /// Rating out of 10, if rated.
    pub rating: Option<f32>,
    /// First-aired year, if known.
    pub year: Option<u16>,
    /// When the provider last changed it, as Unix seconds.
    pub updated: Option<u64>,
}

/// What the detail page of a movie or series shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    /// Synopsis.
    pub plot: String,
    /// Main cast, comma separated.
    pub cast: String,
    /// Director or creators.
    pub director: String,
    /// Genres, comma separated.
    pub genre: String,
    /// Release or first-aired date as the provider writes it.
    pub released: String,
    /// Running time in seconds (per episode for a series).
    pub duration: Option<u32>,
    /// Rating out of 10, if rated.
    pub rating: Option<f32>,
    /// A wide background image, if any.
    pub backdrop: Option<String>,
    /// A larger poster than the list's, if any.
    pub poster: Option<String>,
}

/// The details of one movie.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MovieInfo {
    /// Plot, cast and the rest.
    pub details: Details,
    /// Container, when the details name one.
    pub extension: Option<String>,
}

/// The details of one series, with its seasons and episodes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShowInfo {
    /// Plot, cast and the rest.
    pub details: Details,
    /// Seasons in order; every season with episodes is listed.
    pub seasons: Vec<Season>,
    /// Episodes ordered by season, then number.
    pub episodes: Vec<Episode>,
}

/// One season of a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Season {
    /// Season number; 0 holds specials.
    pub number: u32,
    /// Display name, for example `Season 2`.
    pub name: String,
    /// Synopsis, often empty.
    pub overview: String,
    /// Poster URL, if any.
    pub cover: Option<String>,
}

/// One episode of a series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Episode {
    /// The provider's id, used in the stream URL.
    pub id: EpisodeId,
    /// Season number.
    pub season: u32,
    /// Episode number within the season.
    pub number: u32,
    /// Episode title.
    pub title: String,
    /// Container, used in the stream URL.
    pub extension: String,
    /// Synopsis.
    pub plot: String,
    /// Running time in seconds, if known.
    pub duration: Option<u32>,
    /// A still from the episode, if any.
    pub still: Option<String>,
    /// Air date as the provider writes it.
    pub released: String,
}

impl Credentials {
    /// Returns the URL that plays movie `stream` in container `extension`.
    ///
    /// The URL embeds the username and password; never log it.
    pub fn movie_url(&self, stream: StreamId, extension: &str) -> String {
        format!(
            "{}/movie/{}/{}/{}.{}",
            self.server,
            utf8_percent_encode(&self.username, PATH_SEGMENT),
            utf8_percent_encode(&self.password, PATH_SEGMENT),
            stream.0,
            utf8_percent_encode(extension, PATH_SEGMENT),
        )
    }

    /// Returns the URL that plays `episode` in container `extension`.
    ///
    /// The URL embeds the username and password; never log it.
    pub fn episode_url(&self, episode: &EpisodeId, extension: &str) -> String {
        format!(
            "{}/series/{}/{}/{}.{}",
            self.server,
            utf8_percent_encode(&self.username, PATH_SEGMENT),
            utf8_percent_encode(&self.password, PATH_SEGMENT),
            utf8_percent_encode(&episode.0, PATH_SEGMENT),
            utf8_percent_encode(extension, PATH_SEGMENT),
        )
    }
}

/// Parses the body of [`Action::VodStreams`][super::Action::VodStreams],
/// dropping entries without a stream id.
///
/// # Errors
///
/// Returns [`Error::Json`] when the body is not a JSON array of objects.
pub fn parse_movies(json: &str) -> Result<Vec<Movie>> {
    let raw: Vec<RawVodStream> = serde_json::from_str(json).map_err(Error::Json)?;
    Ok(raw
        .into_iter()
        .filter_map(|m| {
            let name = m.name.unwrap_or_default();
            Some(Movie {
                id: StreamId(m.stream_id?),
                year: m
                    .year
                    .as_deref()
                    .and_then(year_in)
                    .or_else(|| year_ending(&name)),
                name,
                category_id: m.category_id.map(CategoryId),
                poster: m.stream_icon,
                rating: m.rating.or(m.rating_5based.map(|r| r * 2.0)),
                added: m.added,
                extension: m
                    .container_extension
                    .unwrap_or_else(|| DEFAULT_EXTENSION.to_owned()),
            })
        })
        .collect())
}

/// Parses the body of [`Action::Series`][super::Action::Series], dropping
/// entries without a series id.
///
/// # Errors
///
/// Returns [`Error::Json`] when the body is not a JSON array of objects.
pub fn parse_shows(json: &str) -> Result<Vec<Show>> {
    let raw: Vec<RawSeries> = serde_json::from_str(json).map_err(Error::Json)?;
    Ok(raw
        .into_iter()
        .filter_map(|s| {
            let name = s.name.unwrap_or_default();
            Some(Show {
                id: SeriesId(s.series_id?),
                year: [s.year.as_deref(), s.release_date.as_deref()]
                    .into_iter()
                    .flatten()
                    .find_map(year_in)
                    .or_else(|| year_ending(&name)),
                name,
                category_id: s.category_id.map(CategoryId),
                poster: s.cover,
                rating: s.rating.or(s.rating_5based.map(|r| r * 2.0)),
                updated: s.last_modified,
            })
        })
        .collect())
}

/// Parses the body of [`Action::MovieInfo`][super::Action::MovieInfo].
///
/// # Errors
///
/// Returns [`Error::Json`] when the body is not a JSON object.
pub fn parse_movie_info(json: &str) -> Result<MovieInfo> {
    let raw: RawVodInfo = serde_json::from_str(json).map_err(Error::Json)?;
    let details = raw.info.map_or_else(Details::default, |i| Details {
        plot: i.plot.unwrap_or_default(),
        cast: i.cast.unwrap_or_default(),
        director: i.director.unwrap_or_default(),
        genre: i.genre.unwrap_or_default(),
        released: i.releasedate.unwrap_or_default(),
        duration: seconds(i.duration_secs, i.duration.as_deref()),
        rating: i.rating,
        backdrop: i.backdrop_path,
        poster: i.cover_big.or(i.movie_image),
    });
    Ok(MovieInfo {
        details,
        extension: raw.movie_data.and_then(|d| d.container_extension),
    })
}

/// Parses the body of [`Action::ShowInfo`][super::Action::ShowInfo].
///
/// # Errors
///
/// Returns [`Error::Json`] when the body is not a JSON object.
pub fn parse_show_info(json: &str) -> Result<ShowInfo> {
    let raw: RawSeriesInfo = serde_json::from_str(json).map_err(Error::Json)?;
    let mut episodes: Vec<Episode> = raw.episodes.into_iter().filter_map(episode).collect();
    episodes.sort_by_key(|e| (e.season, e.number));

    let mut seasons: Vec<Season> = raw
        .seasons
        .into_iter()
        .filter_map(|s| {
            let number = u32::try_from(s.season_number?).ok()?;
            Some(Season {
                number,
                name: s.name.unwrap_or_else(|| season_name(number)),
                overview: s.overview.unwrap_or_default(),
                cover: s.cover,
            })
        })
        .collect();
    for e in &episodes {
        if !seasons.iter().any(|s| s.number == e.season) {
            seasons.push(Season {
                number: e.season,
                name: season_name(e.season),
                overview: String::new(),
                cover: None,
            });
        }
    }
    seasons.sort_by_key(|s| s.number);
    seasons.dedup_by_key(|s| s.number);

    let details = raw.info.map_or_else(Details::default, |i| Details {
        plot: i.plot.unwrap_or_default(),
        cast: i.cast.unwrap_or_default(),
        director: i.director.unwrap_or_default(),
        genre: i.genre.unwrap_or_default(),
        released: i.release_date.unwrap_or_default(),
        duration: i
            .episode_run_time
            .and_then(|m| m.trim().parse::<u32>().ok())
            .filter(|&m| m > 0)
            .map(|m| m * 60),
        rating: i.rating.or(i.rating_5based.map(|r| r * 2.0)),
        backdrop: i.backdrop_path,
        poster: i.cover,
    });
    Ok(ShowInfo {
        details,
        seasons,
        episodes,
    })
}

fn episode(e: RawEpisode) -> Option<Episode> {
    let info = e.info;
    let (still, plot, released, duration) = match info {
        Some(i) => (
            i.movie_image,
            i.plot.unwrap_or_default(),
            i.releasedate.unwrap_or_default(),
            seconds(i.duration_secs, i.duration.as_deref()),
        ),
        None => (None, String::new(), String::new(), None),
    };
    let number = e
        .episode_num
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    Some(Episode {
        id: EpisodeId(e.id?),
        season: e.season.and_then(|n| u32::try_from(n).ok()).unwrap_or(1),
        title: e.title.unwrap_or_else(|| format!("Episode {number}")),
        number,
        extension: e
            .container_extension
            .unwrap_or_else(|| DEFAULT_EXTENSION.to_owned()),
        plot,
        duration,
        still,
        released,
    })
}

fn season_name(number: u32) -> String {
    match number {
        0 => "Specials".to_owned(),
        n => format!("Season {n}"),
    }
}

/// Seconds from `duration_secs`, or from a `01:42:10` duration.
fn seconds(secs: Option<u64>, duration: Option<&str>) -> Option<u32> {
    secs.filter(|&s| s > 0)
        .and_then(|s| u32::try_from(s).ok())
        .or_else(|| {
            let parts: Vec<u32> = duration?
                .split(':')
                .map(|p| p.trim().parse().ok())
                .collect::<Option<_>>()?;
            let total = parts.iter().fold(0, |acc, p| acc * 60 + p);
            (total > 0).then_some(total)
        })
}

/// The year at the start of a year or date, for example `2024-05-01`.
fn year_in(text: &str) -> Option<u16> {
    let year: u16 = text.trim().get(..4)?.parse().ok()?;
    (1900..=2100).contains(&year).then_some(year)
}

/// The year in a name ending with `(2024)`.
fn year_ending(name: &str) -> Option<u16> {
    let inner = name.trim_end().strip_suffix(')')?.rsplit_once('(')?.1;
    if inner.len() == 4 {
        year_in(inner)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movies_accept_provider_quirks() {
        let json = r#"[
            {"num":1,"name":"Cottontail (2026)","stream_type":"movie","stream_id":"501",
             "stream_icon":"http://img/501.jpg","rating":"7.4","added":"1700000000",
             "category_id":"10","container_extension":"mkv"},
            {"name":"Red Fury","stream_id":502,"rating":0,"rating_5based":"3.5",
             "year":"2025","stream_icon":"","container_extension":null},
            {"name":"No id","stream_id":null}
        ]"#;
        let movies = parse_movies(json).unwrap();
        assert_eq!(movies.len(), 2);
        let one = &movies[0];
        assert_eq!((one.id, one.year), (StreamId(501), Some(2026)));
        assert_eq!(one.rating, Some(7.4));
        assert_eq!(one.extension, "mkv");
        assert_eq!(one.category_id, Some(CategoryId("10".into())));
        let two = &movies[1];
        assert_eq!(two.rating, Some(7.0), "5-based rating doubled");
        assert_eq!(two.year, Some(2025));
        assert_eq!(
            (two.poster.as_deref(), two.extension.as_str()),
            (None, "mp4")
        );
        assert!(matches!(parse_movies("{}"), Err(Error::Json(_))));
    }

    #[test]
    fn shows_read_year_from_release_date() {
        let json = r#"[{"series_id":"77","name":"Small Thieves","cover":"http://img/77.jpg",
            "releaseDate":"2024-03-01","rating":"7.8","last_modified":"1700000500",
            "backdrop_path":["http://img/77b.jpg"],"category_id":3},
            {"series_id":78,"name":"Old Show (1999)"}]"#;
        let shows = parse_shows(json).unwrap();
        assert_eq!(shows[0].id, SeriesId(77));
        assert_eq!(shows[0].year, Some(2024));
        assert_eq!(shows[0].updated, Some(1_700_000_500));
        assert_eq!(shows[0].category_id, Some(CategoryId("3".into())));
        assert_eq!(shows[1].year, Some(1999));
    }

    #[test]
    fn movie_info_handles_empty_and_full_bodies() {
        let json = r#"{"info":{"movie_image":"http://img/s.jpg","backdrop_path":"http://img/b.jpg",
            "plot":"A rabbit.","actors":"Ana Dimas","director":"Teo Larsen","genre":"Comedy",
            "releasedate":"2026-01-02","duration_secs":0,"duration":"01:34:05","rating":"7.4"},
            "movie_data":{"stream_id":501,"container_extension":"mkv"}}"#;
        let info = parse_movie_info(json).unwrap();
        assert_eq!(info.details.duration, Some(5645));
        assert_eq!(info.details.cast, "Ana Dimas");
        assert_eq!(info.details.backdrop.as_deref(), Some("http://img/b.jpg"));
        assert_eq!(info.details.poster.as_deref(), Some("http://img/s.jpg"));
        assert_eq!(info.extension.as_deref(), Some("mkv"));

        let empty = parse_movie_info(r#"{"info":[],"movie_data":[]}"#).unwrap();
        assert_eq!(empty, MovieInfo::default());
    }

    #[test]
    fn show_info_accepts_keyed_and_listed_episodes() {
        let keyed = r#"{"seasons":[{"season_number":"2","name":"Season Two","cover":"http://c/2.jpg"}],
            "info":{"name":"Small Thieves","plot":"Rodents.","episode_run_time":"24",
                    "backdrop_path":[],"rating_5based":4},
            "episodes":{"2":[{"id":"9002","episode_num":"2","title":"The Orchard Job",
                              "container_extension":"mp4","info":{"duration_secs":"1500",
                              "movie_image":"http://s/9002.jpg","plot":"One apple."}},
                             {"id":"9001","episode_num":1,"title":"Nuts About You","info":[]}],
                        "1":[{"id":"8001","episode_num":1,"season":1,"title":"Pilot"}]}}"#;
        let info = parse_show_info(keyed).unwrap();
        let seasons: Vec<_> = info
            .seasons
            .iter()
            .map(|s| (s.number, s.name.as_str()))
            .collect();
        assert_eq!(seasons, [(1, "Season 1"), (2, "Season Two")]);
        let order: Vec<_> = info.episodes.iter().map(|e| e.id.0.as_str()).collect();
        assert_eq!(order, ["8001", "9001", "9002"]);
        let orchard = &info.episodes[2];
        assert_eq!((orchard.season, orchard.number), (2, 2));
        assert_eq!(orchard.duration, Some(1500));
        assert_eq!(orchard.still.as_deref(), Some("http://s/9002.jpg"));
        assert_eq!(info.details.duration, Some(24 * 60));
        assert_eq!(info.details.rating, Some(8.0));
        assert_eq!(info.details.backdrop, None);

        let listed = r#"{"seasons":[],"info":[],"episodes":[[{"id":"1","season":"0","episode_num":"1"}],
            [{"id":"2","season":3,"episode_num":4,"title":"Late"}]]}"#;
        let info = parse_show_info(listed).unwrap();
        let seasons: Vec<_> = info.seasons.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(seasons, ["Specials", "Season 3"]);
        assert_eq!(info.episodes[0].title, "Episode 1");
        assert!(matches!(parse_show_info("<html>"), Err(Error::Json(_))));
    }

    #[test]
    fn stream_urls_escape_credentials_and_ids() {
        let creds = Credentials::new("http://host.tv:8080", "al ice", "p/ss").unwrap();
        assert_eq!(
            creds.movie_url(StreamId(501), "mkv"),
            "http://host.tv:8080/movie/al%20ice/p%2Fss/501.mkv"
        );
        assert_eq!(
            creds.episode_url(&EpisodeId("90/02".into()), "mp4"),
            "http://host.tv:8080/series/al%20ice/p%2Fss/90%2F02.mp4"
        );
    }

    #[test]
    fn years_come_from_dates_and_name_endings() {
        assert_eq!(year_in("2024-05-01"), Some(2024));
        assert_eq!(year_in("24"), None);
        assert_eq!(year_in("0000"), None);
        assert_eq!(year_ending("Heat (1995) "), Some(1995));
        assert_eq!(year_ending("Blade Runner 2049"), None);
        assert_eq!(year_ending("Top 10 (Part 2)"), None);
        assert_eq!(seconds(None, Some("42:10")), Some(2530));
        assert_eq!(seconds(Some(0), Some("bad")), None);
    }
}
