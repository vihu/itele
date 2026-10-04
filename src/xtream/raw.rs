//! The player API's JSON as providers send it, before cleaning up.

use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct RawEnvelope {
    #[serde(default)]
    pub(super) user_info: Option<RawUserInfo>,
    #[serde(default)]
    pub(super) server_info: Option<RawServerInfo>,
}

#[derive(Deserialize)]
pub(super) struct RawServerInfo {
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) timezone: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawUserInfo {
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) auth: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) status: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) exp_date: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) is_trial: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) active_cons: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) max_connections: Option<u64>,
    #[serde(default, deserialize_with = "lenient::strings")]
    pub(super) allowed_output_formats: Vec<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) message: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawCategory {
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) category_name: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawLiveStream {
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) stream_id: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) num: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) name: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) stream_icon: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) epg_channel_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) tv_archive: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) tv_archive_duration: Option<u64>,
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
