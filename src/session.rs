//! What the window shows and why: provider login, catalog refresh, channel
//! selection, and playback.
//!
//! Network and keychain work runs on worker threads; results come back to
//! the UI thread through `slint::invoke_from_event_loop`. Stream URLs carry
//! the password, so none is ever logged or shown.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::thread;
use std::time::{Duration, Instant};

use itele::provider::{self, Library, Paths, Provider};
use itele::xtream::{self, Action, Client, Credentials, LiveStream};
use mpv_engine::{EndReason, Engine, PlaybackEvent};
use slint::{ComponentHandle, Image, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::live::{Catalog, thousands};
use crate::playback::{self, SEEK_STEP, VOLUME_STEP};
use crate::ui::{AppWindow, Screen};

mod player;

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
    select_timer: Timer,
    banner_timer: Timer,
    banner_until: Cell<Instant>,
    poll_timer: Timer,
}

#[derive(Default)]
struct State {
    /// Bumped on sign-out, so late results for the old provider are dropped.
    epoch: u64,
    provider: Option<Provider>,
    credentials: Option<Credentials>,
    catalog: Option<Catalog>,
    group: usize,
    playing: Option<LiveStream>,
}

/// Wires `app` to a new session and opens the saved provider, or the login
/// screen when there is none.
pub fn start(app: &AppWindow, engine: Arc<Engine>, paths: Paths) {
    engine.set_wakeup_callback(drain_events(Arc::downgrade(&engine), app.as_weak()));
    let session = Rc::new(Session {
        app: app.as_weak(),
        paths,
        engine,
        state: RefCell::default(),
        select_timer: Timer::default(),
        banner_timer: Timer::default(),
        banner_until: Cell::new(Instant::now()),
        poll_timer: Timer::default(),
    });
    SESSION.with(|s| *s.borrow_mut() = Some(Rc::clone(&session)));

    app.on_login(|| with_session(|s| s.login()));
    app.on_group_selected(|i| with_session(|s| s.select_group(i.max(0) as usize)));
    app.on_channel_selected(|i| with_session(|s| s.select_channel(i)));
    app.on_watch(|| with_session(|s| s.watch()));
    app.on_back(|| with_session(|s| s.back()));
    app.on_zap(|delta| with_session(|s| s.zap(delta)));
    app.on_show_banner(|| with_session(|s| s.show_banner()));
    app.on_sign_out(|| with_session(|s| s.sign_out()));
    app.on_escape(|| with_session(|s| s.escape()));
    app.on_go_live(|| with_session(|s| s.go_live()));
    app.on_toggle_fullscreen(|| with_session(|s| s.toggle_fullscreen()));
    app.on_toggle_pause(|| with_session(|s| playback::toggle_pause(&s.engine)));
    app.on_seek(|sign| {
        with_session(|s| playback::seek(&s.engine, f64::from(sign.signum()) * SEEK_STEP))
    });
    app.on_seek_to(|fraction| with_session(|s| playback::seek_to(&s.engine, fraction)));
    app.on_set_volume(|percent| with_session(|s| playback::set_volume(&s.engine, percent)));
    app.on_toggle_mute(|| with_session(|s| playback::toggle_mute(&s.engine)));
    app.on_change_volume(|sign| {
        with_session(|s| playback::change_volume(&s.engine, f64::from(sign.signum()) * VOLUME_STEP))
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

/// Runs `f` with the session on the UI thread, from any thread, unless the
/// user signed out since `epoch`.
fn on_ui_thread(epoch: u64, f: impl FnOnce(&Rc<Session>) + Send + 'static) {
    // Err only once the event loop has quit; nothing is left to update.
    let _ = slint::invoke_from_event_loop(move || {
        with_session(|s| {
            if s.state.borrow().epoch == epoch {
                f(s);
            }
        });
    });
}

// Flows
impl Session {
    fn open_saved(&self) {
        match self.paths.load_providers() {
            Ok(providers) => match providers.into_iter().next() {
                Some(provider) => self.open(provider),
                None => self.show_login(""),
            },
            Err(e) => self.show_login(&e.to_string()),
        }
    }

    /// Shows the cached catalog at once, then refreshes it in the background.
    fn open(&self, provider: Provider) {
        if let Some(app) = self.app.upgrade() {
            app.set_provider_name(provider.name.as_str().into());
            app.set_provider_user(provider.username.as_str().into());
            app.set_avatar(initials(&provider.username).into());
        }
        let epoch = {
            let mut state = self.state.borrow_mut();
            state.provider = Some(provider.clone());
            state.epoch
        };
        let cache = self.paths.cache(&provider);
        match Library::from_cache(&cache) {
            Some(library) => self.show_catalog(library, "Updating…".into()),
            None => self.set_status("Loading channels…".into()),
        }
        self.show(Screen::Live);

        thread::spawn(move || {
            let credentials = match provider.credentials() {
                Ok(credentials) => credentials,
                Err(e) => return on_ui_thread(epoch, move |s| s.refresh_failed(&e)),
            };
            let client = Client::new(credentials.clone());
            on_ui_thread(epoch, move |s| s.credentials_ready(credentials));
            let result = Library::fetch(&client, &cache);
            on_ui_thread(epoch, move |s| match result {
                Ok(library) => {
                    let status = s.status_line(&library);
                    s.show_catalog(library, status);
                }
                Err(e) => s.refresh_failed(&e),
            });
        });
    }

    fn login(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let credentials = match Credentials::new(
            &app.get_login_server(),
            app.get_login_username().trim(),
            app.get_login_password(),
        ) {
            Ok(credentials) => credentials,
            Err(e) => return app.set_login_error(e.to_string().into()),
        };
        app.set_login_error(SharedString::new());
        app.set_login_busy(true);

        // Only the account is checked here; the channel list, which can be
        // tens of megabytes, loads in the background once Live TV shows.
        let paths = self.paths.clone();
        let epoch = self.state.borrow().epoch;
        thread::spawn(move || {
            let provider = Provider::new(host_of(credentials.server()), &credentials);
            let result = Client::new(credentials.clone())
                .fetch(Action::Account)
                .and_then(|body| xtream::parse_account(&body))
                .map_err(provider::Error::Xtream)
                .and_then(|_| {
                    provider.save_password(credentials.password())?;
                    paths.save_providers(std::slice::from_ref(&provider))?;
                    Ok(provider)
                });
            on_ui_thread(epoch, move |s| s.logged_in(result));
        });
    }

    fn logged_in(&self, result: provider::Result<Provider>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_login_busy(false);
        match result {
            Ok(provider) => {
                app.set_login_password(SharedString::new());
                self.open(provider);
            }
            Err(e) => app.set_login_error(e.to_string().into()),
        }
    }

    /// Forgets the provider: password, cache, and saved entry.
    fn sign_out(&self) {
        let provider = {
            let mut state = self.state.borrow_mut();
            let provider = state.provider.take();
            *state = State {
                epoch: state.epoch + 1,
                ..State::default()
            };
            provider
        };
        self.poll_timer.stop();
        self.select_timer.stop();
        if let Err(e) = self.engine.stop() {
            eprintln!("stop playback: {e}");
        }
        if let Some(app) = self.app.upgrade() {
            app.set_groups(ModelRc::default());
            app.set_channels(ModelRc::default());
            app.set_channel_index(-1);
            app.set_frame(Image::default());
            app.set_video_note(SharedString::new());
            app.set_status(SharedString::new());
            app.set_login_server(provider.as_ref().map_or("", |p| p.server.as_str()).into());
            app.set_login_username(SharedString::new());
        }
        let cleanup = provider.map_or(Ok(()), |provider| {
            provider.forget_password()?;
            self.paths.remove_provider(&provider)
        });
        match cleanup {
            Ok(()) => self.show_login(""),
            Err(e) => self.show_login(&format!("Signed out, but cleanup failed: {e}")),
        }
    }

    fn credentials_ready(&self, credentials: Credentials) {
        self.state.borrow_mut().credentials = Some(credentials);
        if self.state.borrow().playing.is_none() {
            self.play_selected();
        }
    }

    fn refresh_failed(&self, error: &provider::Error) {
        if self.state.borrow().catalog.is_some() {
            self.set_status(format!("Could not update: {error}").into());
        } else {
            self.show_login(&error.to_string());
        }
    }

    fn show_catalog(&self, library: Library, status: SharedString) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let catalog = Catalog::new(library);
        app.set_groups(ModelRc::new(VecModel::from(catalog.group_items())));
        let group = {
            let mut state = self.state.borrow_mut();
            state.catalog = Some(catalog);
            state.group
        };
        self.set_status(status);
        self.select_group(group);
    }

    fn select_group(&self, group: usize) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let State {
            catalog: Some(catalog),
            playing,
            ..
        } = &*state
        else {
            return;
        };
        let group = group.min(catalog.group_items().len().saturating_sub(1));
        app.set_group_index(group as i32);
        app.set_group_title(catalog.group_name(group).into());
        app.set_channels(ModelRc::new(VecModel::from(catalog.channel_items(group))));
        let row = playing.as_ref().and_then(|p| catalog.row_of(group, p));
        app.set_channel_index(row.map_or(-1, |r| r as i32));
        state.group = group;
    }

    fn select_channel(&self, row: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_channel_index(row);
        self.select_timer
            .start(TimerMode::SingleShot, SELECT_DELAY, || {
                with_session(|s| s.play_selected());
            });
    }
}

// Helpers
impl Session {
    /// Plays the selected channel, unless it is already playing.
    fn play_selected(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Ok(row) = usize::try_from(app.get_channel_index()) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let State {
            catalog: Some(catalog),
            credentials,
            group,
            playing,
            ..
        } = &mut *state
        else {
            return;
        };
        let Some(stream) = catalog.stream(*group, row) else {
            return;
        };
        if playing.as_ref().is_some_and(|p| p.id == stream.id) {
            return;
        }
        let Some(credentials) = credentials else {
            return app.set_video_note("Connecting to the provider…".into());
        };
        let url = credentials.live_url(stream.id, catalog.account().preferred_format());
        match self.engine.load(&url) {
            Ok(()) => {
                app.set_video_note("Tuning…".into());
                *playing = Some(stream.clone());
            }
            Err(_) => app.set_video_note("Could not start playback".into()),
        }
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
        }
        self.show(Screen::Login);
    }

    fn set_status(&self, status: SharedString) {
        if let Some(app) = self.app.upgrade() {
            app.set_status(status);
        }
    }

    /// For example `1,284 channels · until 12 Jan 2027`.
    fn status_line(&self, library: &Library) -> SharedString {
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
            Some(date) => format!("{channels} · until {date}").into(),
            None => channels.into(),
        }
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
