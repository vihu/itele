//! What the window shows and why: providers, the channel lists, and
//! playback.
//!
//! Several providers are signed in at once. Network and keychain work runs
//! on worker threads; results come back to the UI thread through
//! `slint::invoke_from_event_loop`. Stream URLs carry the password, so none
//! is ever logged or shown.

mod about;
mod browse;
mod catchup;
mod details;
mod grid;
mod guide;
mod navigate;
mod page;
mod player;
mod preferences;
mod providers;
mod refresh;
mod search;
mod series;
mod settings;
mod shelves;
mod timefmt;
mod vod;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use itele::epg::{Programme, Store};
use itele::history::{Entry, History};
use itele::provider::{Library, Paths, Provider};
use itele::settings::Settings;
use itele::xtream::{Credentials, LiveStream, Movie, Show};
use mpv_engine::{EndReason, Engine, PlaybackEvent};
use slint::{ComponentHandle, SharedString, Timer, TimerMode, VecModel};

use crate::art::Art;
use crate::live::{Catalog, GuideKey, View};
use crate::logos::Logos;
use crate::names::thousands;
use crate::playback::{self, SEEK_STEP, VOLUME_STEP};
use crate::tracks;
use crate::ui::{AppWindow, ChannelItem, Screen, SearchData, SettingsData, Shell};

/// Delay before the preview follows the selection, so holding Down does
/// not open a stream per row.
const SELECT_DELAY: Duration = Duration::from_millis(300);
/// How long the player banner stays up after the last key or mouse move.
const BANNER_TIME: Duration = Duration::from_secs(4);
/// How often the player banner rereads mpv while it is showing.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How often now and next are read again, so progress moves and
/// programmes roll over.
const GUIDE_TICK: Duration = Duration::from_secs(30);
/// How often the top bar's clock is set: often enough to turn with the
/// minute.
const CLOCK_TICK: Duration = Duration::from_secs(5);

