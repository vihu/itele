//! Catch-up: replaying a past programme from the provider's archive.

use itele::epg::Programme;

use super::timefmt::{clock, day_time, now};
use super::{Playing, Session, State};
use crate::ui::ProgrammeInfo;

/// Seconds in a day, for the archive window.
const DAY: i64 = 86_400;

/// Whether `programme` has ended and is still in an archive of
/// `archive_days`.
pub(super) fn replayable(programme: &Programme, archive_days: u32, now: i64) -> bool {
    archive_days > 0
        && programme.stop <= now
        && now - programme.start < i64::from(archive_days) * DAY
}

impl Session {
    /// Plays `programme` on the selected channel from catch-up, in the
    /// player.
    pub(super) fn replay_selected(&self, programme: Programme) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Ok(row) = usize::try_from(app.get_channel_index()) else {
            return;
        };
        {
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
            if !replayable(&programme, stream.archive_days, now()) {
                return;
            }
            let Some(credentials) = slots
                .iter()
                .find(|slot| slot.provider.id == source.id)
                .and_then(|slot| slot.credentials.as_ref())
            else {
                return app.set_video_note("Connecting to the provider…".into());
            };
            let minutes = u32::try_from((programme.stop - programme.start + 59) / 60).unwrap_or(0);
            let url = credentials.timeshift_url(
                stream.id,
                programme.start,
                minutes,
                source.library.account.timezone.as_deref(),
            );
            if self.engine.load(&url).is_err() {
                return app.set_video_note("Could not start the replay".into());
            }
            app.set_video_note("Tuning…".into());
            app.set_provider_name(source.name.as_str().into());
            app.set_programme(ProgrammeInfo {
                title: programme.title.as_str().into(),
                time: format!(
                    "{} – {}",
                    day_time(programme.start, now()),
                    clock(programme.stop)
                )
                .into(),
                left: "Replay".into(),
                description: programme.description.as_str().into(),
                progress: 0.0,
            });
            *playing = Some(Playing {
                provider: source.id.clone(),
                stream: stream.clone(),
                replay: Some(programme),
            });
        }
        self.select_timer.stop();
        self.enter_player();
    }

    /// Whether mpv is playing a programme from catch-up.
    pub(super) fn replaying(&self) -> bool {
        self.replay_length().is_some()
    }

    /// The length in seconds of the programme being replayed, if any.
    pub(super) fn replay_length(&self) -> Option<f64> {
        let state = self.state.borrow();
        let replay = state.playing.as_ref()?.replay.as_ref()?;
        Some((replay.stop - replay.start) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replayable_needs_an_ended_programme_inside_the_archive() {
        let p = Programme {
            start: 0,
            stop: 3600,
            title: String::new(),
            description: String::new(),
        };
        assert!(replayable(&p, 7, 7200));
        assert!(!replayable(&p, 0, 7200), "no archive");
        assert!(!replayable(&p, 7, 1800), "still on");
        assert!(!replayable(&p, 1, DAY + 1), "older than the archive");
    }
}
