//! The player screen: entering and leaving it, channel switching,
//! fullscreen, the banner, and its live readout of mpv.

use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, TimerMode, VecModel};

use super::{BANNER_TIME, Content, POLL_INTERVAL, Session, with_session};
use crate::info;
use crate::playback::{self, Timeline};
use crate::ui::Screen;

impl Session {
    pub(super) fn watch(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if app.get_channel_index() < 0 {
            return;
        }
        self.select_timer.stop();
        self.play_selected();
        self.enter_player();
    }

    /// Shows the player with its banner and starts reading mpv.
    pub(super) fn enter_player(&self) {
        if let Some(app) = self.app.upgrade() {
            let label = if self.timeline() == Timeline::Title {
                "Back"
            } else {
                "Live TV"
            };
            app.set_player_back(label.into());
        }
        self.show(Screen::Player);
        self.show_banner();
        self.poll();
        self.poll_timer
            .start(TimerMode::Repeated, POLL_INTERVAL, || {
                with_session(|s| s.poll());
            });
    }

    /// Leaves the player: for Live TV, which keeps playing, or for the
    /// page of the title playing, which stops.
    pub(super) fn back(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        self.poll_timer.stop();
        let title = self
            .state
            .borrow()
            .playing
            .as_ref()
            .is_some_and(|p| matches!(p.content, Content::Title(_)));
        if title {
            self.stop_preview();
            self.show(Screen::Details);
        } else {
            self.show(Screen::Live);
            app.invoke_reveal_current();
        }
    }

    /// A movie or an episode played to its end: back to its page.
    pub(super) fn title_ended(&self) {
        let in_player = self
            .app
            .upgrade()
            .is_some_and(|app| app.get_screen() == Screen::Player);
        if in_player && self.timeline() == Timeline::Title {
            self.back();
        }
    }

    /// Esc leaves fullscreen first, then the player.
    pub(super) fn escape(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if app.window().is_fullscreen() {
            app.window().set_fullscreen(false);
        } else {
            self.back();
        }
    }

    pub(super) fn toggle_fullscreen(&self) {
        if let Some(app) = self.app.upgrade() {
            let window = app.window();
            window.set_fullscreen(!window.is_fullscreen());
        }
    }

    /// Jumps to the live edge by retuning, which is right even after a long
    /// pause filled the cache and stopped it following the stream; from a
    /// replay, returns to the channel live.
    pub(super) fn go_live(&self) {
        if self.timeline() == Timeline::Title {
            return;
        }
        self.state.borrow_mut().playing = None;
        self.play_selected();
        self.refresh_programme();
        if let Err(e) = self.engine.set_paused(false) {
            eprintln!("resume: {e}");
        }
    }

    pub(super) fn poll(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_player(playback::read(
            &self.engine,
            app.window().is_fullscreen(),
            self.timeline(),
        ));
        if app.get_info_visible() {
            let name = match self.state.borrow().playing.as_ref().map(|p| &p.content) {
                Some(Content::Title(title)) => title.name.as_str().into(),
                _ => app
                    .get_channels()
                    .row_data(app.get_channel_index().max(0) as usize)
                    .map(|c| c.name)
                    .unwrap_or_default(),
            };
            let lines = info::read(&self.engine, &name, &app.get_provider_name());
            app.set_info(ModelRc::new(VecModel::from(lines)));
        }
    }

    pub(super) fn zap(&self, delta: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if self.timeline() == Timeline::Title {
            return;
        }
        let len = {
            let state = self.state.borrow();
            state.catalog.group_len(state.group)
        };
        if len == 0 {
            return;
        }
        let row = (app.get_channel_index() + delta).rem_euclid(len as i32);
        app.set_channel_index(row);
        self.refresh_programme();
        self.play_selected();
        self.show_banner();
    }

    /// What the player's timeline spans for what is playing.
    pub(super) fn timeline(&self) -> Timeline {
        let state = self.state.borrow();
        match state.playing.as_ref().map(|p| &p.content) {
            Some(Content::Replay(_, programme)) => {
                Timeline::Programme((programme.stop - programme.start) as f64)
            }
            Some(Content::Title(_)) => Timeline::Title,
            Some(Content::Live(_)) | None => Timeline::Live,
        }
    }

    /// Whether a channel is playing live, or nothing is.
    pub(super) fn playing_live(&self) -> bool {
        self.timeline() == Timeline::Live
    }

    pub(super) fn show_banner(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_banner_visible(true);
        self.banner_until.set(Instant::now() + BANNER_TIME);
        self.banner_timer
            .start(TimerMode::SingleShot, BANNER_TIME, || {
                with_session(|s| {
                    // A stale expiry can still fire in the same tick as a
                    // restart; only the latest deadline hides the banner.
                    if Instant::now() < s.banner_until.get() {
                        return;
                    }
                    // A paused player keeps its controls up.
                    if let Some(app) = s.app.upgrade()
                        && !app.get_player().paused
                    {
                        app.set_banner_visible(false);
                    }
                });
            });
    }
}
