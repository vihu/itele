//! Saved providers: settings on disk, the password in the OS keychain, and
//! the provider's JSON cached between runs (`cache.rs`).
//!
//! The account response echoes the password back, so it is stripped before
//! the body reaches the cache. No file this module writes holds a secret.

use std::fs;
use std::io;

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::xtream::{self, Credentials};

mod cache;

pub use cache::{Cache, Library, Listing, Shelf};

/// Result type for this module.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Keychain service name every provider password is stored under.
const KEYCHAIN_SERVICE: &str = "itele";
/// File in the config directory that lists the saved providers.
const PROVIDERS_FILE: &str = "providers.json";
/// File in the config directory that holds the settings.
const SETTINGS_FILE: &str = "settings.json";

/// Where itele keeps its settings, its data and its cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    config: Utf8PathBuf,
    data: Utf8PathBuf,
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
    /// Hours added to this provider's guide times, for guides that are
    /// off by an hour or two.
    #[serde(default)]
    pub guide_shift: i32,
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
    /// The platform's standard locations, for example `~/.config/itele`,
    /// `~/.local/share/itele` and `~/.cache/itele` on Linux.
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
            data: utf8(dirs.data_dir())?,
            cache: utf8(dirs.cache_dir())?,
        })
    }

    /// Settings under `root/config`, data under `root/data` and the cache
    /// under `root/cache`.
    pub fn under(root: &Utf8Path) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
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

    /// Loads the settings; none saved yet is the defaults.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the file exists but cannot be read, and
    /// [`Error::Config`] when it is not valid JSON.
    pub fn load_settings(&self) -> Result<Settings> {
        let path = self.config.join(SETTINGS_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| Error::Config(path, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(Error::Io(path, e)),
        }
    }

    /// Saves the settings, replacing the file atomically.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the config directory cannot be written.
    pub fn save_settings(&self, settings: &Settings) -> Result {
        let json = serde_json::to_string_pretty(settings).expect("settings always serialize");
        write_atomic(&self.config.join(SETTINGS_FILE), &json)
    }

    /// The settings directory.
    pub fn config_dir(&self) -> &Utf8Path {
        &self.config
    }

    /// The data directory, which holds the watch history.
    pub fn data_dir(&self) -> &Utf8Path {
        &self.data
    }

    /// The cache directory: lists, posters, logos and the guide.
    pub fn cache_dir(&self) -> &Utf8Path {
        &self.cache
    }

    /// Deletes everything in the cache directory but the guide store,
    /// which stays open while itele runs (empty it through the store).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when something cannot be deleted.
    pub fn clear_cache(&self) -> Result {
        let entries = match fs::read_dir(&self.cache) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(Error::Io(self.cache.clone(), e)),
        };
        let guide = self.guide_path();
        for entry in entries {
            let entry = entry.map_err(|e| Error::Io(self.cache.clone(), e))?;
            let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) else {
                continue;
            };
            if path.as_str().starts_with(guide.as_str()) {
                continue;
            }
            let removed = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            removed.map_err(|e| Error::Io(path, e))?;
        }
        Ok(())
    }

    /// The watch history store, shared by all providers. It is data, not
    /// cache: clearing the cache keeps it.
    pub fn history_path(&self) -> Utf8PathBuf {
        self.data.join("history.sqlite")
    }

    /// The programme guide store, shared by all providers.
    pub fn guide_path(&self) -> Utf8PathBuf {
        self.cache.join("guide.sqlite")
    }

    /// Where downloaded channel logos are kept, shared by all providers.
    pub fn logos_dir(&self) -> Utf8PathBuf {
        self.cache.join("logos")
    }

    /// Where downloaded posters and backdrops are kept, shared by all
    /// providers.
    pub fn art_dir(&self) -> Utf8PathBuf {
        self.cache.join("art")
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
            guide_shift: 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::xtream::Action;

    const CATEGORIES: &str = r#"[{"category_id":"1","category_name":"News"}]"#;

    pub(super) fn temp_paths() -> (tempfile::TempDir, Paths) {
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
    fn settings_save_and_load_and_default_when_missing() {
        let (_dir, paths) = temp_paths();
        assert_eq!(paths.load_settings().unwrap(), Settings::default());
        let settings = Settings {
            keep_days: 3,
            ..Settings::default()
        };
        paths.save_settings(&settings).unwrap();
        assert_eq!(paths.load_settings().unwrap(), settings);
    }

    #[test]
    fn providers_from_before_the_guide_shift_load() {
        let (_dir, paths) = temp_paths();
        write_atomic(
            &paths.config.join(PROVIDERS_FILE),
            r#"[{"id":"a","name":"A","server":"http://a","username":"u"}]"#,
        )
        .unwrap();
        assert_eq!(paths.load_providers().unwrap()[0].guide_shift, 0);
    }

    #[test]
    fn clear_cache_keeps_the_guide_and_the_rest_of_the_data() {
        let (_dir, paths) = temp_paths();
        paths.clear_cache().unwrap();
        let cache = paths.cache(&provider());
        cache.write(Action::LiveCategories, CATEGORIES).unwrap();
        fs::create_dir_all(paths.art_dir()).unwrap();
        fs::write(paths.art_dir().join("x.png"), b"x").unwrap();
        fs::write(paths.guide_path(), b"guide").unwrap();
        fs::write(paths.guide_path().with_extension("sqlite-wal"), b"wal").unwrap();
        paths.save_settings(&Settings::default()).unwrap();

        paths.clear_cache().unwrap();
        assert!(cache.read(Action::LiveCategories).is_none());
        assert!(!paths.art_dir().exists());
        assert!(paths.guide_path().exists());
        assert!(paths.guide_path().with_extension("sqlite-wal").exists());
        assert_eq!(paths.load_settings().unwrap(), Settings::default());
    }

    #[test]
    fn damaged_providers_file_is_an_error() {
        let (_dir, paths) = temp_paths();
        write_atomic(&paths.config.join(PROVIDERS_FILE), "{not json").unwrap();
        assert!(matches!(paths.load_providers(), Err(Error::Config(..))));
    }
}
