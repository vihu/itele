//! The player screen: entering and leaving it, channel switching,
//! fullscreen, the banner, and its live readout of mpv.

use std::time::Instant;

use slint::{ComponentHandle, Model, ModelRc, TimerMode, VecModel};

use super::{BANNER_TIME, POLL_INTERVAL, Session, with_session};
use crate::ui::Screen;
use crate::{info, playback};

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
        self.show(Screen::Player);
        self.show_banner();
        self.poll();
        self.poll_timer
            .start(TimerMode::Repeated, POLL_INTERVAL, || {
                with_session(|s| s.poll());
            });
    }

    pub(super) fn back(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        self.poll_timer.stop();
        self.show(Screen::Live);
        app.invoke_reveal_current();
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
            self.replay_length(),
        ));
        if app.get_info_visible() {
            let channel = app
                .get_channels()
                .row_data(app.get_channel_index().max(0) as usize);
            let channel = channel.map(|c| c.name).unwrap_or_default();
            let lines = info::read(&self.engine, &channel, &app.get_provider_name());
            app.set_info(ModelRc::new(VecModel::from(lines)));
        }
    }

    pub(super) fn zap(&self, delta: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
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
