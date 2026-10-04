//! Saved providers: settings on disk, the password in the OS keychain, and
//! the provider's JSON cached between runs.
//!
//! The account response echoes the password back, so it is stripped before
//! the body reaches the cache. No file this module writes holds a secret.

use std::fs;
use std::io;
use std::time::{Duration, SystemTime};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::xtream::{self, Account, Action, Category, Client, Credentials, LiveStream};

/// Result type for this module.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Keychain service name every provider password is stored under.
const KEYCHAIN_SERVICE: &str = "itele";
/// File in the config directory that lists the saved providers.
const PROVIDERS_FILE: &str = "providers.json";

/// Where itele keeps its settings and its cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    config: Utf8PathBuf,
    cache: Utf8PathBuf,
}

/// A saved provider. The password lives in the OS keychain, never here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// Stable id, used for the keychain entry and the cache directory.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Server base URL, as normalized by [`Credentials::new`].
    pub server: String,
    /// Account username.
    pub username: String,
}

/// One provider's cached API responses.
#[derive(Clone, Debug)]
pub struct Cache {
    dir: Utf8PathBuf,
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

/// What can go wrong loading or saving providers.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The OS reports no home directory to put settings in.
    NoHome,
    /// Reading or writing a file failed.
    Io(Utf8PathBuf, io::Error),
    /// A settings file is not valid JSON.
    Config(Utf8PathBuf, serde_json::Error),
    /// The OS keychain refused to store or return the password.
    Keychain(keyring::Error),
    /// The provider could not be reached or rejected the login.
    Xtream(xtream::Error),
}

// Public API
impl Paths {
    /// The platform's standard locations, for example `~/.config/itele` and
    /// `~/.cache/itele` on Linux.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoHome`] when the OS reports no home directory.
    pub fn system() -> Result<Self> {
        let dirs =
            directories::ProjectDirs::from("dev", "MotleyCode", "itele").ok_or(Error::NoHome)?;
        let utf8 = |p: &std::path::Path| {
            Utf8PathBuf::from_path_buf(p.to_owned()).map_err(|_| Error::NoHome)
        };
        Ok(Self {
            config: utf8(dirs.config_dir())?,
            cache: utf8(dirs.cache_dir())?,
        })
    }

    /// Settings under `root/config` and the cache under `root/cache`.
    pub fn under(root: &Utf8Path) -> Self {
        Self {
            config: root.join("config"),
            cache: root.join("cache"),
        }
    }

    /// Loads the saved providers; none saved yet is an empty list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the file exists but cannot be read, and
    /// [`Error::Config`] when it is not valid JSON.
    pub fn load_providers(&self) -> Result<Vec<Provider>> {
        let path = self.config.join(PROVIDERS_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| Error::Config(path, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(Error::Io(path, e)),
        }
    }

    /// Saves the provider list, replacing the file atomically.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the config directory cannot be written.
    pub fn save_providers(&self, providers: &[Provider]) -> Result {
        let json = serde_json::to_string_pretty(providers).expect("providers always serialize");
        write_atomic(&self.config.join(PROVIDERS_FILE), &json)
    }

    /// Saves `provider` in the list: replaces the entry with the same id in
    /// place, or appends it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] or [`Error::Config`] when the provider list
    /// cannot be read or written.
    pub fn add_provider(&self, provider: &Provider) -> Result {
        let mut providers = self.load_providers()?;
        match providers.iter_mut().find(|p| p.id == provider.id) {
            Some(slot) => *slot = provider.clone(),
            None => providers.push(provider.clone()),
        }
        self.save_providers(&providers)
    }

    /// Removes `provider` from the saved list and deletes its cache. The
    /// keychain entry is separate: see [`Provider::forget_password`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] or [`Error::Config`] when the provider list
    /// cannot be read or written, or the cache cannot be deleted.
    pub fn remove_provider(&self, provider: &Provider) -> Result {
        let mut providers = self.load_providers()?;
        providers.retain(|p| p.id != provider.id);
        self.save_providers(&providers)?;
        let dir = self.cache(provider).dir;
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(dir, e)),
        }
    }

    /// The cache for `provider`.
    pub fn cache(&self, provider: &Provider) -> Cache {
        Cache {
            dir: self.cache.join(&provider.id),
        }
    }
}

impl Provider {
    /// Describes a new provider from validated credentials.
    pub fn new(name: impl Into<String>, credentials: &Credentials) -> Self {
        let id = format!("{}-{}", credentials.server(), credentials.username())
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        Self {
            id,
            name: name.into(),
            server: credentials.server().to_owned(),
            username: credentials.username().to_owned(),
        }
    }

    /// Stores `password` in the OS keychain for this provider.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Keychain`] when the keychain is locked or missing.
    pub fn save_password(&self, password: &str) -> Result {
        self.keychain_entry()?
            .set_password(password)
            .map_err(Error::Keychain)
    }