thread_local! {
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

/// The window's controller. Lives on the UI thread.
pub struct Session {
    app: slint::Weak<AppWindow>,
    paths: Paths,
    settings: RefCell<Settings>,
    engine: Arc<Engine>,
    state: RefCell<State>,
    logos: RefCell<Logos>,
    art: RefCell<Art>,
    /// Read side of the guide store; imports write through their own
    /// connection on a worker thread.
    guide: RefCell<Option<Store>>,
    /// Watch history; `None` when it cannot be opened, which only loses
    /// resume.
    history: Option<History>,
    /// When the progress of the title playing was last saved.
    saved_at: Cell<Instant>,
    /// The length of the title playing, as mpv last reported it; at the end
    /// of the file mpv no longer can.
    title_length: Cell<f64>,
    select_timer: Timer,
    banner_timer: Timer,
    banner_until: Cell<Instant>,
    poll_timer: Timer,
    guide_timer: Timer,
    search_timer: Timer,
    refresh_timer: Timer,
    clock_timer: Timer,
}

#[derive(Default)]
struct State {
    slots: Vec<Slot>,
    catalog: Catalog,
    group: usize,
    /// The rows of the channel list, updated in place as logos arrive.
    channels: Rc<VecModel<ChannelItem>>,
    /// Each row's logo URL, empty when the provider has none.
    row_logos: Vec<String>,
    /// Where each row finds its programmes.
    row_guides: Vec<GuideKey>,
    /// Days of catch-up each row keeps; 0 for none.
    row_archive: Vec<u32>,
    /// Rows the channel list last reported on screen.
    visible: std::ops::Range<usize>,
    grid: grid::Grid,
    search: search::Search,
    movies: crate::vod::Catalog<Movie>,
    shows: crate::vod::Catalog<Show>,
    browse: vod::Browse,
    /// The detail page showing, if any.
    page: Option<page::Page>,
    /// Source of page tokens.
    next_page: u64,
    playing: Option<Playing>,
    /// Source of [`Slot::epoch`] values.
    next_epoch: u64,
}

/// A signed-in provider.
struct Slot {
    provider: Provider,
    credentials: Option<Credentials>,
    status: String,
    /// How the last guide import went; empty before the first one.
    guide: String,
    movies: shelves::ShelfState,
    shows: shelves::ShelfState,
    /// A refresh is running; another is not started meanwhile.
    refreshing: bool,
    /// Results of work started for an earlier slot with the same provider
    /// (signed out since) carry another epoch and are dropped.
    epoch: u64,
}

/// What mpv is playing, and from which provider.
#[derive(Clone)]
struct Playing {
    provider: String,
    content: Content,
}

#[derive(Clone)]
enum Content {
    /// A channel, live.
    Live(LiveStream),
    /// A past programme of a channel, from catch-up.
    Replay(LiveStream, Programme),
    /// A movie or an episode.
    Title(Title),
}

/// A movie or an episode playing.
#[derive(Clone)]
struct Title {
    name: String,
    /// Where its progress is kept.
    entry: Entry,
}

impl Playing {
    /// The channel, when a channel is playing (live or from catch-up).
    fn channel(&self) -> Option<&LiveStream> {
        match &self.content {
            Content::Live(stream) | Content::Replay(stream, _) => Some(stream),
            Content::Title(_) => None,
        }
    }
}

/// Wires `app` to a new session and opens the saved providers, or the login
/// screen when there are none.
pub fn start(app: &AppWindow, engine: Arc<Engine>, paths: Paths) {
    engine.set_wakeup_callback(drain_events(Arc::downgrade(&engine), app.as_weak()));
    let logos = Logos::start(paths.logos_dir(), |url, path| {
        on_ui_thread(move |s| s.logo_ready(url, path));
    });
    let art = Art::start(paths.art_dir(), |url, size, picture| {
        on_ui_thread(move |s| s.art_ready(url, size, picture));
    });
    let guide = Store::open(&paths.guide_path()).ok();
    let history = History::open(&paths.history_path())
        .inspect_err(|e| eprintln!("watch history: {e}"))
        .ok();
    let settings = paths
        .load_settings()
        .inspect_err(|e| eprintln!("settings: {e}"))
        .unwrap_or_default();
    let session = Rc::new(Session {
        app: app.as_weak(),
        paths,
        settings: RefCell::new(settings),
        engine,
        state: RefCell::default(),
        logos: RefCell::new(logos),
        art: RefCell::new(art),
        guide: RefCell::new(guide),
        history,
        saved_at: Cell::new(Instant::now()),
        title_length: Cell::new(0.0),
        select_timer: Timer::default(),
        banner_timer: Timer::default(),
        banner_until: Cell::new(Instant::now()),
        poll_timer: Timer::default(),
        guide_timer: Timer::default(),
        search_timer: Timer::default(),
        refresh_timer: Timer::default(),
        clock_timer: Timer::default(),
    });
    SESSION.with(|s| *s.borrow_mut() = Some(Rc::clone(&session)));

    app.on_login(|| with_session(|s| s.login()));
    app.on_cancel_login(|| with_session(|s| s.cancel_login()));
    app.on_add_provider(|| with_session(|s| s.add_provider()));
    app.on_sign_out(|i| with_session(|s| s.sign_out(i)));
    app.on_view_selected(|i| with_session(|s| s.select_view(i)));
    app.on_group_selected(|i| with_session(|s| s.select_group(i.max(0) as usize)));
    app.on_channel_selected(|i| with_session(|s| s.select_channel(i)));
    app.on_rows_visible(|first, count| with_session(|s| s.rows_visible(first, count)));
    app.on_watch(|| with_session(|s| s.watch()));
    app.on_back(|| with_session(|s| s.back()));
    app.on_zap(|delta| with_session(|s| s.zap(delta)));
    app.on_show_banner(|| with_session(|s| s.show_banner()));
    app.on_escape(|| with_session(|s| s.escape()));
    app.on_go_live(|| with_session(|s| s.go_live()));
    app.on_toggle_fullscreen(|| with_session(|s| s.toggle_fullscreen()));
    app.on_toggle_pause(|| with_session(|s| playback::toggle_pause(&s.engine)));
    app.on_seek(|sign| {
        with_session(|s| playback::seek(&s.engine, f64::from(sign.signum()) * SEEK_STEP));
    });
    app.on_seek_to(|fraction| {
        with_session(|s| playback::seek_to(&s.engine, fraction, s.timeline()));
    });
    app.on_set_volume(|percent| with_session(|s| playback::set_volume(&s.engine, percent)));
    app.on_toggle_mute(|| with_session(|s| playback::toggle_mute(&s.engine)));
    app.on_change_volume(|sign| {
        with_session(|s| {
            playback::change_volume(&s.engine, f64::from(sign.signum()) * VOLUME_STEP);
        });
    });
    app.on_navigate(|i| with_session(|s| s.navigate(i)));
    app.on_guide_rows_visible(|first, count| with_session(|s| s.grid_rows_visible(first, count)));
    app.on_guide_move(|dx, dy| with_session(|s| s.grid_move(dx, dy)));
    app.on_guide_page(|direction| with_session(|s| s.grid_page(direction)));
    app.on_guide_now(|| with_session(|s| s.grid_now()));
    app.on_guide_shift(|direction| with_session(|s| s.grid_shift(direction)));
    app.on_guide_cell_clicked(|row, cell| with_session(|s| s.grid_cell_clicked(row, cell)));
    app.on_guide_enter(|| with_session(|s| s.grid_enter()));
    app.on_guide_replay(|| with_session(|s| s.grid_enter()));
    let search = app.global::<SearchData>();
    search.on_edited(|_| with_session(|s| s.search_edited()));
    search.on_picked(|i| with_session(|s| s.search_picked(i)));
    search.on_move(|delta| with_session(|s| s.search_move(delta)));
    search.on_all(|| with_session(|s| s.open_search()));
    app.on_vod_group_selected(|i| with_session(|s| s.vod_group_selected(i)));
    app.on_vod_items_visible(|first, count| with_session(|s| s.vod_items_visible(first, count)));
    app.on_vod_open(|i| with_session(|s| s.vod_open(i)));
    app.on_details_play(|| with_session(|s| s.details_play(Start::Resume)));
    app.on_details_restart(|| with_session(|s| s.details_play(Start::Beginning)));
    app.on_details_back(|| with_session(|s| s.details_back()));
    app.on_details_season_selected(|i| with_session(|s| s.details_season_selected(i)));
    app.global::<Shell>()
        .on_toggle_sidebar(|| with_session(|s| s.toggle_sidebar()));
    session.apply_preferences();
    session.fill_about();
    let settings = app.global::<SettingsData>();
    settings.set_version(env!("CARGO_PKG_VERSION").into());
    settings.on_rename(|i, name| with_session(|s| s.rename_provider(i, &name)));
    settings.on_refresh_provider(|i| {
        with_session(|s| s.refresh_provider_at(i, refresh::Force::Yes));
    });
    settings.on_refresh_guide(|i| with_session(|s| s.refresh_provider_at(i, refresh::Force::No)));
    settings.on_refresh_all(|| with_session(|s| s.refresh_all(refresh::Force::Yes)));
    settings.on_shift_guide(|i, sign| with_session(|s| s.shift_guide(i, sign)));
    settings.on_set_setting(|key, value| with_session(|s| s.set_setting(key, value)));
    settings.on_measure_storage(|| with_session(|s| s.measure_storage()));
    settings.on_clear_cache(|| with_session(|s| s.clear_cache()));
    app.on_open_tracks(|kind| with_session(|s| s.open_tracks(track_kind(kind))));
    app.on_choose_track(|kind, id| {
        with_session(|s| tracks::select(&s.engine, track_kind(kind), i64::from(id)));
    });

    session.open_saved();
    session
        .guide_timer
        .start(TimerMode::Repeated, GUIDE_TICK, || {
            with_session(|s| s.tick_guide());
        });
    session.tick_clock();
    session
        .clock_timer
        .start(TimerMode::Repeated, CLOCK_TICK, || {
            with_session(|s| s.tick_clock());
        });
    session
        .refresh_timer
        .start(TimerMode::Repeated, refresh::CHECK_EVERY, || {
            with_session(|s| s.refresh_all(refresh::Force::No));
        });
}

/// Saves what is left to save before the window closes: the progress of
/// the title playing.
pub fn finish() {
    with_session(|s| s.save_progress());
}

/// Where a title starts playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    /// Where it was left, when far enough in and not finished.
    Resume,
    /// From the beginning.
    Beginning,
}

