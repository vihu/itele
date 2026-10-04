//! Search across every provider: channels by name, programmes by title.
//! Enter plays a channel, or the channel of a programme that is on now.

use std::time::Duration;

use itele::xtream::StreamId;
use slint::{ModelRc, SharedString, TimerMode, VecModel};

use super::timefmt::{day_time, now, when};
use super::{Session, with_session};
use crate::live::{short_name, tint};
use crate::ui::{Screen, SearchItem};

/// Pause after the last keystroke before searching.
const DEBOUNCE: Duration = Duration::from_millis(200);
/// Channels listed at most.
const CHANNEL_LIMIT: usize = 8;
/// Programmes listed at most.
const PROGRAMME_LIMIT: usize = 40;

/// What each result row leads to.
#[derive(Default)]
pub(super) struct Search {
    targets: Vec<Target>,
}

enum Target {
    Heading,
    Channel {
        provider: String,
        stream: StreamId,
    },
    Programme {
        provider: String,
        stream: StreamId,
        live: bool,
    },
}

impl Session {
    pub(super) fn open_search(&self) {
        self.show(Screen::Search);
        self.run_search();
    }

    pub(super) fn search_edited(&self) {
        self.search_timer
            .start(TimerMode::SingleShot, DEBOUNCE, || {
                with_session(|s| s.run_search());
            });
    }

    /// Runs the query in the search field.
    pub(super) fn run_search(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let query = app.get_search_query().trim().to_owned();
        let now = now();
        let mut items = Vec::new();
        let mut targets = Vec::new();
        if !query.is_empty() {
            let state = self.state.borrow();
            let mut logos = self.logos.borrow_mut();
            let mut logo = |url: Option<&str>| {
                let url = url.unwrap_or("");
                logos.request(url);
                logos.get(url)
            };

            let channels = state.catalog.search_channels(&query, CHANNEL_LIMIT);
            if !channels.is_empty() {
                items.push(heading("Channels"));
                targets.push(Target::Heading);
            }
            for hit in channels {
                let image = logo(hit.stream.icon.as_deref());
                let subtitle = match hit.category {
                    "" => hit.source.name.clone(),
                    category => format!("{category} · {}", hit.source.name),
                };
                items.push(SearchItem {
                    kind: 0,
                    title: hit.stream.name.as_str().into(),
                    subtitle: subtitle.into(),
                    detail: SharedString::new(),
                    short: short_name(&hit.stream.name).into(),
                    tint: tint(&hit.stream.name),
                    has_logo: image.is_some(),
                    logo: image.unwrap_or_default(),
                    live: false,
                    replay: false,
                });
                targets.push(Target::Channel {
                    provider: hit.source.id.clone(),
                    stream: hit.stream.id,
                });
            }

            let hits = self
                .guide
                .borrow()
                .as_ref()
                .and_then(|store| store.search(&query, now, PROGRAMME_LIMIT).ok())
                .unwrap_or_default();
            let mut first = true;
            for hit in hits {
                let Some((source, stream)) = state.catalog.by_guide_id(&hit.provider, &hit.channel)
                else {
                    continue;
                };
                if first {
                    items.push(heading("Programmes"));
                    targets.push(Target::Heading);
                    first = false;
                }
                let p = &hit.programme;
                let live = p.start <= now && now < p.stop;
                let replayable =
                    p.stop <= now && now - p.start < i64::from(stream.archive_days) * 86_400;
                let image = logo(stream.icon.as_deref());
                items.push(SearchItem {
                    kind: 1,
                    title: p.title.as_str().into(),
                    subtitle: format!("{} · {}", stream.name, source.name).into(),
                    detail: format!("{}  ·  {}", day_time(p.start, now), when(p, now)).into(),
                    short: short_name(&stream.name).into(),
                    tint: tint(&stream.name),
                    has_logo: image.is_some(),
                    logo: image.unwrap_or_default(),
                    live,
                    replay: replayable,
                });
                targets.push(Target::Programme {
                    provider: source.id.clone(),
                    stream: stream.id,
                    live,
                });
            }
        }

        let note = if query.is_empty() {
            String::new()
        } else if items.is_empty() {
            format!("Nothing found for \u{201c}{query}\u{201d}.")
        } else {
            String::new()
        };
        let index = targets
            .iter()
            .position(|t| !matches!(t, Target::Heading))
            .map_or(-1, |i| i as i32);
        app.set_search_note(note.into());
        app.set_search_items(ModelRc::new(VecModel::from(items)));
        app.set_search_index(index);
        self.state.borrow_mut().search.targets = targets;
        app.invoke_reveal_search_row();
    }

    /// Moves the selection by `delta` results, skipping headings.
    pub(super) fn search_move(&self, delta: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let state = self.state.borrow();
        let targets = &state.search.targets;
        let mut index = app.get_search_index();
        loop {
            let next = index + delta.signum();
            let Some(target) = usize::try_from(next).ok().and_then(|i| targets.get(i)) else {
                break;
            };
            index = next;
            if !matches!(target, Target::Heading) {
                app.set_search_index(index);
                break;
            }
        }
        drop(state);
        app.invoke_reveal_search_row();
    }

    /// Enter or a click: plays a channel, or a programme's channel when the
    /// programme is on now.
    pub(super) fn search_picked(&self, index: i32) {
        let chosen = {
            let state = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|i| state.search.targets.get(i))
                .and_then(|target| match target {
                    Target::Channel { provider, stream }
                    | Target::Programme {
                        provider,
                        stream,
                        live: true,
                    } => Some((provider.clone(), *stream)),
                    Target::Heading | Target::Programme { live: false, .. } => None,
                })
        };
        if let Some((provider, stream)) = chosen {
            self.play_channel(&provider, stream);
        }
    }

    /// Shows `provider`'s channels in Live TV with `stream` selected, and
    /// watches it.
    pub(super) fn play_channel(&self, provider: &str, stream: StreamId) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(index) = self
            .state
            .borrow()
            .slots
            .iter()
            .position(|s| s.provider.id == provider)
        else {
            return;
        };
        self.select_view(index as i32);
        let row = self.state.borrow().catalog.row_of(0, provider, stream);
        let Some(row) = row else {
            return;
        };
        app.set_channel_index(row as i32);
        self.refresh_programme();
        app.invoke_reveal_current();
        self.watch();
    }
}

fn heading(title: &str) -> SearchItem {
    SearchItem {
        kind: 2,
        title: title.into(),
        ..SearchItem::default()
    }
}
