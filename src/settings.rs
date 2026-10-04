//! The user's settings: kept in `settings.json` in the config directory.
//!
//! Every field has a default and unknown fields are ignored, so a file from
//! an older or newer itele, or one edited by hand, still loads. Settings
//! that belong to one provider (its name, its guide time shift) live with
//! the provider in `providers.json` instead.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Everything the Settings screen changes, apart from the providers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How old a guide may get before it is downloaded again.
    pub guide_refresh: Refresh,
    /// How old the channel, movie and series lists may get.
    pub list_refresh: Refresh,
    /// Days of past programmes kept for catch-up.
    pub keep_days: u32,
    /// Decode on the GPU when mpv can.
    pub hardware_decoding: bool,
    /// Preferred audio language as mpv's `alang` takes it, for example
    /// `eng`; empty leaves the stream's default.
    pub audio_language: String,
    /// Preferred subtitle language; empty turns subtitles off.
    pub subtitle_language: String,
    /// The live stream container to ask providers for.
    pub stream_format: StreamFormat,
    /// Megabytes of a live channel kept to pause and rewind.
    pub rewind_megabytes: u32,
    /// Continue movies and episodes where they stopped.
    pub resume: bool,
    /// Play the next episode when one ends.
    pub next_episode: bool,
    /// The sidebar with labels, or icons only.
    pub sidebar: Sidebar,
    /// The screen itele opens with.
    pub start_on: StartOn,
    /// The screen showing when itele last closed, for
    /// [`StartOn::LastScreen`].
    pub last_screen: Place,
    /// 24-hour or 12-hour times.
    pub clock: Clock,
}

/// A screen itele can open with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Place {
    /// Live TV.
    LiveTv,
    /// The guide.
    Guide,
    /// Movies.
    Movies,
    /// Series.
    Series,
    /// Favorites.
    Favorites,
}

/// How often something is downloaded again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refresh {
    /// When older than 6 hours.
    #[serde(rename = "6h")]
    Every6Hours,
    /// When older than 12 hours.
    #[serde(rename = "12h")]
    Every12Hours,
    /// When older than a day.
    Daily,
    /// Only when the user asks.
    Manual,
}

/// The live stream container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamFormat {
    /// MPEG-TS when the account allows it, else HLS.
    Automatic,
    /// Always MPEG-TS.
    Ts,
    /// Always HLS.
    Hls,
}

/// How the sidebar shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sidebar {
    /// Icons with labels.
    Expanded,
    /// Icons only.
    Collapsed,
}

/// The screen itele opens with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartOn {
    /// Live TV.
    LiveTv,
    /// The screen showing when itele last closed.
    LastScreen,
}

/// How times are written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Clock {
    /// `21:30`.
    Hours24,
    /// `9:30 PM`.
    Hours12,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            guide_refresh: Refresh::Every12Hours,
            list_refresh: Refresh::Every12Hours,
            keep_days: 7,
            hardware_decoding: true,
            audio_language: String::new(),
            subtitle_language: String::new(),
            stream_format: StreamFormat::Automatic,
            rewind_megabytes: 512,
            resume: true,
            next_episode: true,
            sidebar: Sidebar::Expanded,
            start_on: StartOn::LiveTv,
            last_screen: Place::LiveTv,
            clock: Clock::Hours24,
        }
    }
}

impl Refresh {
    /// The age past which to download again; `None` for [`Refresh::Manual`].
    pub const fn max_age(self) -> Option<Duration> {
        match self {
            Refresh::Every6Hours => Some(Duration::from_secs(6 * 3600)),
            Refresh::Every12Hours => Some(Duration::from_secs(12 * 3600)),
            Refresh::Daily => Some(Duration::from_secs(24 * 3600)),
            Refresh::Manual => None,
        }
    }

    /// Whether something `age` old is due again.
    pub fn is_due(self, age: Duration) -> bool {
        self.max_age().is_some_and(|max| age >= max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let settings = Settings {
            guide_refresh: Refresh::Manual,
            sidebar: Sidebar::Collapsed,
            audio_language: "eng".into(),
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains(r#""guide_refresh":"manual""#), "{json}");
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    }

    #[test]
    fn partial_and_unknown_fields_load() {
        let settings: Settings =
            serde_json::from_str(r#"{"clock":"hours12","from_the_future":1}"#).unwrap();
        assert_eq!(settings.clock, Clock::Hours12);
        assert_eq!(settings.guide_refresh, Refresh::Every12Hours, "defaulted");
        assert_eq!(
            serde_json::from_str::<Settings>("{}").unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn refresh_is_due_by_age() {
        let hour = Duration::from_secs(3600);
        assert!(!Refresh::Every6Hours.is_due(5 * hour));
        assert!(Refresh::Every6Hours.is_due(6 * hour));
        assert!(Refresh::Daily.is_due(25 * hour));
        assert!(!Refresh::Manual.is_due(1000 * hour));
    }
}