/// The window's number for a track menu: 0 audio, else subtitles.
fn track_kind(kind: i32) -> tracks::Kind {
    if kind == 0 {
        tracks::Kind::Audio
    } else {
        tracks::Kind::Subtitles
    }
}

/// Runs `f` with the session, if the window still has one.
fn with_session(f: impl FnOnce(&Rc<Session>)) {
    // Clone out first: `f` may re-enter through a callback.
    if let Some(session) = SESSION.with(|s| s.borrow().clone()) {
        f(&session);
    }
}

/// Runs `f` with the session on the UI thread, from any thread.
fn on_ui_thread(f: impl FnOnce(&Rc<Session>) + Send + 'static) {
    // Err only once the event loop has quit; nothing is left to update.
    let _ = slint::invoke_from_event_loop(move || with_session(f));
}

// Helpers
impl Session {
    /// Loads `url` in mpv, starting `start` seconds in, or at the start.
    /// mpv's `start` option applies to every later load, so it is set (or
    /// cleared) before each one.
    fn load(&self, url: &str, start: Option<f64>) -> mpv_engine::Result<()> {
        let start = start.map_or_else(|| "none".to_owned(), |s| format!("{s:.1}"));
        self.engine.set_property("start", start.as_str())?;
        self.reset_tracks();
        self.engine.load(url)
    }

