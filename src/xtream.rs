//! Xtream Codes player API client: account, live categories, live streams.
//!
//! Fetching returns the raw JSON body so callers can cache it as-is; the
//! `parse_*` functions turn a body into typed values. Providers disagree on
//! types (numbers sent as strings, `null` for empty), so parsing is lenient
//! field by field and drops entries without an id instead of failing.
//!
//! Credentials travel in every request and stream URL. Nothing in this
//! module logs or formats a URL, and [`Credentials`] redacts the password in
//! its `Debug` output.

use std::time::Duration;

use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;

/// Result type for this module.
pub type Result<T = ()> = std::result::Result<T, Error>;

/// Whole-request timeout: large providers send tens of megabytes of JSON.
const TIMEOUT: Duration = Duration::from_secs(60);
/// Body size cap. ureq's 10 MiB default is too small for big channel lists.
const MAX_BODY: u64 = 256 * 1024 * 1024;
/// Sent with every API request; some panels reject an empty user agent.
const USER_AGENT: &str = concat!("itele/", env!("CARGO_PKG_VERSION"));
/// Characters escaped in a URL path segment.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// A provider login: server address, username, and password.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    server: String,
    username: String,
    password: String,
}

/// A blocking client for one provider's player API.
pub struct Client {
    credentials: Credentials,
    agent: ureq::Agent,
}

/// A player API request this client knows how to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Account status, expiry, and connection limits.
    Account,
    /// Live TV categories.
    LiveCategories,
    /// Every live channel, across all categories.
    LiveStreams,
}

/// The provider's view of the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// Account status as the panel reports it, for example `Active`.
    pub status: String,
    /// Expiry as Unix seconds; `None` means the account does not expire.
    pub expires_at: Option<u64>,
    /// Whether this is a trial account.
    pub is_trial: bool,
    /// Streams open on this account right now, when reported.
    pub active_connections: Option<u32>,
    /// Streams the account may open at once, when reported.
    pub max_connections: Option<u32>,
    /// Stream formats the account may use, for example `ts` and `m3u8`.
    pub output_formats: Vec<String>,
    /// The panel's message of the day, when set.
    pub message: Option<String>,
}

/// A provider's id for a live, VOD, or series stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamId(pub u64);

/// A provider's id for a category.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CategoryId(pub String);

/// A group of channels or titles, as the provider names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Category {
    /// The provider's id.
    pub id: CategoryId,
    /// Display name.
    pub name: String,
}

/// One live channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveStream {
    /// The provider's id, used in the stream URL.
    pub id: StreamId,
    /// Channel number, when the provider assigns one.
    pub number: Option<u64>,
    /// Display name.
    pub name: String,
    /// Category the channel is listed under, if any.
    pub category_id: Option<CategoryId>,
    /// Logo URL, if any.
    pub icon: Option<String>,
    /// XMLTV channel id for EPG matching, if any.
    pub epg_channel_id: Option<String>,
    /// Days of catch-up the provider keeps; 0 means none.
    pub archive_days: u32,
}

/// Container format for live streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    /// MPEG transport stream (`.ts`).
    Ts,
    /// HLS playlist (`.m3u8`).
    Hls,
}

/// What can go wrong talking to a provider.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The server address is not usable as an `http(s)://` base URL.
    InvalidServer(String),
    /// The provider rejected the username or password.
    Unauthorized,
    /// The server answered with an HTTP error status.
    Status(u16),
    /// The request failed before a response arrived. The text never
    /// contains the request URL.
    Network(String),
    /// The response was not the JSON the player API documents.
    Json(serde_json::Error),
}

