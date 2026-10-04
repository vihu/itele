//! Home: the channel watched last, still playing, then the titles left
//! unfinished, the favorite channels with what is on, and the newest
//! movies. Layout: .ai-docs/mockups/01-home.html.

use std::rc::Rc;

use itele::history::{Entry, Kind as Watched, Progress};
use itele::xtream::{Movie, SeriesId, Show, StreamId};
use slint::{Image, Model, ModelRc, VecModel};

use super::Session;
use super::favorite_titles::FALLBACK_EXTENSION;
use super::guide::progress;
use super::timefmt::{clock, minutes_left, now};
use super::vod::Kind;
use crate::art::Size;
use crate::names::tint;
use crate::ui::{ChannelItem, HomeCard, PosterItem, Screen};

/// Cards on each row of Home at most.
const ROW_LIMIT: usize = 12;

/// What Home's rows lead to, and their cards, which take posters as they
/// arrive.
#[derive(Default)]
pub(super) struct Home {
    unfinished: Vec<Unfinished>,
    unfinished_items: Rc<VecModel<HomeCard>>,
    /// Rows of the Favorites group behind the channel cards.
    channel_rows: Vec<usize>,
    movies: Vec<(String, Movie)>,
    movie_items: Rc<VecModel<PosterItem>>,
}

/// A title left unfinished: a movie, or a series by its latest episode.
struct Unfinished {
    entry: Entry,
    /// Its poster URL, once its list has loaded; empty without one.
    poster: String,
}

impl Session {
    /// Opens Home on the channel selected, or else the one watched last.
    pub(super) fn open_home(&self) {
        self.load_shelves(Kind::Movies);
        self.load_shelves(Kind::Series);
        self.select_last_channel();
        self.fill_home();
        self.show(Screen::Home);
        self.resume_preview();
    }

    /// Fills Home's rows from the history, the favorites and the lists.
    pub(super) fn fill_home(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let unfinished = self.unfinished();
        let unfinished_items: Vec<HomeCard> = unfinished
            .iter()
            .map(|(u, progress)| unfinished_card(u, progress))
            .collect();
        let (channel_rows, channels) = self.favorite_channels_on_now();
        let movies = self.newest_movies();
        let movie_items: Vec<PosterItem> = movies
            .iter()
            .map(|(_, m)| PosterItem {
                title: m.name.as_str().into(),
                detail: m.year.map(|y| y.to_string()).unwrap_or_default().into(),
                rating: m
                    .rating
                    .map(|r| format!("{r:.1}").into())
                    .unwrap_or_default(),
                tint: tint(&m.name),
                progress: -1.0,
                ..PosterItem::default()
            })
            .collect();
        {
            let mut art = self.art.borrow_mut();
            for url in unfinished
                .iter()
                .map(|(u, _)| u.poster.as_str())
                .chain(movies.iter().filter_map(|(_, m)| m.poster.as_deref()))
            {
                art.request(url, Size::Poster);
            }
        }
        let unfinished_items = Rc::new(VecModel::from(unfinished_items));
        let movie_items = Rc::new(VecModel::from(movie_items));
        app.set_home_unfinished(ModelRc::from(Rc::clone(&unfinished_items)));
        app.set_home_channels(ModelRc::new(VecModel::from(channels)));
        app.set_home_movies(ModelRc::from(Rc::clone(&movie_items)));
        self.state.borrow_mut().home = Home {
            unfinished: unfinished.into_iter().map(|(u, _)| u).collect(),
            unfinished_items,
            channel_rows,
            movies,
            movie_items,
        };
    }

    /// A card on Home: `row` 1 the titles left unfinished, 2 the favorite
    /// channels, 3 the newest movies.
    pub(super) fn home_open(&self, row: i32, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        match row {
            1 => self.open_unfinished(index),
            2 => {
                let target = {
                    let state = self.state.borrow();
                    state
                        .catalog
                        .favorites_group()
                        .zip(state.home.channel_rows.get(index).copied())
                };
                let Some((group, row)) = target else {
                    return;
                };
                self.select_group(group);
                if let Some(app) = self.app.upgrade() {
                    app.set_channel_index(row as i32);
                }
                self.watch();
            }
            3 => {
                let movie = self.state.borrow().home.movies.get(index).cloned();
                if let Some((provider, movie)) = movie {
                    self.open_movie(provider, movie, Screen::Home);
                }
            }
            _ => {}
        }
    }

    /// Shows a poster that arrived on every Home card that uses it.
    pub(super) fn show_home_poster(&self, url: &str, image: &Image) {
        let state = self.state.borrow();
        let home = &state.home;
        for (row, _) in home
            .unfinished
            .iter()
            .enumerate()
            .filter(|(_, u)| u.poster == url)
        {
            if let Some(mut card) = home.unfinished_items.row_data(row) {
                card.image = image.clone();
                card.has_image = true;
                home.unfinished_items.set_row_data(row, card);
            }
        }
        for (row, _) in home
            .movies
            .iter()
            .enumerate()
            .filter(|(_, (_, m))| m.poster.as_deref() == Some(url))
        {
            if let Some(mut item) = home.movie_items.row_data(row) {
                item.poster = image.clone();
                item.has_poster = true;
                home.movie_items.set_row_data(row, item);
            }
        }
    }
}

