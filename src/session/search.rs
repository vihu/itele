//! Search across every provider: channels by name, movies and series by
//! title, programmes by title. Enter plays a channel, the channel of a
//! programme that is on now, or an ended programme from catch-up, and opens
//! the page of a movie or series.

use std::rc::Rc;
use std::time::Duration;

use itele::epg::Programme;
use itele::xtream::{Movie, Show, StreamId};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, TimerMode, VecModel};

use super::catchup::replayable;
use super::timefmt::{day_time, now, when};
use super::vod::Kind;
use super::{Session, with_session};
use crate::art::Size;
use crate::names::{short_name, tint};
use crate::ui::{Screen, SearchData, SearchItem};
use crate::vod::{Tile, TitleHit};

/// Pause after the last keystroke before searching.
const DEBOUNCE: Duration = Duration::from_millis(200);
/// Channels listed at most.
const CHANNEL_LIMIT: usize = 8;
/// Programmes listed at most.
const PROGRAMME_LIMIT: usize = 40;
/// Movies, and series, listed at most.
const TITLE_LIMIT: usize = 8;

/// What each result row leads to, and the rows, which take posters as
/// they arrive.
#[derive(Default)]
pub(super) struct Search {
    targets: Vec<Target>,
    items: Rc<VecModel<SearchItem>>,
    /// Each row's poster URL; empty for rows without one.
    posters: Vec<String>,
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
        programme: Programme,
        live: bool,
        replay: bool,
    },
    Movie {
        provider: String,
        movie: Movie,
    },
    Show {
        provider: String,
        show: Show,
    },
}

impl Session {
    /// `/` or Ctrl+K: focuses the search field in the top bar; results
    /// drop down under it as the user types.
    pub(super) fn focus_search(&self) {
        self.load_shelves(Kind::Movies);
        self.load_shelves(Kind::Series);
        if let Some(app) = self.app.upgrade() {
            app.invoke_focus_search();
        }
        self.run_search();
    }

