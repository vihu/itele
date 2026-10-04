//! Movie and series favorites: the heart on a detail page, and the Movies
//! and Series tabs of the Favorites screen, which keep each title's name,
//! poster and year from when it was added.

use std::rc::Rc;

use itele::favorites::{Favorite, Kind};
use itele::history::Kind as Watched;
use itele::xtream::{Movie, SeriesId, Show, StreamId};
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};

use super::Session;
use super::timefmt::now;
use super::vod::Kind as Shelf;
use crate::art::Size;
use crate::names::tint;
use crate::ui::{PosterItem, Screen, Shell};

/// Container assumed for a movie whose list has not loaded; its details
/// name the real one before it plays.
pub(super) const FALLBACK_EXTENSION: &str = "mp4";

/// The titles on the Favorites screen's tab, which take posters as they
/// arrive.
#[derive(Default)]
pub(super) struct FavoriteTitles {
    list: Vec<Favorite>,
    items: Rc<VecModel<PosterItem>>,
}

impl Session {
    /// Whether `favorite` is saved.
    pub(super) fn is_favorite(&self, favorite: &Favorite) -> bool {
        self.favorites
            .borrow()
            .as_ref()
            .and_then(|store| {
                store
                    .contains(&favorite.provider, favorite.kind, &favorite.id)
                    .ok()
            })
            .unwrap_or(false)
    }

    /// `F` or the heart on a detail page: the title joins the favorites,
    /// or leaves.
    pub(super) fn toggle_page_favorite(&self) {
        let favorite = self.state.borrow().page.as_ref().map(|p| p.as_favorite());
        let Some(favorite) = favorite else {
            return;
        };
        let keep = !self.is_favorite(&favorite);
        if let Some(store) = self.favorites.borrow().as_ref() {
            let result = if keep {
                store.add(&favorite, now())
            } else {
                store.remove(&favorite.provider, favorite.kind, &favorite.id)
            };
            if let Err(e) = result {
                eprintln!("favorites: {e}");
            }
        }
        if let Some(page) = self.state.borrow_mut().page.as_mut() {
            page.favorite = keep;
        }
        self.push_page();
        self.push_favorite_counts();
    }

    /// Shows a tab of the Favorites screen: 0 channels, 1 movies, 2 series.
    pub(super) fn favorites_tab(&self, tab: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_favorites_tab(tab.rem_euclid(3));
        self.fill_favorite_titles();
        app.invoke_focus_screen();
    }

    /// Lists the favorites of the tab showing, when it is Movies or Series,
    /// limited by the provider filter.
    pub(super) fn fill_favorite_titles(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(kind) = tab_kind(app.get_favorites_tab()) else {
            return;
        };
        // Opening one needs its provider's list.
        self.load_shelves(match kind {
            Kind::Series => Shelf::Series,
            _ => Shelf::Movies,
        });
        let list: Vec<Favorite> = {
            let state = self.state.borrow();
            let filter = state.catalog.favorites_filter();
            self.favorites
                .borrow()
                .as_ref()
                .and_then(|store| store.list(kind).ok())
                .unwrap_or_default()
                .into_iter()
                .filter(|f| filter.is_none_or(|p| f.provider == p))
                .collect()
        };
        let items: Vec<PosterItem> = {
            let state = self.state.borrow();
            list.iter()
                .map(|f| {
                    let provider = state
                        .slots
                        .iter()
                        .find(|s| s.provider.id == f.provider)
                        .map_or(f.provider.as_str(), |s| s.provider.name.as_str());
                    let mut item = PosterItem {
                        title: f.title.as_str().into(),
                        detail: f.year.map(|y| y.to_string()).unwrap_or_default().into(),
                        rating: SharedString::new(),
                        tint: tint(&f.title),
                        poster: Image::default(),
                        has_poster: false,
                        progress: -1.0,
                        watched: false,
                        provider: provider.into(),
                        hue: state.catalog.hue_of(&f.provider),
                    };
                    if kind == Kind::Movie
                        && let Some(progress) = self.history.as_ref().and_then(|h| {
                            h.progress(&f.provider, Watched::Movie, &f.id)
                                .ok()
                                .flatten()
                        })
                    {
                        item.watched = progress.watched;
                        if progress.resume_at().is_some() {
                            item.progress = progress.fraction();
                        }
                    }
                    item
                })
                .collect()
        };
        {
            let mut art = self.art.borrow_mut();
            for f in &list {
                art.request(&f.poster, Size::Poster);
            }
        }
        let count = items.len() as i32;
        let items = Rc::new(VecModel::from(items));
        app.set_favorite_titles(ModelRc::from(Rc::clone(&items)));
        app.set_favorite_title_index(app.get_favorite_title_index().clamp(0, (count - 1).max(0)));
        self.state.borrow_mut().favorite_titles = FavoriteTitles { list, items };
    }