// Public API
impl Credentials {
    /// Builds credentials from what the user typed.
    ///
    /// A missing scheme defaults to `http://`, and trailing slashes are
    /// dropped, so `host:8080/` becomes `http://host:8080`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidServer`] when the address is empty, has a
    /// scheme other than `http` or `https`, or carries a query or fragment.
    pub fn new(
        server: &str,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            server: normalize_server(server)?,
            username: username.into(),
            password: password.into(),
        })
    }

    /// The server's base URL, without a trailing slash.
    pub fn server(&self) -> &str {
        &self.server
    }

    /// The account's username.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The account's password.
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Returns the URL that plays `stream` live in `format`.
    ///
    /// The URL embeds the username and password; never log it.
    pub fn live_url(&self, stream: StreamId, format: OutputFormat) -> String {
        let extension = match format {
            OutputFormat::Ts => "ts",
            OutputFormat::Hls => "m3u8",
        };
        format!(
            "{}/live/{}/{}/{}.{extension}",
            self.server,
            utf8_percent_encode(&self.username, PATH_SEGMENT),
            utf8_percent_encode(&self.password, PATH_SEGMENT),
            stream.0,
        )
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("server", &self.server)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

impl Client {
    /// Creates a client for the provider behind `credentials`.
    pub fn new(credentials: Credentials) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .user_agent(USER_AGENT)
            .build()
            .into();
        Self { credentials, agent }
    }

    /// The credentials this client signs requests with.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// Fetches the raw JSON body for `action`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Status`] for an HTTP error status and
    /// [`Error::Network`] when no response arrives.
    pub fn fetch(&self, action: Action) -> Result<String> {
        let mut request = self
            .agent
            .get(format!("{}/player_api.php", self.credentials.server))
            .query("username", &self.credentials.username)
            .query("password", &self.credentials.password);
        if let Some(name) = action.query() {
            request = request.query("action", name);
        }
        let mut response = request.call().map_err(Error::from_ureq)?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(Error::from_ureq)
    }
}

impl Account {
    /// The live stream format to request: `ts` when allowed (or when the
    /// panel lists nothing), otherwise HLS.
    pub fn preferred_format(&self) -> OutputFormat {
        let allows = |name: &str| self.output_formats.iter().any(|f| f == name);
        if self.output_formats.is_empty() || allows("ts") || !allows("m3u8") {
            OutputFormat::Ts
        } else {
            OutputFormat::Hls
        }
    }
}

/// Parses the body of [`Action::Account`].
///
/// # Errors
///
/// Returns [`Error::Unauthorized`] when the panel reports `auth: 0` or
/// sends no account, and [`Error::Json`] when the body is not JSON.
pub fn parse_account(json: &str) -> Result<Account> {
    let envelope: RawEnvelope = serde_json::from_str(json).map_err(Error::Json)?;
    let info = envelope.user_info.ok_or(Error::Unauthorized)?;
    if info.auth != Some(1) {
        return Err(Error::Unauthorized);
    }
    Ok(Account {
        status: info.status.unwrap_or_default(),
        expires_at: info.exp_date.filter(|&t| t > 0),
        is_trial: info.is_trial == Some(1),
        active_connections: info.active_cons.and_then(|n| u32::try_from(n).ok()),
        max_connections: info.max_connections.and_then(|n| u32::try_from(n).ok()),
        output_formats: info.allowed_output_formats,
        message: info.message,
    })
}

/// Parses the body of [`Action::LiveCategories`], dropping entries without
/// an id.
///
/// # Errors
///
/// Returns [`Error::Json`] when the body is not a JSON array of objects.
pub fn parse_categories(json: &str) -> Result<Vec<Category>> {
    let raw: Vec<RawCategory> = serde_json::from_str(json).map_err(Error::Json)?;
    Ok(raw
        .into_iter()
        .filter_map(|c| {
            Some(Category {
                id: CategoryId(c.category_id?),
                name: c.category_name.unwrap_or_default(),
            })
        })
        .collect())
}

/// Parses the body of [`Action::LiveStreams`], dropping entries without a
/// stream id.
///
/// # Errors
///
/// Returns [`Error::Json`] when the body is not a JSON array of objects.
pub fn parse_live_streams(json: &str) -> Result<Vec<LiveStream>> {
    let raw: Vec<RawLiveStream> = serde_json::from_str(json).map_err(Error::Json)?;
    Ok(raw
        .into_iter()
        .filter_map(|s| {
            Some(LiveStream {
                id: StreamId(s.stream_id?),
                number: s.num,
                name: s.name.unwrap_or_default(),
                category_id: s.category_id.map(CategoryId),
                icon: s.stream_icon,
                epg_channel_id: s.epg_channel_id,
                archive_days: match s.tv_archive {
                    Some(0) | None => 0,
                    Some(_) => s
                        .tv_archive_duration
                        .and_then(|d| u32::try_from(d).ok())
                        .unwrap_or(0),
                },
            })
        })
        .collect())
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Json(e) => Some(e),
            Error::InvalidServer(_)
            | Error::Unauthorized
            | Error::Status(_)
            | Error::Network(_) => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::InvalidServer(server) => {
                write!(
                    f,
                    "server must be an http:// or https:// address, got {server:?}"
                )
            }
            Error::Unauthorized => write!(f, "the provider rejected the username or password"),
            Error::Status(code) => write!(f, "the server answered with HTTP status {code}"),
            Error::Network(reason) => write!(f, "could not reach the server: {reason}"),
            Error::Json(e) => write!(f, "the server sent an unexpected response: {e}"),
        }
    }
}

