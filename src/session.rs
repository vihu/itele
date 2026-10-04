//! What the window shows and why: providers, the channel lists, and
//! playback.
//!
//! Several providers are signed in at once. Network and keychain work runs
//! on worker threads; results come back to the UI thread through
//! `slint::invoke_from_event_loop`. Stream URLs carry the password, so none
//! is ever logged or shown.

mod browse;
mod guide;
mod player;
mod providers;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use itele::epg::Store;
use itele::provider::{Library, Paths, Provider};
use itele::xtream::{Credentials, LiveStream};
use mpv_engine::{EndReason, Engine, PlaybackEvent};
use slint::{ComponentHandle, SharedString, Timer, VecModel};

use crate::live::{Catalog, View, thousands};
use crate::logos::Logos;
use crate::playback::{self, SEEK_STEP, VOLUME_STEP};
use crate::ui::{AppWindow, ChannelItem, Screen};

/// Delay before the preview follows the selection, so holding Down does
/// not open a stream per row.
const SELECT_DELAY: Duration = Duration::from_millis(300);
/// How long the player banner stays up after the last key or mouse move.
const BANNER_TIME: Duration = Duration::from_secs(4);
/// How often the player banner rereads mpv while it is showing.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

thread_local! {
    static SESSION: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

/// The window's controller. Lives on the UI thread.
pub struct Session {
    app: slint::Weak<AppWindow>,
    paths: Paths,
    engine: Arc<Engine>,
    state: RefCell<State>,
    logos: RefCell<Logos>,
    /// Read side of the guide store; imports write through their own
    /// connection on a worker thread.
    guide: RefCell<Option<Store>>,
    select_timer: Timer,
    banner_timer: Timer,
    banner_until: Cell<Instant>,
    poll_timer: Timer,
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
    /// Results of work started for an earlier slot with the same provider
    /// (signed out since) carry another epoch and are dropped.
    epoch: u64,
}

/// The channel mpv is playing.
#[derive(Clone)]
struct Playing {
    provider: String,
    stream: LiveStream,
}

/// Wires `app` to a new session and opens the saved providers, or the login
/// screen when there are none.
pub fn start(app: &AppWindow, engine: Arc<Engine>, paths: Paths) {
    engine.set_wakeup_callback(drain_events(Arc::downgrade(&engine), app.as_weak()));
    let logos = Logos::start(paths.logos_dir(), |url, path| {
        on_ui_thread(move |s| s.logo_ready(url, path));
    });
    let guide = Store::open(&paths.guide_path()).ok();
    let session = Rc::new(Session {
        app: app.as_weak(),
        paths,
        engine,
        state: RefCell::default(),
        logos: RefCell::new(logos),
        guide: RefCell::new(guide),
        select_timer: Timer::default(),
        banner_timer: Timer::default(),
        banner_until: Cell::new(Instant::now()),
        poll_timer: Timer::default(),
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
    app.on_seek_to(|fraction| with_session(|s| playback::seek_to(&s.engine, fraction)));
    app.on_set_volume(|percent| with_session(|s| playback::set_volume(&s.engine, percent)));
    app.on_toggle_mute(|| with_session(|s| playback::toggle_mute(&s.engine)));
    app.on_change_volume(|sign| {
        with_session(|s| {
            playback::change_volume(&s.engine, f64::from(sign.signum()) * VOLUME_STEP);
        });
    });
    app.on_cycle_audio(|| with_session(|s| playback::next_audio(&s.engine)));
    app.on_cycle_subtitles(|| with_session(|s| playback::next_subtitles(&s.engine)));

    session.open_saved();
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
                .map_or_else(SharedString::new, |slot| slot.status.as_str().into()),
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

/// For example `1,284 channels · until 12 Jan 2027`.
fn library_status(library: &Library) -> String {
    let channels = format!("{} channels", thousands(library.streams.len()));
    let until = library.account.expires_at.and_then(|secs| {
        let at = jiff::Timestamp::from_second(i64::try_from(secs).ok()?).ok()?;
        Some(
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%-d %b %Y")
                .to_string(),
        )
    });
    match until {
        Some(date) => format!("{channels} · until {date}"),
        None => channels,
    }
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
                        app.set_video_note("This channel is not available right now".into());
                    }
                    PlaybackEvent::Ended {
                        reason: EndReason::Eof,
                    } => {
                        app.set_video_note("The stream ended".into());
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
