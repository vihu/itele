//! The Live TV lists: picking a group or a channel, and starting playback.

use slint::{ModelRc, TimerMode, VecModel};

use super::{Playing, SELECT_DELAY, Session, State, with_session};

impl Session {
    pub(super) fn select_group(&self, group: usize) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let group = group.min(state.catalog.group_count().saturating_sub(1));
        app.set_group_index(group as i32);
        app.set_group_title(state.catalog.group_name(group).into());
        app.set_channels(ModelRc::new(VecModel::from(
            state.catalog.channel_items(group),
        )));
        let row = state
            .playing
            .as_ref()
            .and_then(|p| state.catalog.row_of(group, &p.provider, p.stream.id));
        app.set_channel_index(row.map_or(-1, |r| r as i32));
        state.group = group;
    }

    pub(super) fn select_channel(&self, row: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_channel_index(row);
        self.select_timer
            .start(TimerMode::SingleShot, SELECT_DELAY, || {
                with_session(|s| s.play_selected());
            });
    }

    /// Plays the selected channel with its own provider's login, unless it
    /// is already playing.
    pub(super) fn play_selected(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Ok(row) = usize::try_from(app.get_channel_index()) else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let State {
            catalog,
            slots,
            group,
            playing,
            ..
        } = &mut *state;
        let Some((source, stream)) = catalog.stream(*group, row) else {
            return;
        };
        if playing
            .as_ref()
            .is_some_and(|p| p.provider == source.id && p.stream.id == stream.id)
        {
            return;
        }
        let Some(credentials) = slots
            .iter()
            .find(|slot| slot.provider.id == source.id)
            .and_then(|slot| slot.credentials.as_ref())
        else {
            return app.set_video_note("Connecting to the provider…".into());
        };
        let url = credentials.live_url(stream.id, source.library.account.preferred_format());
        match self.engine.load(&url) {
            Ok(()) => {
                app.set_video_note("Tuning…".into());
                app.set_provider_name(source.name.as_str().into());
                *playing = Some(Playing {
                    provider: source.id.clone(),
                    stream: stream.clone(),
                });
            }
            Err(_) => app.set_video_note("Could not start playback".into()),
        }
    }
}