// Private API
impl Action {
    const fn query(self) -> Option<&'static str> {
        match self {
            Action::Account => None,
            Action::LiveCategories => Some("get_live_categories"),
            Action::LiveStreams => Some("get_live_streams"),
        }
    }
}

impl Error {
    /// Maps a ureq error without keeping anything that may hold the URL.
    fn from_ureq(e: ureq::Error) -> Self {
        let reason = match e {
            ureq::Error::StatusCode(code) => return Error::Status(code),
            ureq::Error::Timeout(_) => "timed out".to_owned(),
            ureq::Error::HostNotFound => "server not found".to_owned(),
            ureq::Error::ConnectionFailed => "connection refused".to_owned(),
            ureq::Error::Io(io) => io.to_string(),
            ureq::Error::BodyExceedsLimit(limit) => format!("response larger than {limit} bytes"),
            ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "secure connection failed".to_owned(),
            ureq::Error::BadUri(_) => "invalid server address".to_owned(),
            _ => "request failed".to_owned(),
        };
        Error::Network(reason)
    }
}

fn normalize_server(input: &str) -> Result<String> {
    let invalid = || Error::InvalidServer(input.to_owned());
    let trimmed = input.trim();
    if trimmed.contains(['?', '#']) || trimmed.contains(char::is_whitespace) {
        return Err(invalid());
    }
    let (scheme, rest) = trimmed.split_once("://").unwrap_or(("http", trimmed));
    let rest = rest.trim_end_matches('/');
    if !matches!(scheme, "http" | "https") || rest.is_empty() {
        return Err(invalid());
    }
    Ok(format!("{scheme}://{rest}"))
}

#[derive(Deserialize)]
struct RawEnvelope {
    #[serde(default)]
    user_info: Option<RawUserInfo>,
}

#[derive(Deserialize)]
struct RawUserInfo {
    #[serde(default, deserialize_with = "lenient::u64")]
    auth: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    status: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    exp_date: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    is_trial: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    active_cons: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    max_connections: Option<u64>,
    #[serde(default, deserialize_with = "lenient::strings")]
    allowed_output_formats: Vec<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    message: Option<String>,
}

#[derive(Deserialize)]
struct RawCategory {
    #[serde(default, deserialize_with = "lenient::string")]
    category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    category_name: Option<String>,
}

#[derive(Deserialize)]
struct RawLiveStream {
    #[serde(default, deserialize_with = "lenient::u64")]
    stream_id: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    num: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    stream_icon: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    epg_channel_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    tv_archive: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    tv_archive_duration: Option<u64>,
}