    /// Opens the Search screen with every result for the query.
    pub(super) fn open_search(&self) {
        // Movies and series are searched once loaded, which starts here if
        // their screens were not visited yet.
        self.load_shelves(Kind::Movies);
        self.load_shelves(Kind::Series);
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
        let query = app.global::<SearchData>().get_query().trim().to_owned();
        let now = now();
        let mut items = Vec::new();
        let mut targets = Vec::new();
        let mut posters = Vec::new();
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
            posters.resize(items.len(), String::new());

            let movies = state.movies.search(&query, TITLE_LIMIT);
            if !movies.is_empty() {
                items.push(heading("Movies"));
                targets.push(Target::Heading);
                posters.push(String::new());
            }
            for hit in movies {
                items.push(title_item(&hit, "Movie"));
                posters.push(hit.title.poster().unwrap_or("").to_owned());
                targets.push(Target::Movie {
                    provider: hit.source.id.clone(),
                    movie: hit.title.clone(),
                });
            }
            let shows = state.shows.search(&query, TITLE_LIMIT);
            if !shows.is_empty() {
                items.push(heading("Series"));
                targets.push(Target::Heading);
                posters.push(String::new());
            }
            for hit in shows {
                items.push(title_item(&hit, "Series"));
                posters.push(hit.title.poster().unwrap_or("").to_owned());
                targets.push(Target::Show {
                    provider: hit.source.id.clone(),
                    show: hit.title.clone(),
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
                let replay = replayable(p, stream.archive_days, now);
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
                    replay,
                });
                targets.push(Target::Programme {
                    provider: source.id.clone(),
                    stream: stream.id,
                    programme: p.clone(),
                    live,
                    replay,
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
        // The Search screen selects the first result; the results under the
        // field start on the field, where Enter opens them all.
        let index = if app.get_screen() == Screen::Search {
            targets
                .iter()
                .position(|t| !matches!(t, Target::Heading))
                .map_or(-1, |i| i as i32)
        } else {
            -1
        };
        posters.resize(items.len(), String::new());
        let items = Rc::new(VecModel::from(items));
        let data = app.global::<SearchData>();
        data.set_note(note.into());
        data.set_items(ModelRc::from(Rc::clone(&items)));
        data.set_index(index);
        {
            let mut art = self.art.borrow_mut();
            for url in posters.iter().filter(|u| !u.is_empty()) {
                art.request(url, Size::Poster);
            }
        }
        {
            let search = &mut self.state.borrow_mut().search;
            search.targets = targets;
            search.items = items;
            search.posters = posters;
        }
        app.invoke_reveal_search_row();
    }

    /// Moves the selection by `delta` results, skipping headings; above
    /// the first, the results under the field go back to the field.
    pub(super) fn search_move(&self, delta: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let state = self.state.borrow();
        let targets = &state.search.targets;
        let data = app.global::<SearchData>();
        let mut index = data.get_index();
        loop {
            let next = index + delta.signum();
            let Some(target) = usize::try_from(next).ok().and_then(|i| targets.get(i)) else {
                if next < 0 && app.get_screen() != Screen::Search {
                    data.set_index(-1);
                }
                break;
            };
            index = next;
            if !matches!(target, Target::Heading) {
                data.set_index(index);
                break;
            }
        }
        drop(state);
        app.invoke_reveal_search_row();
    }

    /// Shows a poster that arrived on every result row that uses it.
    pub(super) fn show_search_poster(&self, url: &str, image: &Image) {
        let state = self.state.borrow();
        let search = &state.search;
        for (row, _) in search.posters.iter().enumerate().filter(|(_, u)| *u == url) {
            if let Some(mut item) = search.items.row_data(row) {
                item.logo = image.clone();
                item.has_logo = true;
                search.items.set_row_data(row, item);
            }
        }
    }

    /// Enter or a click: plays a channel, a programme's channel when the
    /// programme is on now, or the programme from catch-up when it ended;
    /// opens the page of a movie or series.
    pub(super) fn search_picked(&self, index: i32) {
        let title = {
            let state = self.state.borrow();
            match usize::try_from(index)
                .ok()
                .and_then(|i| state.search.targets.get(i))
            {
                Some(Target::Movie { provider, movie }) => {
                    Some((provider.clone(), Some(movie.clone()), None))
                }
                Some(Target::Show { provider, show }) => {
                    Some((provider.clone(), None, Some(show.clone())))
                }
                _ => None,
            }
        };
        match title {
            Some((provider, Some(movie), _)) => {
                return self.open_movie(provider, movie, Screen::Search);
            }
            Some((provider, None, Some(show))) => {
                return self.open_show(provider, show, Screen::Search);
            }
            _ => {}
        }
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
                        ..
                    } => Some((provider.clone(), *stream, None)),
                    Target::Programme {
                        provider,
                        stream,
                        programme,
                        replay: true,
                        ..
                    } => Some((provider.clone(), *stream, Some(programme.clone()))),
                    Target::Heading
                    | Target::Programme { .. }
                    | Target::Movie { .. }
                    | Target::Show { .. } => None,
                })
        };
        let Some((provider, stream, programme)) = chosen else {
            return;
        };
        if !self.select_by_id(&provider, stream) {
            return;
        }
        match programme {
            Some(programme) => self.replay_selected(programme),
            None => self.watch(),
        }
    }

    /// Shows `provider`'s channels in Live TV with `stream` selected;
    /// `false` when the channel is gone.
    fn select_by_id(&self, provider: &str, stream: StreamId) -> bool {
        let Some(app) = self.app.upgrade() else {
            return false;
        };
        let Some(index) = self
            .state
            .borrow()
            .slots
            .iter()
            .position(|s| s.provider.id == provider)
        else {
            return false;
        };
        self.select_view(index as i32);
        let row = {
            let state = self.state.borrow();
            state
                .catalog
                .row_of(state.catalog.first_group(), provider, stream)
        };
        let Some(row) = row else {
            return false;
        };
        app.set_channel_index(row as i32);
        self.refresh_programme();
        app.invoke_reveal_current();
        true
    }
}

/// A movie or series row: `kind` names which, beside the year and, the
/// provider.
fn title_item<T: Tile>(hit: &TitleHit<'_, T>, kind: &str) -> SearchItem {
    let year = hit.title.year().map(|y| y.to_string());
    let subtitle = [Some(kind.to_owned()), year, Some(hit.source.name.clone())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    SearchItem {
        kind: 3,
        title: hit.title.name().into(),
        subtitle: subtitle.into(),
        detail: hit
            .title
            .rating()
            .map(|r| format!("{r:.1} / 10").into())
            .unwrap_or_default(),
        short: short_name(hit.title.name()).into(),
        tint: tint(hit.title.name()),
        ..SearchItem::default()
    }
}

fn heading(title: &str) -> SearchItem {
    SearchItem {
        kind: 2,
        title: title.into(),
        ..SearchItem::default()
    }
}