    /// Enter or a click on a favorite title: opens its page.
    pub(super) fn open_favorite_title(&self, index: i32) {
        let favorite = {
            let state = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|i| state.favorite_titles.list.get(i).cloned())
        };
        let Some(f) = favorite else {
            return;
        };
        let poster = Some(f.poster.clone()).filter(|p| !p.is_empty());
        match f.kind {
            Kind::Movie => {
                let movie = self
                    .state
                    .borrow()
                    .movies
                    .find(&f.provider, &f.id)
                    .cloned()
                    .unwrap_or_else(|| Movie {
                        id: StreamId(f.id.parse().unwrap_or_default()),
                        name: f.title.clone(),
                        category_id: None,
                        poster,
                        rating: None,
                        year: f.year,
                        added: None,
                        extension: FALLBACK_EXTENSION.to_owned(),
                    });
                self.open_movie(f.provider, movie, Screen::Favorites);
            }
            Kind::Series => {
                let show = self
                    .state
                    .borrow()
                    .shows
                    .find(&f.provider, &f.id)
                    .cloned()
                    .unwrap_or_else(|| Show {
                        id: SeriesId(f.id.parse().unwrap_or_default()),
                        name: f.title.clone(),
                        category_id: None,
                        poster,
                        rating: None,
                        year: f.year,
                        updated: None,
                    });
                self.open_show(f.provider, show, Screen::Favorites);
            }
            Kind::Channel => {}
        }
    }

    /// `F`, Delete or the heart on a favorite title: it leaves the list.
    pub(super) fn remove_favorite_title(&self, index: i32) {
        let favorite = {
            let state = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|i| state.favorite_titles.list.get(i).cloned())
        };
        let Some(f) = favorite else {
            return;
        };
        if let Some(store) = self.favorites.borrow().as_ref()
            && let Err(e) = store.remove(&f.provider, f.kind, &f.id)
        {
            eprintln!("favorites: {e}");
        }
        self.fill_favorite_titles();
        self.push_favorite_counts();
    }

    /// Shows a poster that arrived on every favorite title that uses it.
    pub(super) fn show_favorite_poster(&self, url: &str, image: &Image) {
        let state = self.state.borrow();
        let titles = &state.favorite_titles;
        for (row, _) in titles
            .list
            .iter()
            .enumerate()
            .filter(|(_, f)| f.poster == url)
        {
            if let Some(mut item) = titles.items.row_data(row) {
                item.poster = image.clone();
                item.has_poster = true;
                titles.items.set_row_data(row, item);
            }
        }
    }

    /// Counts the favorites of each kind, for the sidebar and the tabs.
    pub(super) fn push_favorite_counts(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let count = |kind| {
            self.favorites
                .borrow()
                .as_ref()
                .and_then(|store| store.list(kind).ok())
                .map_or(0, |list| list.len() as i32)
        };
        let (channels, movies, series) = (
            count(Kind::Channel),
            count(Kind::Movie),
            count(Kind::Series),
        );
        app.set_favorite_counts(ModelRc::new(VecModel::from(vec![channels, movies, series])));
        app.global::<Shell>()
            .set_favorite_count(channels + movies + series);
    }
}

/// The kind of favorite a tab of the Favorites screen lists.
fn tab_kind(tab: i32) -> Option<Kind> {
    match tab {
        1 => Some(Kind::Movie),
        2 => Some(Kind::Series),
        _ => None,
    }
}