    /// Deletes this provider's password from the OS keychain. A missing
    /// entry is not an error.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Keychain`] when the keychain is locked or missing.
    pub fn forget_password(&self) -> Result {
        match self.keychain_entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Keychain(e)),
        }
    }

    /// Builds credentials with the password from the OS keychain.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Keychain`] when no password is stored or the
    /// keychain is unavailable, and [`Error::Xtream`] when the saved server
    /// address is no longer valid.
    pub fn credentials(&self) -> Result<Credentials> {
        let password = self
            .keychain_entry()?
            .get_password()
            .map_err(Error::Keychain)?;
        Credentials::new(&self.server, &self.username, password).map_err(Error::Xtream)
    }
}

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

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::NoHome => None,
            Error::Io(_, e) => Some(e),
            Error::Config(_, e) => Some(e),
            Error::Keychain(e) => Some(e),
            Error::Xtream(e) => Some(e),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::NoHome => write!(f, "the system reports no home directory for settings"),
            Error::Io(path, e) => write!(f, "could not access {path}: {e}"),
            Error::Config(path, e) => write!(f, "settings file {path} is damaged: {e}"),
            Error::Keychain(e) => write!(f, "the system keychain failed: {e}"),
            Error::Xtream(e) => write!(f, "{e}"),
        }
    }
}

// Private API
impl Provider {
    fn keychain_entry(&self) -> Result<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, &self.id).map_err(Error::Keychain)
    }
}

impl Cache {
    fn path(&self, action: Action) -> Utf8PathBuf {
        let name = match action {
            Action::Account => "account.json",
            Action::LiveCategories => "live_categories.json",
            Action::LiveStreams => "live_streams.json",
        };
        self.dir.join(name)
    }
}

/// Writes through a temporary file and renames it over `path`, so a crash
/// never leaves a half-written file behind.
fn write_atomic(path: &Utf8Path, contents: &str) -> Result {
    let io_err = |e| Error::Io(path.to_owned(), e);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents).map_err(io_err)?;
    fs::rename(&tmp, path).map_err(io_err)
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

    const ACCOUNT: &str = r#"{"user_info":{"username":"alice","password":"s3cret","auth":1,
        "status":"Active","exp_date":"1735689600","max_connections":"2"},"server_info":{}}"#;
    const CATEGORIES: &str = r#"[{"category_id":"1","category_name":"News"}]"#;
    const STREAMS: &str = r#"[{"num":1,"name":"One","stream_id":7,"category_id":"1"}]"#;

    fn temp_paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(dir.path().to_owned()).unwrap();
        let paths = Paths::under(&root);
        (dir, paths)
    }

    fn provider() -> Provider {
        Provider::new(
            "Northwind",
            &Credentials::new("http://tv.example:8080", "alice", "s3cret").unwrap(),
        )
    }

    #[test]
    fn provider_id_is_filename_safe() {
        assert_eq!(provider().id, "tv.example_8080-alice");
    }

    #[test]
    fn providers_round_trip_without_password() {
        let (_dir, paths) = temp_paths();
        assert_eq!(paths.load_providers().unwrap(), Vec::new());
        paths.save_providers(&[provider()]).unwrap();
        assert_eq!(paths.load_providers().unwrap(), vec![provider()]);
        let text = fs::read_to_string(paths.config.join(PROVIDERS_FILE)).unwrap();
        assert!(!text.contains("s3cret"), "{text}");
    }

    #[test]
    fn add_provider_replaces_by_id_or_appends() {
        let (_dir, paths) = temp_paths();
        let other = Provider::new("Other", &Credentials::new("other.tv", "bob", "x").unwrap());
        paths.add_provider(&provider()).unwrap();
        paths.add_provider(&other).unwrap();
        let renamed = Provider {
            name: "Renamed".into(),
            ..provider()
        };
        paths.add_provider(&renamed).unwrap();
        assert_eq!(paths.load_providers().unwrap(), vec![renamed, other]);
    }

    #[test]
    fn remove_provider_drops_entry_and_cache() {
        let (_dir, paths) = temp_paths();
        let other = Provider::new("Other", &Credentials::new("other.tv", "bob", "x").unwrap());
        paths.save_providers(&[provider(), other.clone()]).unwrap();
        let cache = paths.cache(&provider());
        cache.write(Action::LiveCategories, CATEGORIES).unwrap();

        paths.remove_provider(&provider()).unwrap();
        assert_eq!(paths.load_providers().unwrap(), vec![other]);
        assert!(cache.read(Action::LiveCategories).is_none());
        paths.remove_provider(&provider()).unwrap();
    }

    #[test]
    fn damaged_providers_file_is_an_error() {
        let (_dir, paths) = temp_paths();
        write_atomic(&paths.config.join(PROVIDERS_FILE), "{not json").unwrap();
        assert!(matches!(paths.load_providers(), Err(Error::Config(..))));
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
    fn cached_account_has_no_password() {
        let stripped = strip_password(ACCOUNT);
        assert!(!stripped.contains("s3cret"), "{stripped}");
        assert_eq!(xtream::parse_account(&stripped).unwrap().status, "Active");
        assert_eq!(strip_password("not json"), "not json");
    }
}
