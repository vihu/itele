//! The programme guide's side of a provider refresh: importing XMLTV in the
//! background, and recording how it went.

use std::collections::HashSet;
use std::io::BufReader;
use std::ops::Range;

use camino::Utf8Path;
use itele::epg::{Programme, Store};
use itele::provider::Library;
use itele::xtream::Client;
use slint::{Model, ModelRc, SharedString, VecModel};

use super::Session;
use super::timefmt::{clock, minutes_left, now};
use crate::live::thousands;
use crate::ui::{ProgrammeInfo, UpcomingItem};

/// A guide younger than this is not downloaded again.
const MAX_AGE: i64 = 12 * 3600;
/// Programmes kept before now: a week of catch-up, plus a day.
const KEEP_BEHIND: i64 = 8 * 86_400;
/// Programmes kept after now.
const KEEP_AHEAD: i64 = 8 * 86_400;
/// How far ahead the preview's "Up next" looks.
const UPCOMING_SPAN: i64 = 12 * 3600;
/// Programmes listed under "Up next".
const UPCOMING_COUNT: usize = 3;

/// The XMLTV ids of `library`'s channels, lowercased.
pub(super) fn wanted_channels(library: &Library) -> HashSet<String> {
    library
        .streams
        .iter()
        .filter_map(|s| s.epg_channel_id.as_deref())
        .map(str::to_lowercase)
        .collect()
}

/// Imports `provider`'s guide into the store at `path` unless it is fresh.
/// Runs on a worker thread. `Ok(None)` means the stored guide was fresh.
pub(super) fn refresh(
    client: &Client,
    path: &Utf8Path,
    provider: &str,
    wanted: &HashSet<String>,
) -> Result<Option<usize>, String> {
    let mut store = Store::open(path).map_err(|e| e.to_string())?;
    let now = jiff::Timestamp::now().as_second();
    let imported = store.imported_at(provider).map_err(|e| e.to_string())?;
    if imported.is_some_and(|at| now - at < MAX_AGE) {
        return Ok(None);
    }
    let xml = client.xmltv().map_err(|e| e.to_string())?;
    let keep = now - KEEP_BEHIND..now + KEEP_AHEAD;
    store
        .import(provider, wanted, keep, BufReader::new(xml), now)
        .map(Some)
        .map_err(|e| e.to_string())
}

impl Session {
    /// Reads now and next again for the rows on screen and the selection.
    pub(super) fn tick_guide(&self) {
        let visible = self.state.borrow().visible.clone();
        self.fill_guide(visible);
        self.refresh_programme();
    }

    /// Fills now and next on `rows` of the channel list from the guide.
    pub(super) fn fill_guide(&self, rows: Range<usize>) {
        let guide = self.guide.borrow();
        let Some(store) = guide.as_ref() else {
            return;
        };
        let now = now();
        let state = self.state.borrow();
        for row in rows {
            let Some(id) = state.row_guide_ids.get(row).filter(|id| !id.is_empty()) else {
                continue;
            };
            let Ok((current, next)) = store.now_next(&state.row_provider, id, now) else {
                continue;
            };
            let Some(mut item) = state.channels.row_data(row) else {
                continue;
            };
            let (now_title, now_progress) =
                current.as_ref().map_or((SharedString::new(), -1.0), |p| {
                    (p.title.as_str().into(), progress(p, now))
                });
            let (next_time, next_title) = next
                .as_ref()
                .map_or((SharedString::new(), SharedString::new()), |p| {
                    (clock(p.start).into(), p.title.as_str().into())
                });
            let changed = item.now_title != now_title
                || item.now_progress != now_progress
                || item.next_title != next_title;
            if changed {
                item.now_title = now_title;
                item.now_progress = now_progress;
                item.next_time = next_time;
                item.next_title = next_title;
                state.channels.set_row_data(row, item);
            }
        }
    }

    /// Shows the selected channel's programme and what follows it.
    pub(super) fn refresh_programme(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let now = now();
        let programmes = {
            let state = self.state.borrow();
            let guide = self.guide.borrow();
            usize::try_from(app.get_channel_index())
                .ok()
                .and_then(|row| state.row_guide_ids.get(row))
                .filter(|id| !id.is_empty())
                .zip(guide.as_ref())
                .and_then(|(id, store)| {
                    store
                        .between(&state.row_provider, id, now, now + UPCOMING_SPAN)
                        .ok()
                })
                .unwrap_or_default()
        };
        let mut programmes = programmes.into_iter().peekable();
        let current = programmes.next_if(|p| p.start <= now);
        let info = current.map_or_else(ProgrammeInfo::default, |p| ProgrammeInfo {
            title: p.title.as_str().into(),
            time: format!("{} – {}", clock(p.start), clock(p.stop)).into(),
            left: minutes_left(p.stop - now).into(),
            description: p.description.as_str().into(),
            progress: progress(&p, now),
        });
        let upcoming: Vec<UpcomingItem> = programmes
            .take(UPCOMING_COUNT)
            .map(|p| UpcomingItem {
                time: clock(p.start).into(),
                title: p.title.as_str().into(),
            })
            .collect();
        app.set_programme(info);
        app.set_upcoming(ModelRc::new(VecModel::from(upcoming)));
    }

    /// Records how `provider`'s guide refresh went.
    pub(super) fn guide_ready(&self, id: &str, epoch: u64, result: Result<Option<usize>, String>) {
        {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.provider.id == id && s.epoch == epoch)
            else {
                return;
            };
            match result {
                Ok(Some(count)) => slot.guide = format!("guide: {} programmes", thousands(count)),
                Ok(None) => {}
                Err(e) => slot.guide = format!("guide failed: {e}"),
            }
        }
        self.refresh_lists();
        self.tick_guide();
    }
}

/// How far into `programme` `now` is, 0 to 1.
fn progress(programme: &Programme, now: i64) -> f32 {
    let length = (programme.stop - programme.start).max(1) as f32;
    ((now - programme.start) as f32 / length).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_clamped() {
        let p = Programme {
            start: 100,
            stop: 200,
            title: String::new(),
            description: String::new(),
        };
        assert_eq!(progress(&p, 150), 0.5);
        assert_eq!(progress(&p, 50), 0.0);
        assert_eq!(progress(&p, 500), 1.0);
    }
}
