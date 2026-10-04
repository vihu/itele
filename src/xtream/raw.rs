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

#[derive(Deserialize)]
pub(super) struct RawVodStream {
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) stream_id: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) name: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) stream_icon: Option<String>,
    #[serde(default, deserialize_with = "lenient::f32")]
    pub(super) rating: Option<f32>,
    #[serde(default, deserialize_with = "lenient::f32")]
    pub(super) rating_5based: Option<f32>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) added: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) container_extension: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) year: Option<String>,
}

/// A series in `get_series`, and the `info` of `get_series_info`.
///
/// Panels often send one value under several names at once (`releaseDate`
/// and `release_date`). Serde rejects two aliases of one field in the same
/// object, so each name is a field of its own here; the parsers take the
/// first one set.
#[derive(Deserialize)]
pub(super) struct RawSeries {
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) series_id: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) name: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) category_id: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) cover: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) plot: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) cast: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) director: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) genre: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) release_date: Option<String>,
    #[serde(default, rename = "releaseDate", deserialize_with = "lenient::string")]
    pub(super) release_date_camel: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) releasedate: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) year: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) last_modified: Option<u64>,
    #[serde(default, deserialize_with = "lenient::f32")]
    pub(super) rating: Option<f32>,
    #[serde(default, deserialize_with = "lenient::f32")]
    pub(super) rating_5based: Option<f32>,
    #[serde(default, deserialize_with = "lenient::first_string")]
    pub(super) backdrop_path: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) episode_run_time: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawVodInfo {
    #[serde(default, deserialize_with = "lenient::object")]
    pub(super) info: Option<RawMovieDetails>,
    #[serde(default, deserialize_with = "lenient::object")]
    pub(super) movie_data: Option<RawMovieData>,
}

#[derive(Deserialize)]
pub(super) struct RawMovieDetails {
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) movie_image: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) cover_big: Option<String>,
    #[serde(default, deserialize_with = "lenient::first_string")]
    pub(super) backdrop_path: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) plot: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) description: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) cast: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) actors: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) director: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) genre: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) releasedate: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) release_date: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) duration_secs: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) duration: Option<String>,
    #[serde(default, deserialize_with = "lenient::f32")]
    pub(super) rating: Option<f32>,
}

#[derive(Deserialize)]
pub(super) struct RawMovieData {
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) container_extension: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawSeriesInfo {
    #[serde(default, deserialize_with = "lenient::list")]
    pub(super) seasons: Vec<RawSeason>,
    #[serde(default, deserialize_with = "lenient::object")]
    pub(super) info: Option<RawSeries>,
    #[serde(default, deserialize_with = "lenient::episodes")]
    pub(super) episodes: Vec<RawEpisode>,
}

#[derive(Deserialize)]
pub(super) struct RawSeason {
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) season_number: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) name: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) overview: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) cover: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RawEpisode {
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) id: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) episode_num: Option<u64>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) season: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) title: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) container_extension: Option<String>,
    #[serde(default, deserialize_with = "lenient::object")]
    pub(super) info: Option<RawEpisodeInfo>,
}

#[derive(Deserialize)]
pub(super) struct RawEpisodeInfo {
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) movie_image: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) plot: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) releasedate: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) release_date: Option<String>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) air_date: Option<String>,
    #[serde(default, deserialize_with = "lenient::u64")]
    pub(super) duration_secs: Option<u64>,
    #[serde(default, deserialize_with = "lenient::string")]
    pub(super) duration: Option<String>,
}

/// Field deserializers that accept the shapes providers actually send.
mod lenient {
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    use super::RawEpisode;

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

    /// A number sent as a number or a numeric string; zero, which panels
    /// send for "no rating", and anything else is `None`.
    pub fn f32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
        let n = match Value::deserialize(d)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        };
        Ok(n.filter(|n| n.is_finite() && *n > 0.0).map(|n| n as f32))
    }

    /// Text sent as a string or as a list of strings (the first one).
    pub fn first_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::String(s) if !s.trim().is_empty() => Some(s),
            Value::Array(items) => items.into_iter().find_map(|v| match v {
                Value::String(s) if !s.trim().is_empty() => Some(s),
                _ => None,
            }),
            _ => None,
        })
    }

    /// An object; PHP panels send `[]` for an empty one, which is `None`
    /// like anything else that does not parse.
    pub fn object<'de, D: Deserializer<'de>, T: DeserializeOwned>(
        d: D,
    ) -> Result<Option<T>, D::Error> {
        Ok(match Value::deserialize(d)? {
            value @ Value::Object(_) => serde_json::from_value(value).ok(),
            _ => None,
        })
    }

    /// A list of objects, skipping entries that do not parse.
    pub fn list<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Array(items) => items
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect(),
            _ => Vec::new(),
        })
    }

    /// Episodes as an object keyed by season number (the documented shape)
    /// or as a list of per-season lists. An episode without a `season`
    /// takes its key's.
    pub fn episodes<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RawEpisode>, D::Error> {
        let seasons: Vec<(Option<u64>, Value)> = match Value::deserialize(d)? {
            Value::Object(map) => map
                .into_iter()
                .map(|(key, v)| (key.trim().parse().ok(), v))
                .collect(),
            Value::Array(items) => items.into_iter().map(|v| (None, v)).collect(),
            _ => Vec::new(),
        };
        let mut episodes = Vec::new();
        for (season, value) in seasons {
            let items = match value {
                Value::Array(items) => items,
                single @ Value::Object(_) => vec![single],
                _ => continue,
            };
            for item in items {
                if let Ok(mut episode) = serde_json::from_value::<RawEpisode>(item) {
                    episode.season = episode.season.or(season);
                    episodes.push(episode);
                }
            }
        }
        Ok(episodes)
    }
}
