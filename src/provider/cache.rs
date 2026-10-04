//! One provider's cached API responses, and what is built from them: the
//! live library and the movie and series shelves.

use std::fs;
use std::time::{Duration, SystemTime};

use camino::Utf8PathBuf;

use super::{Error, Result, write_atomic};
use crate::xtream::{self, Account, Action, Category, Client, LiveStream, Movie, Show};

/// One provider's cached API responses.
#[derive(Clone, Debug)]
pub struct Cache {
    pub(super) dir: Utf8PathBuf,
}

/// Everything the live TV screens need from a provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    /// Account status and limits.
    pub account: Account,
    /// Live categories in provider order.
    pub categories: Vec<Category>,
    /// Live channels in provider order.
    pub streams: Vec<LiveStream>,
}

/// One provider's movies or series.
#[derive(Clone, Debug, PartialEq)]
pub struct Shelf<T> {
    /// Categories in provider order.
    pub categories: Vec<Category>,
    /// Titles in provider order.
    pub titles: Vec<T>,
}

/// What a [`Shelf`] lists: [`Movie`] or [`Show`].
pub trait Listing: Sized {
    /// The request for the categories.
    const CATEGORIES: Action;
    /// The request for the titles.
    const TITLES: Action;

    /// Parses the body of [`Self::TITLES`].
    ///
    /// # Errors
    ///
    /// Returns an error when the body is not the expected JSON.
    fn parse(json: &str) -> xtream::Result<Vec<Self>>;
}

// Public API
impl Cache {
    /// The cached body for `action`, if one was saved.
    pub fn read(&self, action: Action) -> Option<String> {
        fs::read_to_string(self.path(action)).ok()
    }

    /// How long ago the body for `action` was saved.
    pub fn age(&self, action: Action) -> Option<Duration> {
        let modified = fs::metadata(self.path(action)).ok()?.modified().ok()?;
        SystemTime::now().duration_since(modified).ok()
    }

    /// Saves the body for `action`, replacing the old one atomically.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the cache directory cannot be written.
    pub fn write(&self, action: Action, body: &str) -> Result {
        write_atomic(&self.path(action), body)
    }

    /// The cached body for `action`, parsed; `None` when missing or no
    /// longer parseable.
    pub fn load<T>(&self, action: Action, parse: impl Fn(&str) -> xtream::Result<T>) -> Option<T> {
        parse(&self.read(action)?).ok()
    }

    /// Fetches `action` from the provider and parses it, caching the body
    /// once it parses, so a broken response never replaces a good one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Xtream`] when the request fails or the body does not
    /// parse, and [`Error::Io`] when the cache cannot be written.
    pub fn refresh<T>(
        &self,
        client: &Client,
        action: Action,
        parse: impl Fn(&str) -> xtream::Result<T>,
    ) -> Result<T> {
        let body = client.fetch(action).map_err(Error::Xtream)?;
        let parsed = parse(&body).map_err(Error::Xtream)?;
        self.write(action, &body)?;
        Ok(parsed)
    }
}

impl Library {
    /// Builds the library from the cache alone, for an instant start.
    /// `None` when any part is missing or no longer parses.
    pub fn from_cache(cache: &Cache) -> Option<Self> {
        Some(Self {
            account: xtream::parse_account(&cache.read(Action::Account)?).ok()?,
            categories: xtream::parse_categories(&cache.read(Action::LiveCategories)?).ok()?,
            streams: xtream::parse_live_streams(&cache.read(Action::LiveStreams)?).ok()?,
        })
    }

    /// Fetches only the account, for a current status, expiry and
    /// connection count, keeping `self`'s categories and channels.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Xtream`] when the request fails or the login is
    /// rejected, and [`Error::Io`] when the cache cannot be written.
    pub fn with_fresh_account(self, client: &Client, cache: &Cache) -> Result<Self> {
        let body = client.fetch(Action::Account).map_err(Error::Xtream)?;
        let account = xtream::parse_account(&body).map_err(Error::Xtream)?;
        cache.write(Action::Account, &strip_password(&body))?;
        Ok(Self { account, ..self })
    }

    /// Fetches the library from the provider and refreshes the cache.
    ///
    /// The account is checked first, so a rejected login fails before the
    /// large channel list is downloaded.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Xtream`] when a request fails or the login is
    /// rejected, and [`Error::Io`] when the cache cannot be written.
    pub fn fetch(client: &Client, cache: &Cache) -> Result<Self> {
        let account_body = client.fetch(Action::Account).map_err(Error::Xtream)?;
        let account = xtream::parse_account(&account_body).map_err(Error::Xtream)?;
        let categories_body = client
            .fetch(Action::LiveCategories)
            .map_err(Error::Xtream)?;
        let categories = xtream::parse_categories(&categories_body).map_err(Error::Xtream)?;
        let streams_body = client.fetch(Action::LiveStreams).map_err(Error::Xtream)?;
        let streams = xtream::parse_live_streams(&streams_body).map_err(Error::Xtream)?;

        cache.write(Action::Account, &strip_password(&account_body))?;
        cache.write(Action::LiveCategories, &categories_body)?;
        cache.write(Action::LiveStreams, &streams_body)?;
        Ok(Self {
            account,
            categories,
            streams,
        })
    }
}

