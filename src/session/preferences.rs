//! The user's preferences: the values the Settings controls show, saving a
//! change, and applying it to mpv and the screens at once.

use itele::settings::{
    ChannelRef, Clock, Place, Refresh, Settings, Sidebar, StartOn, StreamFormat,
};
use itele::xtream::{Account, OutputFormat, StreamId};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{Session, timefmt};
use crate::ui::{Screen, SettingKey, SettingValues, SettingsData, Shell};

/// The interval choices, in the order the screen lists them.
const INTERVALS: [Refresh; 4] = [
    Refresh::Every6Hours,
    Refresh::Every12Hours,
    Refresh::Daily,
    Refresh::Manual,
];
/// The days-to-keep choices, in order.
const KEEP_DAYS: [u32; 3] = [3, 7, 14];
/// The rewind buffer choices, in megabytes.
const REWIND: [u32; 4] = [256, 512, 1024, 2048];
/// Languages as mpv's `alang` and `slang` take them, with their names. The
/// first is "none": the stream's own choice for audio, off for subtitles.
const LANGUAGES: [(&str, &str); 14] = [
    ("", ""),
    ("eng,en", "English"),
    ("fra,fre,fr", "French"),
    ("deu,ger,de", "German"),
    ("spa,es", "Spanish"),
    ("ita,it", "Italian"),
    ("nld,dut,nl", "Dutch"),
    ("por,pt", "Portuguese"),
    ("ara,ar", "Arabic"),
    ("tur,tr", "Turkish"),
    ("pol,pl", "Polish"),
    ("rus,ru", "Russian"),
    ("hin,hi", "Hindi"),
    ("jpn,ja", "Japanese"),
];

impl Session {
    /// Sets the top bar's clock to the current minute.
    pub(super) fn tick_clock(&self) {
        if let Some(app) = self.app.upgrade() {
            app.set_clock(timefmt::clock(timefmt::now()).into());
        }
    }

    /// Fills the Settings data that never changes, and applies the saved
    /// preferences; once, at start.
    pub(super) fn apply_preferences(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let names = |none: &str| -> ModelRc<SharedString> {
            ModelRc::new(VecModel::from(
                LANGUAGES
                    .iter()
                    .map(|&(code, name)| if code.is_empty() { none } else { name }.into())
                    .collect::<Vec<SharedString>>(),
            ))
        };
        let data = app.global::<SettingsData>();
        data.set_audio_languages(names("Stream default"));
        data.set_subtitle_languages(names("Off"));
        let settings = self.settings.borrow().clone();
        app.global::<Shell>()
            .set_sidebar_expanded(settings.sidebar == Sidebar::Expanded);
        timefmt::set_twelve_hour(settings.clock == Clock::Hours12);
        self.apply_playback(&settings);
    }

    /// The setting values as the controls show them.
    pub(super) fn setting_values(&self) -> SettingValues {
        let s = self.settings.borrow();
        let language = |code: &str| {
            LANGUAGES
                .iter()
                .position(|&(c, _)| c == code)
                .map_or(0, |i| i as i32)
        };
        SettingValues {
            guide_refresh: index_of(&INTERVALS, &s.guide_refresh),
            list_refresh: index_of(&INTERVALS, &s.list_refresh),
            keep_days: index_of(&KEEP_DAYS, &s.keep_days),
            hardware_decoding: s.hardware_decoding,
            audio_language: language(&s.audio_language),
            subtitle_language: language(&s.subtitle_language),
            stream_format: match s.stream_format {
                StreamFormat::Automatic => 0,
                StreamFormat::Ts => 1,
                StreamFormat::Hls => 2,
            },
            rewind_buffer: index_of(&REWIND, &s.rewind_megabytes),
            resume: s.resume,
            next_episode: s.next_episode,
            sidebar: i32::from(s.sidebar == Sidebar::Collapsed),
            start_on: i32::from(s.start_on == StartOn::LastScreen),
            clock: i32::from(s.clock == Clock::Hours12),
        }
    }

