//! The player screen: entering and leaving it, channel switching,
//! fullscreen, the banner, and its live readout of mpv.

use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, TimerMode, VecModel};

use super::{BANNER_TIME, Content, POLL_INTERVAL, Session, with_session};
use crate::info;
use crate::playback::{self, Timeline};
use crate::ui::Screen;

/// How often the progress of a title playing is saved.
const SAVE_EVERY: Duration = Duration::from_secs(10);

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
            self.save_progress();
            self.stop_preview();
            self.show(Screen::Details);
            self.refresh_page_history();
        } else {
            self.show(Screen::Live);
            app.invoke_reveal_current();
        }
    }

    /// A movie or an episode played to its end: back to its page, and on
    /// to the next episode when there is one.
    pub(super) fn title_ended(&self) {
        let in_player = self
            .app
            .upgrade()
            .is_some_and(|app| app.get_screen() == Screen::Player);
        if !in_player || self.timeline() != Timeline::Title {
            return;
        }
        let finished = self.playing_entry();
        // At the end mpv has no position left to read.
        self.save_at_end();
        self.back();
        if let Some(entry) = finished.filter(|e| e.kind == itele::history::Kind::Episode) {
            self.play_next_episode(&entry.id);
        }
    }

    /// Saves how far the title playing is, if one is.
    pub(super) fn save_progress(&self) {
        let (Some(position), duration) = (self.engine.position(), self.engine.duration()) else {
            return;
        };
        self.record(position, duration.unwrap_or(0.0));
    }

    /// Saves the title playing as watched to the end.
    fn save_at_end(&self) {
        let length = self.title_length.get();
        if length > 0.0 {
            self.record(length, length);
        }
    }

    fn record(&self, position: f64, duration: f64) {
        let (Some(history), Some(entry)) = (&self.history, self.playing_entry()) else {
            return;
        };
        self.saved_at.set(Instant::now());
        if let Err(e) = history.save(&entry, position, duration, super::timefmt::now()) {
            eprintln!("save progress: {e}");
        }
    }

    fn playing_entry(&self) -> Option<itele::history::Entry> {
        match &self.state.borrow().playing.as_ref()?.content {
            Content::Title(title) => Some(title.entry.clone()),
            Content::Live(_) | Content::Replay(..) => None,
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
        let timeline = self.timeline();
        app.set_player(playback::read(
            &self.engine,
            app.window().is_fullscreen(),
            timeline,
        ));
        if timeline == Timeline::Title {
            if let Some(length) = self.engine.duration() {
                self.title_length.set(length);
            }
            if self.saved_at.get().elapsed() >= SAVE_EVERY {
                self.save_progress();
            }
        }
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