/// Field deserializers that accept the shapes providers actually send.
mod lenient {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    /// A non-negative integer sent as a number, a numeric string, or a bool;
    /// anything else is `None`.
    pub fn u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse().ok(),
            Value::Bool(b) => Some(u64::from(b)),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        })
    }

    /// Text sent as a string or a number; empty text is `None`.
    pub fn string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(s) if !s.trim().is_empty() => Some(s),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    }

    /// A list of strings; anything that is not a string list is empty.
    pub fn strings<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Array(items) => items
                .into_iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_normalizes() {
        let creds = |s| Credentials::new(s, "u", "p").map(|c| c.server().to_owned());
        assert_eq!(creds("host.tv:8080").unwrap(), "http://host.tv:8080");
        assert_eq!(creds(" https://host.tv/ ").unwrap(), "https://host.tv");
        assert_eq!(
            creds("http://host.tv:80/iptv//").unwrap(),
            "http://host.tv:80/iptv"
        );
    }

    #[test]
    fn server_rejects_bad_input() {
        for bad in [
            "",
            "   ",
            "ftp://host.tv",
            "http://",
            "host.tv/get.php?x=1",
            "a b",
        ] {
            assert!(
                matches!(
                    Credentials::new(bad, "u", "p"),
                    Err(Error::InvalidServer(_))
                ),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn debug_redacts_password() {
        let creds = Credentials::new("host.tv", "alice", "s3cret").unwrap();
        let debug = format!("{creds:?}");
        assert!(
            debug.contains("alice") && !debug.contains("s3cret"),
            "{debug}"
        );
    }

    #[test]
    fn live_url_escapes_credentials() {
        let creds = Credentials::new("http://host.tv:8080", "al ice", "p/ss#1").unwrap();
        assert_eq!(
            creds.live_url(StreamId(42), OutputFormat::Ts),
            "http://host.tv:8080/live/al%20ice/p%2Fss%231/42.ts"
        );
        assert!(
            creds
                .live_url(StreamId(42), OutputFormat::Hls)
                .ends_with("/42.m3u8")
        );
    }

    #[test]
    fn account_parses_strings_as_numbers() {
        let json = r#"{"user_info":{"username":"u","password":"p","message":"",
            "auth":1,"status":"Active","exp_date":"1735689600","is_trial":"0",
            "active_cons":"1","max_connections":"2",
            "allowed_output_formats":["m3u8","ts"]},"server_info":{}}"#;
        let account = parse_account(json).unwrap();
        assert_eq!(account.status, "Active");
        assert_eq!(account.expires_at, Some(1_735_689_600));
        assert!(!account.is_trial);
        assert_eq!(
            (account.active_connections, account.max_connections),
            (Some(1), Some(2))
        );
        assert_eq!(account.message, None);
        assert_eq!(account.preferred_format(), OutputFormat::Ts);
    }

    #[test]
    fn account_without_expiry_and_hls_only() {
        let json = r#"{"user_info":{"auth":"1","status":"Active","exp_date":null,
            "max_connections":1,"allowed_output_formats":["m3u8"]}}"#;
        let account = parse_account(json).unwrap();
        assert_eq!(account.expires_at, None);
        assert_eq!(account.active_connections, None);
        assert_eq!(account.preferred_format(), OutputFormat::Hls);
    }

    #[test]
    fn account_rejects_failed_auth() {
        for json in [
            r#"{"user_info":{"auth":0}}"#,
            r#"{"server_info":{}}"#,
            r#"{}"#,
        ] {
            assert!(
                matches!(parse_account(json), Err(Error::Unauthorized)),
                "{json}"
            );
        }
        assert!(matches!(parse_account("<html>"), Err(Error::Json(_))));
    }

    #[test]
    fn categories_drop_entries_without_id() {
        let json = r#"[{"category_id":"1","category_name":"Sports","parent_id":0},
            {"category_id":7,"category_name":"News"},
            {"category_id":null,"category_name":"Broken"}]"#;
        let categories = parse_categories(json).unwrap();
        assert_eq!(categories.len(), 2);
        assert_eq!(categories[0].id, CategoryId("1".into()));
        assert_eq!(categories[1].id, CategoryId("7".into()));
        assert_eq!(categories[1].name, "News");
        assert!(matches!(parse_categories("{}"), Err(Error::Json(_))));
    }

    #[test]
    fn live_streams_accept_provider_quirks() {
        let json = r#"[
            {"num":1,"name":"One","stream_type":"live","stream_id":123,
             "stream_icon":"http://logo/1.png","epg_channel_id":"one.uk","added":"1609459200",
             "category_id":"1","custom_sid":"","tv_archive":1,"direct_source":"",
             "tv_archive_duration":7},
            {"num":"2","name":"Two","stream_id":"124","stream_icon":"","epg_channel_id":null,
             "category_id":null,"tv_archive":"0","tv_archive_duration":"3"},
            {"num":null,"name":"No id","stream_id":null},
            {"name":"Archive as string","stream_id":125,"tv_archive":"1","tv_archive_duration":"5"}
        ]"#;
        let streams = parse_live_streams(json).unwrap();
        assert_eq!(streams.len(), 3);

        let one = &streams[0];
        assert_eq!((one.id, one.number), (StreamId(123), Some(1)));
        assert_eq!(one.category_id, Some(CategoryId("1".into())));
        assert_eq!(one.icon.as_deref(), Some("http://logo/1.png"));
        assert_eq!(one.epg_channel_id.as_deref(), Some("one.uk"));
        assert_eq!(one.archive_days, 7);

        let two = &streams[1];
        assert_eq!((two.id, two.number), (StreamId(124), Some(2)));
        assert_eq!(
            (two.icon.as_deref(), two.epg_channel_id.as_deref()),
            (None, None)
        );
        assert_eq!(two.category_id, None);
        assert_eq!(two.archive_days, 0, "tv_archive 0 means no catch-up");

        assert_eq!(streams[2].archive_days, 5);
    }
}