    fn show(&self, screen: Screen) {
        if let Some(app) = self.app.upgrade() {
            app.set_screen(screen);
            app.invoke_focus_screen();
        }
    }

    fn show_login(&self, error: &str) {
        if let Some(app) = self.app.upgrade() {
            app.set_login_error(error.into());
            app.set_login_can_cancel(!self.state.borrow().slots.is_empty());
        }
        self.show(Screen::Login);
    }

    /// The status line under "Live TV": the viewed provider's, or a summary
    /// of all of them.
    fn view_status(&self) -> SharedString {
        let state = self.state.borrow();
        match state.catalog.view() {
            View::One(id) => state
                .slots
                .iter()
                .find(|slot| &slot.provider.id == id)
                .map_or_else(SharedString::new, |slot| {
                    format!("{} · {}", slot.status, slot.provider.name).into()
                }),
            View::All => match state.slots.len() {
                0 => SharedString::new(),
                1 => state.slots[0].status.as_str().into(),
                n => format!(
                    "{n} providers · {} channels",
                    thousands(state.catalog.total_channels())
                )
                .into(),
            },
        }
    }
}

/// For example `1,284 channels`.
fn library_status(library: &Library) -> String {
    format!("{} channels", thousands(library.streams.len()))
}

/// When the account expires, for example `12 Jan 2027`; `None` when it
/// does not.
fn expiry(account: &itele::xtream::Account) -> Option<String> {
    let secs = i64::try_from(account.expires_at?).ok()?;
    let at = jiff::Timestamp::from_second(secs).ok()?;
    Some(
        at.to_zoned(jiff::tz::TimeZone::system())
            .strftime("%-d %b %Y")
            .to_string(),
    )
}

/// Builds mpv's wakeup callback, which turns playback events into the
/// video note on the UI thread. mpv forbids API calls inside the callback.
fn drain_events(
    engine: Weak<Engine>,
    app: slint::Weak<AppWindow>,
) -> impl Fn() + Send + Sync + 'static {
    move || {
        let engine = engine.clone();
        let app = app.clone();
        // Err only once the event loop has quit.
        let _ = slint::invoke_from_event_loop(move || {
            let (Some(engine), Some(app)) = (engine.upgrade(), app.upgrade()) else {
                return;
            };
            for event in engine.pump_events() {
                match event {
                    PlaybackEvent::PlaybackRestart => app.set_video_note(SharedString::new()),
                    PlaybackEvent::Failed { .. } => {
                        app.set_video_note("This stream is not available right now".into());
                    }
                    PlaybackEvent::Ended {
                        reason: EndReason::Eof,
                    } => {
                        app.set_video_note("The stream ended".into());
                        with_session(|s| s.title_ended());
                    }
                    _ => {}
                }
            }
        });
    }
}

/// `http://tv.example.com:8080` as `tv.example.com`.
fn host_of(server: &str) -> &str {
    let rest = server.split_once("://").map_or(server, |(_, rest)| rest);
    rest.split([':', '/']).next().unwrap_or(rest)
}

/// Two letters for the avatar, from the username.
fn initials(username: &str) -> String {
    username
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_take_two_letters() {
        assert_eq!(initials("north.wind"), "NO");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn host_of_strips_scheme_port_and_path() {
        assert_eq!(host_of("http://tv.example.com:8080"), "tv.example.com");
        assert_eq!(host_of("https://tv.example.com/iptv"), "tv.example.com");
        assert_eq!(host_of("tv.example.com"), "tv.example.com");
    }
}