impl<T: Listing> Shelf<T> {
    /// Builds the shelf from the cache alone; `None` when any part is
    /// missing or no longer parses.
    pub fn from_cache(cache: &Cache) -> Option<Self> {
        Some(Self {
            categories: cache.load(T::CATEGORIES, xtream::parse_categories)?,
            titles: cache.load(T::TITLES, T::parse)?,
        })
    }

    /// Fetches the shelf from the provider and refreshes the cache.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Xtream`] when a request fails, and [`Error::Io`]
    /// when the cache cannot be written.
    pub fn fetch(client: &Client, cache: &Cache) -> Result<Self> {
        Ok(Self {
            categories: cache.refresh(client, T::CATEGORIES, xtream::parse_categories)?,
            titles: cache.refresh(client, T::TITLES, T::parse)?,
        })
    }
}

impl Listing for Movie {
    const CATEGORIES: Action = Action::VodCategories;
    const TITLES: Action = Action::VodStreams;

    fn parse(json: &str) -> xtream::Result<Vec<Self>> {
        xtream::parse_movies(json)
    }
}

impl Listing for Show {
    const CATEGORIES: Action = Action::SeriesCategories;
    const TITLES: Action = Action::Series;

    fn parse(json: &str) -> xtream::Result<Vec<Self>> {
        xtream::parse_shows(json)
    }
}

// Private API
impl Cache {
    fn path(&self, action: Action) -> Utf8PathBuf {
        let name = match action {
            Action::Account => "account.json".to_owned(),
            Action::LiveCategories => "live_categories.json".to_owned(),
            Action::LiveStreams => "live_streams.json".to_owned(),
            Action::VodCategories => "vod_categories.json".to_owned(),
            Action::VodStreams => "vod_streams.json".to_owned(),
            Action::SeriesCategories => "series_categories.json".to_owned(),
            Action::Series => "series.json".to_owned(),
            Action::MovieInfo(id) => format!("movie_info/{}.json", id.0),
            Action::ShowInfo(id) => format!("series_info/{}.json", id.0),
        };
        self.dir.join(name)
    }
}

/// Removes `user_info.password` from an account body; other bodies and
/// unparseable text pass through unchanged.
fn strip_password(account_body: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(account_body) else {
        return account_body.to_owned();
    };
    if let Some(info) = value.get_mut("user_info").and_then(|v| v.as_object_mut()) {
        info.remove("password");
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;
    use crate::provider::tests::temp_paths;
    use crate::xtream::Credentials;

    const ACCOUNT: &str = r#"{"user_info":{"username":"alice","password":"s3cret","auth":1,
        "status":"Active","exp_date":"1735689600","max_connections":"2"},"server_info":{}}"#;
    const CATEGORIES: &str = r#"[{"category_id":"1","category_name":"News"}]"#;
    const STREAMS: &str = r#"[{"num":1,"name":"One","stream_id":7,"category_id":"1"}]"#;

    fn provider() -> Provider {
        Provider::new(
            "Northwind",
            &Credentials::new("http://tv.example:8080", "alice", "s3cret").unwrap(),
        )
    }

    #[test]
    fn library_loads_from_cache() {
        let (_dir, paths) = temp_paths();
        let cache = paths.cache(&provider());
        assert!(Library::from_cache(&cache).is_none());
        cache
            .write(Action::Account, &strip_password(ACCOUNT))
            .unwrap();
        cache.write(Action::LiveCategories, CATEGORIES).unwrap();
        assert!(
            Library::from_cache(&cache).is_none(),
            "streams still missing"
        );
        cache.write(Action::LiveStreams, STREAMS).unwrap();

        let library = Library::from_cache(&cache).unwrap();
        assert_eq!(library.account.max_connections, Some(2));
        assert_eq!(library.categories[0].name, "News");
        assert_eq!(library.streams[0].name, "One");
        assert!(cache.age(Action::LiveStreams).unwrap() < Duration::from_secs(60));
    }

    #[test]
    fn shelf_round_trips_through_the_cache() {
        let (_dir, paths) = temp_paths();
        let cache = paths.cache(&provider());
        assert!(Shelf::<Movie>::from_cache(&cache).is_none());
        cache.write(Action::VodCategories, CATEGORIES).unwrap();
        cache
            .write(
                Action::VodStreams,
                r#"[{"name":"Heat (1995)","stream_id":9,"category_id":"1"}]"#,
            )
            .unwrap();
        let movies = Shelf::<Movie>::from_cache(&cache).unwrap();
        assert_eq!(movies.titles[0].year, Some(1995));
        assert!(
            Shelf::<Show>::from_cache(&cache).is_none(),
            "series are cached apart"
        );

        let info = xtream::MovieInfo::default();
        assert_eq!(
            cache.load(Action::MovieInfo(xtream::StreamId(9)), |_| Ok(info.clone())),
            None,
            "nothing cached for this title yet"
        );
        cache
            .write(Action::MovieInfo(xtream::StreamId(9)), "{}")
            .unwrap();
        assert!(
            cache
                .load(
                    Action::MovieInfo(xtream::StreamId(9)),
                    xtream::parse_movie_info
                )
                .is_some()
        );
    }

    #[test]
    fn cached_account_has_no_password() {
        let stripped = strip_password(ACCOUNT);
        assert!(!stripped.contains("s3cret"), "{stripped}");
        assert_eq!(xtream::parse_account(&stripped).unwrap().status, "Active");
        assert_eq!(strip_password("not json"), "not json");
    }
}
