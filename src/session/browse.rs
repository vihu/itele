//! The Live TV lists: picking a group or a channel, and starting playback.

use std::rc::Rc;

use camino::Utf8PathBuf;
use slint::{Image, Model, ModelRc, TimerMode, VecModel};

use super::{Playing, SELECT_DELAY, Session, State, with_session};

/// Rows asked for logos when a group opens, before the list reports what
/// it shows.
const FIRST_ROWS: i32 = 16;

impl Session {
    pub(super) fn select_group(&self, group: usize) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let group = group.min(state.catalog.group_count().saturating_sub(1));
        app.set_group_index(group as i32);
        app.set_group_title(state.catalog.group_name(group).into());
        let row_logos = state.catalog.row_logos(group);
        let (row_provider, row_guide_ids) = state.catalog.row_guide_ids(group);
        let row_archive = state.catalog.row_archive(group);
        let mut items = state.catalog.channel_items(group);
        {
            let logos = self.logos.borrow();
            for (item, url) in items.iter_mut().zip(&row_logos) {
                if let Some(logo) = logos.get(url) {
                    item.logo = logo;
                    item.has_logo = true;
                }
            }
        }
        let channels = Rc::new(VecModel::from(items));
        app.set_channels(ModelRc::from(Rc::clone(&channels)));
        let row = state
            .playing
            .as_ref()
            .and_then(|p| state.catalog.row_of(group, &p.provider, p.stream.id));
        app.set_channel_index(row.map_or(-1, |r| r as i32));
        state.group = group;
        state.channels = channels;
        state.row_logos = row_logos;
        state.row_provider = row_provider;
        state.row_guide_ids = row_guide_ids;
        state.row_archive = row_archive;
        state.visible = 0..0;
        drop(state);
        self.refresh_programme();
        self.rows_visible(row.map_or(0, |r| r as i32 - FIRST_ROWS / 2), FIRST_ROWS);
    }

    /// Asks for the logos of rows `first` to `first + count`.
    pub(super) fn rows_visible(&self, first: i32, count: i32) {
        let first = first.max(0) as usize;
        let urls: Vec<String> = {
            let mut state = self.state.borrow_mut();
            let end = (first + count.max(0) as usize).min(state.row_logos.len());
            state.visible = first.min(end)..end;
            state.row_logos.get(first..end).unwrap_or_default().to_vec()
        };
        let visible = self.state.borrow().visible.clone();
        self.fill_guide(visible);
        for url in urls {
            if self.logos.borrow_mut().request(&url) {
                // Already on disk: loaded now.
                if let Some(image) = self.logos.borrow().get(&url) {
                    self.show_logo(&url, image);
                }
            }
        }
    }

    /// A logo download finished; shows it on every row that uses it.
    pub(super) fn logo_ready(&self, url: String, path: Option<Utf8PathBuf>) {
        let image = self.logos.borrow_mut().finish(url.clone(), path);
        if let Some(image) = image {
            self.show_logo(&url, image);
        }
    }

    fn show_logo(&self, url: &str, image: Image) {
        let rows: Vec<usize> = {
            let state = self.state.borrow();
            let rows: Vec<usize> = state
                .row_logos
                .iter()
                .enumerate()
                .filter(|(_, u)| *u == url)
                .map(|(row, _)| row)
                .collect();
            for &row in &rows {
                if let Some(mut item) = state.channels.row_data(row) {
                    item.logo = image.clone();
                    item.has_logo = true;
                    state.channels.set_row_data(row, item);
                }
            }
            rows
        };
        for row in rows {
            self.show_grid_logo(row, &image);
        }
    }

    pub(super) fn select_channel(&self, row: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_channel_index(row);
        self.refresh_programme();
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
        if playing.as_ref().is_some_and(|p| {
            p.provider == source.id && p.stream.id == stream.id && p.replay.is_none()
        }) {
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
                    replay: None,
                });
            }
            Err(_) => app.set_video_note("Could not start playback".into()),
        }
    }
}