// Private API
impl Session {
    /// With no channel selected, selects the one watched last, or else the
    /// first favorite channel: in its provider's groups, so Live TV and the
    /// guide keep listing every channel, else in Favorites.
    fn select_last_channel(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        if app.get_channel_index() >= 0 {
            return;
        }
        let last = self.settings.borrow().last_channel.clone();
        let found = {
            let state = self.state.borrow();
            let catalog = &state.catalog;
            let target = last.map(|l| (l.provider, StreamId(l.stream))).or_else(|| {
                let group = catalog.favorites_group()?;
                (0..catalog.group_len(group))
                    .find_map(|row| catalog.stream(group, row))
                    .map(|(source, stream)| (source.id.clone(), stream.id))
            });
            target.and_then(|(provider, stream)| {
                let groups =
                    (catalog.first_group()..catalog.group_count()).chain(catalog.favorites_group());
                groups
                    .into_iter()
                    .find_map(|g| catalog.row_of(g, &provider, stream).map(|row| (g, row)))
            })
        };
        if let Some((group, row)) = found {
            self.select_group(group);
            app.set_channel_index(row as i32);
            self.refresh_programme();
        }
    }

    /// The titles left unfinished, latest first, with their posters.
    fn unfinished(&self) -> Vec<(Unfinished, Progress)> {
        let entries = self
            .history
            .as_ref()
            .and_then(|h| h.unfinished(ROW_LIMIT).ok())
            .unwrap_or_default();
        let state = self.state.borrow();
        entries
            .into_iter()
            .map(|(entry, progress)| {
                let poster = match entry.series {
                    Some(series) => state
                        .shows
                        .find(&entry.provider, &series.to_string())
                        .and_then(|s| s.poster.clone()),
                    None => state
                        .movies
                        .find(&entry.provider, &entry.id)
                        .and_then(|m| m.poster.clone()),
                };
                let poster = poster.unwrap_or_default();
                (Unfinished { entry, poster }, progress)
            })
            .collect()
    }

    /// The playable favorite channels with what is on now and next, and
    /// their rows in the Favorites group.
    fn favorite_channels_on_now(&self) -> (Vec<usize>, Vec<ChannelItem>) {
        let state = self.state.borrow();
        let Some(group) = state.catalog.favorites_group() else {
            return (Vec::new(), Vec::new());
        };
        let guides = state.catalog.row_guides(group);
        let logos = self.logos.borrow();
        let urls = state.catalog.row_logos(group);
        let guide = self.guide.borrow();
        let now = now();
        state
            .catalog
            .channel_items(group)
            .into_iter()
            .enumerate()
            .filter(|(_, item)| !item.gone)
            .take(ROW_LIMIT)
            .map(|(row, mut item)| {
                if let Some(logo) = urls.get(row).and_then(|u| logos.get(u)) {
                    item.logo = logo;
                    item.has_logo = true;
                }
                let on = guides
                    .get(row)
                    .filter(|k| !k.channel.is_empty())
                    .zip(guide.as_ref())
                    .and_then(|(k, store)| store.now_next(&k.provider, &k.channel, now).ok());
                if let Some((current, next)) = on {
                    if let Some(p) = current {
                        item.now_title = p.title.as_str().into();
                        item.now_progress = progress(&p, now);
                    }
                    if let Some(p) = next {
                        item.next_time = clock(p.start).into();
                        item.next_title = p.title.as_str().into();
                    }
                }
                (row, item)
            })
            .unzip()
    }

    /// The movies added last across every loaded provider.
    fn newest_movies(&self) -> Vec<(String, Movie)> {
        let state = self.state.borrow();
        let mut movies: Vec<_> = state.movies.titles().collect();
        movies.sort_by_key(|(_, m)| std::cmp::Reverse(m.added.unwrap_or(0)));
        movies
            .into_iter()
            .take(ROW_LIMIT)
            .map(|(source, m)| (source.id.clone(), m.clone()))
            .collect()
    }

    /// Opens the page of a title left unfinished: the movie, or the series
    /// at its latest episode.
    fn open_unfinished(&self, index: usize) {
        let entry = self
            .state
            .borrow()
            .home
            .unfinished
            .get(index)
            .map(|u| u.entry.clone());
        let Some(entry) = entry else {
            return;
        };
        let (name, _) = entry
            .title
            .split_once(" \u{b7} ")
            .unwrap_or((entry.title.as_str(), ""));
        match (entry.kind, entry.series) {
            (Watched::Episode, Some(series)) => {
                let show = self
                    .state
                    .borrow()
                    .shows
                    .find(&entry.provider, &series.to_string())
                    .cloned()
                    .unwrap_or_else(|| Show {
                        id: SeriesId(series),
                        name: name.to_owned(),
                        category_id: None,
                        poster: None,
                        rating: None,
                        year: None,
                        updated: None,
                    });
                self.open_show(entry.provider, show, Screen::Home);
            }
            _ => {
                let movie = self
                    .state
                    .borrow()
                    .movies
                    .find(&entry.provider, &entry.id)
                    .cloned()
                    .unwrap_or_else(|| Movie {
                        id: StreamId(entry.id.parse().unwrap_or_default()),
                        name: name.to_owned(),
                        category_id: None,
                        poster: None,
                        rating: None,
                        year: None,
                        added: None,
                        extension: FALLBACK_EXTENSION.to_owned(),
                    });
                self.open_movie(entry.provider, movie, Screen::Home);
            }
        }
    }
}

/// A card for a title left unfinished: `Movie · 1 h 12 min left`, or the
/// series with `S2 E4 · 18 min left`.
fn unfinished_card(unfinished: &Unfinished, progress: &Progress) -> HomeCard {
    let entry = &unfinished.entry;
    let left = minutes_left(progress.left() as i64);
    let (title, detail) = match entry.title.split_once(" \u{b7} ") {
        Some((show, code)) if entry.series.is_some() => (show, format!("{code} \u{b7} {left}")),
        _ => (entry.title.as_str(), format!("Movie \u{b7} {left}")),
    };
    HomeCard {
        title: title.into(),
        detail: detail.into(),
        tint: tint(title),
        image: Image::default(),
        has_image: false,
        progress: progress.fraction(),
    }
}