    /// One control changed `key` to its option `value` (0 or 1 for a
    /// switch): saves it and applies it.
    pub(super) fn set_setting(&self, key: SettingKey, value: i32) {
        let Ok(index) = usize::try_from(value) else {
            return;
        };
        let on = value != 0;
        let updated = {
            let mut s = self.settings.borrow_mut();
            let changed = match key {
                SettingKey::GuideRefresh => set(&mut s.guide_refresh, INTERVALS.get(index)),
                SettingKey::ListRefresh => set(&mut s.list_refresh, INTERVALS.get(index)),
                SettingKey::KeepDays => set(&mut s.keep_days, KEEP_DAYS.get(index)),
                SettingKey::HardwareDecoding => set(&mut s.hardware_decoding, Some(&on)),
                SettingKey::AudioLanguage => set(
                    &mut s.audio_language,
                    LANGUAGES.get(index).map(|l| l.0.to_owned()).as_ref(),
                ),
                SettingKey::SubtitleLanguage => set(
                    &mut s.subtitle_language,
                    LANGUAGES.get(index).map(|l| l.0.to_owned()).as_ref(),
                ),
                SettingKey::StreamFormat => set(
                    &mut s.stream_format,
                    [StreamFormat::Automatic, StreamFormat::Ts, StreamFormat::Hls].get(index),
                ),
                SettingKey::RewindBuffer => set(&mut s.rewind_megabytes, REWIND.get(index)),
                SettingKey::Resume => set(&mut s.resume, Some(&on)),
                SettingKey::NextEpisode => set(&mut s.next_episode, Some(&on)),
                SettingKey::Sidebar => set(
                    &mut s.sidebar,
                    Some(if on {
                        &Sidebar::Collapsed
                    } else {
                        &Sidebar::Expanded
                    }),
                ),
                SettingKey::StartOn => set(
                    &mut s.start_on,
                    Some(if on {
                        &StartOn::LastScreen
                    } else {
                        &StartOn::LiveTv
                    }),
                ),
                SettingKey::Clock => set(
                    &mut s.clock,
                    Some(if on { &Clock::Hours12 } else { &Clock::Hours24 }),
                ),
            };
            if !changed {
                return;
            }
            if let Err(e) = self.paths.save_settings(&s) {
                eprintln!("save settings: {e}");
            }
            s.clone()
        };
        match key {
            SettingKey::HardwareDecoding
            | SettingKey::AudioLanguage
            | SettingKey::SubtitleLanguage
            | SettingKey::RewindBuffer => self.apply_playback(&updated),
            SettingKey::Sidebar => {
                if let Some(app) = self.app.upgrade() {
                    app.global::<Shell>()
                        .set_sidebar_expanded(updated.sidebar == Sidebar::Expanded);
                }
            }
            SettingKey::Clock => {
                timefmt::set_twelve_hour(updated.clock == Clock::Hours12);
                self.refresh_lists();
                self.tick_guide();
                self.tick_clock();
            }
            SettingKey::StartOn => self.remember_screen(Screen::Live),
            SettingKey::Resume => self.refresh_page_history(),
            _ => {}
        }
        self.push_settings();
    }

    /// Expands or collapses the sidebar, and remembers it.
    pub(super) fn toggle_sidebar(&self) {
        let collapsed = self.settings.borrow().sidebar == Sidebar::Expanded;
        self.set_setting(SettingKey::Sidebar, i32::from(collapsed));
    }

    /// The live format to ask `account`'s provider for.
    pub(super) fn live_format(&self, account: &Account) -> OutputFormat {
        match self.settings.borrow().stream_format {
            StreamFormat::Automatic => account.preferred_format(),
            StreamFormat::Ts => OutputFormat::Ts,
            StreamFormat::Hls => OutputFormat::Hls,
        }
    }

    /// The tracks a new file starts with: mpv's track options persist
    /// across files, so each load sets them again.
    pub(super) fn reset_tracks(&self) {
        let subtitles = self.settings.borrow().subtitle_language.is_empty();
        for (name, value) in [
            ("aid", "auto"),
            ("sid", if subtitles { "no" } else { "auto" }),
        ] {
            if let Err(e) = self.engine.set_property(name, value) {
                eprintln!("player {name}: {e}");
            }
        }
    }

    /// Remembers `screen` for "Where I left off".
    pub(super) fn remember_screen(&self, screen: Screen) {
        let place = match screen {
            Screen::Home => Place::Home,
            Screen::Live => Place::LiveTv,
            Screen::Guide => Place::Guide,
            Screen::Movies => Place::Movies,
            Screen::Series => Place::Series,
            Screen::Favorites => Place::Favorites,
            _ => return,
        };
        let mut s = self.settings.borrow_mut();
        if s.start_on == StartOn::LastScreen && s.last_screen != place {
            s.last_screen = place;
            if let Err(e) = self.paths.save_settings(&s) {
                eprintln!("save settings: {e}");
            }
        }
    }

    /// Remembers the channel playing, which Home opens on next time.
    pub(super) fn remember_channel(&self, provider: &str, stream: StreamId) {
        let channel = Some(ChannelRef {
            provider: provider.to_owned(),
            stream: stream.0,
        });
        let mut s = self.settings.borrow_mut();
        if s.last_channel != channel {
            s.last_channel = channel;
            if let Err(e) = self.paths.save_settings(&s) {
                eprintln!("save settings: {e}");
            }
        }
    }

    /// Opens the screen the user starts on, after the providers opened.
    pub(super) fn open_start_screen(&self) {
        let (start, last) = {
            let s = self.settings.borrow();
            (s.start_on, s.last_screen)
        };
        if start == StartOn::LastScreen {
            match last {
                Place::LiveTv => {}
                Place::Home => self.navigate(0),
                Place::Guide => self.navigate(2),
                Place::Movies => self.navigate(3),
                Place::Series => self.navigate(4),
                Place::Favorites => self.navigate(7),
            }
        }
    }
}

// Private API
impl Session {
    fn apply_playback(&self, s: &Settings) {
        let rewind = format!("{}MiB", s.rewind_megabytes);
        let hwdec = if s.hardware_decoding {
            "auto-safe"
        } else {
            "no"
        };
        for (name, value) in [
            ("hwdec", hwdec),
            ("alang", s.audio_language.as_str()),
            ("slang", s.subtitle_language.as_str()),
            ("demuxer-max-back-bytes", rewind.as_str()),
        ] {
            if let Err(e) = self.engine.set_property(name, value) {
                eprintln!("player {name}: {e}");
            }
        }
    }
}

/// Sets `field` to `value` when there is one and it differs; whether it
/// changed.
fn set<T: Clone + PartialEq>(field: &mut T, value: Option<&T>) -> bool {
    match value {
        Some(value) if value != field => {
            *field = value.clone();
            true
        }
        _ => false,
    }
}

/// The position of `value` among `options`, for a control.
fn index_of<T: PartialEq>(options: &[T], value: &T) -> i32 {
    options
        .iter()
        .position(|o| o == value)
        .map_or(0, |i| i as i32)
}
