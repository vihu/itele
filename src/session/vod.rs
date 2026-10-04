//! Movies and Series: each provider's shelves, loaded on the first visit,
//! the groups, and the poster grid with its lazy, bounded posters.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use itele::provider::{Cache, Listing, Provider, Shelf};
use itele::xtream::{Client, Movie, Show};
use slint::{Image, Model, ModelRc, SharedString, VecModel};

use super::{Session, State, on_ui_thread};
use crate::art::{Picture, Size};
use crate::live::View;
use crate::names::thousands;
use crate::ui::{PosterItem, Screen};
use crate::vod::{Shelves, Source};

/// A shelf younger than this is not downloaded again.
const MAX_AGE: Duration = Duration::from_secs(12 * 3600);
/// Titles asked for posters when a group opens, before the grid reports
/// what it shows.
const FIRST_TITLES: i32 = 40;

/// Which of the two screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Movies,
    Series,
}

/// How one of a provider's shelves is loading.
#[derive(Default)]
pub(super) struct ShelfState {
    started: bool,
    status: String,
}

/// The poster grid.
#[derive(Default)]
pub(super) struct Browse {
    /// The kind showing, or last shown.
    kind: Option<Kind>,
    /// Each kind's selected group and title, kept while the other shows.
    groups: [usize; 2],
    indexes: [i32; 2],
    items: Rc<VecModel<PosterItem>>,
    /// Each item's poster URL, empty when it has none.
    urls: Vec<String>,
    /// Items holding a picture.
    shown: HashSet<usize>,
    /// Items allowed to hold one: those on screen and a screen either side.
    window: Range<usize>,
}

/// A shelf that finished loading.
pub(super) enum Loaded {
    Movies(Shelf<Movie>),
    Series(Shelf<Show>),
}

impl Session {
    /// Shows Movies or Series, loading the shelves not loaded yet.
    pub(super) fn open_vod(&self, kind: Kind) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        self.stop_preview();
        {
            let mut state = self.state.borrow_mut();
            if let Some(shown) = state.browse.kind {
                state.browse.indexes[shown.index()] = app.get_vod_index();
            }
            state.browse.kind = Some(kind);
        }
        self.load_shelves(kind);
        app.set_vod_focus_groups(false);
        self.show(kind.screen());
        self.refresh_vod();
    }

    /// Pushes the groups and status of the kind showing, keeping the
    /// selected group and title where they still exist.
    pub(super) fn refresh_vod(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(kind) = self.state.borrow().browse.kind else {
            return;
        };
        if app.get_screen() == kind.screen() {
            self.state.borrow_mut().browse.indexes[kind.index()] = app.get_vod_index();
        }
        let (groups, group, index, status) = {
            let mut state = self.state.borrow_mut();
            let view = state.catalog.view().clone();
            state.shelves_mut(kind).set_view(view);
            let shelves = state.shelves(kind);
            let count = shelves.group_count();
            (
                shelves.group_items(),
                state.browse.groups[kind.index()].min(count.saturating_sub(1)),
                state.browse.indexes[kind.index()],
                vod_status(&state, kind),
            )
        };
        app.set_vod_groups(ModelRc::new(VecModel::from(groups)));
        app.set_vod_status(status.as_str().into());
        app.set_vod_note(status.into());
        self.select_vod_group(group, index);
    }

    /// A group picked in the column: its titles, from the first.
    pub(super) fn vod_group_selected(&self, group: i32) {
        self.select_vod_group(group.max(0) as usize, 0);
    }

    /// Titles `first` to `first + count` are on screen: asks for their
    /// posters and drops those far off screen.
    pub(super) fn vod_items_visible(&self, first: i32, count: i32) {
        let urls: Vec<String> = {
            let mut state = self.state.borrow_mut();
            let browse = &mut state.browse;
            let len = browse.urls.len();
            let count = count.max(0) as usize;
            let first = (first.max(0) as usize).min(len);
            let end = (first + count).min(len);
            let window = first.saturating_sub(count)..(end + count).min(len);
            let far: Vec<usize> = browse
                .shown
                .iter()
                .copied()
                .filter(|i| !window.contains(i))
                .collect();
            for i in far {
                browse.shown.remove(&i);
                if let Some(mut item) = browse.items.row_data(i) {
                    item.poster = Image::default();
                    item.has_poster = false;
                    browse.items.set_row_data(i, item);
                }
            }
            browse.window = window.clone();
            // On screen first, then the margin either side.
            let order = (first..end).chain(window.filter(|i| !(first..end).contains(i)));
            let mut seen = HashSet::new();
            order
                .filter(|i| !browse.shown.contains(i))
                .map(|i| browse.urls[i].clone())
                .filter(|url| !url.is_empty() && seen.insert(url.clone()))
                .collect()
        };
        let mut art = self.art.borrow_mut();
        for url in urls {
            art.request(&url, Size::Poster);
        }
    }

    /// A picture arrived; shows it wherever it is still wanted.
    pub(super) fn art_ready(&self, url: String, size: Size, picture: Option<Picture>) {
        self.art.borrow_mut().finish(&url, size);
        let Some(image) = picture.and_then(Picture::image) else {
            return;
        };
        if size == Size::Poster {
            self.show_poster(&url, &image);
        }
        self.page_art_ready(&url, size, &image);
    }

    /// Enter or a click on a title: opens its page.
    pub(super) fn vod_open(&self, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let (movie, show) = {
            let state = self.state.borrow();
            let Some(kind) = state.browse.kind else {
                return;
            };
            let group = state.browse.groups[kind.index()];
            match kind {
                Kind::Movies => (
                    state
                        .movies
                        .title(group, index)
                        .map(|(source, movie)| (source.id.clone(), movie.clone())),
                    None,
                ),
                Kind::Series => (
                    None,
                    state
                        .shows
                        .title(group, index)
                        .map(|(source, show)| (source.id.clone(), show.clone())),
                ),
            }
        };
        if let Some((provider, movie)) = movie {
            self.open_movie(provider, movie);
        } else if let Some((provider, show)) = show {
            self.open_show(provider, show);
        }
    }

    /// Stops the live preview, for screens without one.
    pub(super) fn stop_preview(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        self.select_timer.stop();
        if self.state.borrow_mut().playing.take().is_some() {
            if let Err(e) = self.engine.stop() {
                eprintln!("stop preview: {e}");
            }
            app.set_frame(Image::default());
            app.set_video_note(SharedString::new());
        }
    }

    /// Tunes the selected channel again after [`Session::stop_preview`].
    pub(super) fn resume_preview(&self) {
        if self.state.borrow().playing.is_none() {
            self.play_selected();
        }
    }
}

// Private API
impl Session {
    /// Starts loading `kind` for every provider that has not started yet.
    fn load_shelves(&self, kind: Kind) {
        let mut state = self.state.borrow_mut();
        for slot in &mut state.slots {
            let shelf = slot.shelf_state(kind);
            if shelf.started {
                continue;
            }
            shelf.started = true;
            shelf.status = format!("Loading {}…", kind.noun(2));
            let cache = self.paths.cache(&slot.provider);
            let (provider, epoch) = (slot.provider.clone(), slot.epoch);
            match kind {
                Kind::Movies => load::<Movie>(kind, provider, cache, epoch),
                Kind::Series => load::<Show>(kind, provider, cache, epoch),
            }
        }
    }

    fn shelf_ready(&self, id: &str, epoch: u64, loaded: Loaded, updating: bool) {
        let kind = loaded.kind();
        {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.provider.id == id && s.epoch == epoch)
            else {
                return;
            };
            let count = loaded.len();
            slot.shelf_state(kind).status = format!(
                "{} {}{}",
                thousands(count),
                kind.noun(count),
                if updating { " · updating…" } else { "" }
            );
            let name = slot.provider.name.clone();
            let id = id.to_owned();
            match loaded {
                Loaded::Movies(shelf) => state.movies.upsert(Source { id, name, shelf }),
                Loaded::Series(shelf) => state.shows.upsert(Source { id, name, shelf }),
            }
        }
        if self.state.borrow().browse.kind == Some(kind) {
            self.refresh_vod();
        }
    }

    fn shelf_failed(&self, kind: Kind, id: &str, epoch: u64, error: &str) {
        {
            let mut state = self.state.borrow_mut();
            let loaded = state.shelves(kind).count_of(id);
            let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.provider.id == id && s.epoch == epoch)
            else {
                return;
            };
            slot.shelf_state(kind).status = match loaded {
                Some(count) => format!(
                    "{} {} · could not update: {error}",
                    thousands(count),
                    kind.noun(count)
                ),
                None => format!("Could not load {}: {error}", kind.noun(2)),
            };
        }
        if self.state.borrow().browse.kind == Some(kind) {
            self.refresh_vod();
        }
    }

    /// Lists `group`'s titles with title `index` selected.
    fn select_vod_group(&self, group: usize, index: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(kind) = self.state.borrow().browse.kind else {
            return;
        };
        let (items, name) = {
            let mut state = self.state.borrow_mut();
            let shelves = state.shelves(kind);
            let items = Rc::new(VecModel::from(shelves.poster_items(group)));
            let urls = shelves.posters(group);
            let name = shelves.group_name(group).to_owned();
            let browse = &mut state.browse;
            browse.groups[kind.index()] = group;
            browse.items = Rc::clone(&items);
            browse.urls = urls;
            browse.shown.clear();
            browse.window = 0..0;
            (items, name)
        };
        let len = items.row_count() as i32;
        let index = if len == 0 {
            -1
        } else {
            index.clamp(0, len - 1)
        };
        app.set_vod_items(ModelRc::from(items));
        app.set_vod_group_index(group as i32);
        app.set_vod_group_title(name.into());
        app.set_vod_index(index);
        app.invoke_reveal_vod();
        self.vod_items_visible(index - FIRST_TITLES / 2, FIRST_TITLES);
    }

    fn show_poster(&self, url: &str, image: &Image) {
        let mut state = self.state.borrow_mut();
        let browse = &mut state.browse;
        let rows: Vec<usize> = browse
            .window
            .clone()
            .filter(|&i| browse.urls.get(i).is_some_and(|u| u == url))
            .collect();
        for i in rows {
            if let Some(mut item) = browse.items.row_data(i) {
                item.poster = image.clone();
                item.has_poster = true;
                browse.items.set_row_data(i, item);
                browse.shown.insert(i);
            }
        }
    }
}

impl State {
    fn shelves(&self, kind: Kind) -> &dyn Shelves {
        match kind {
            Kind::Movies => &self.movies,
            Kind::Series => &self.shows,
        }
    }

    fn shelves_mut(&mut self, kind: Kind) -> &mut dyn Shelves {
        match kind {
            Kind::Movies => &mut self.movies,
            Kind::Series => &mut self.shows,
        }
    }
}

impl super::Slot {
    fn shelf_state(&mut self, kind: Kind) -> &mut ShelfState {
        match kind {
            Kind::Movies => &mut self.movies,
            Kind::Series => &mut self.shows,
        }
    }
}

impl Kind {
    const fn index(self) -> usize {
        match self {
            Kind::Movies => 0,
            Kind::Series => 1,
        }
    }

    pub(super) const fn screen(self) -> Screen {
        match self {
            Kind::Movies => Screen::Movies,
            Kind::Series => Screen::Series,
        }
    }

    const fn noun(self, count: usize) -> &'static str {
        match (self, count) {
            (Kind::Movies, 1) => "movie",
            (Kind::Movies, _) => "movies",
            (Kind::Series, _) => "series",
        }
    }
}

impl Loaded {
    const fn kind(&self) -> Kind {
        match self {
            Loaded::Movies(_) => Kind::Movies,
            Loaded::Series(_) => Kind::Series,
        }
    }

    fn len(&self) -> usize {
        match self {
            Loaded::Movies(shelf) => shelf.titles.len(),
            Loaded::Series(shelf) => shelf.titles.len(),
        }
    }
}

impl From<Shelf<Movie>> for Loaded {
    fn from(shelf: Shelf<Movie>) -> Self {
        Loaded::Movies(shelf)
    }
}

impl From<Shelf<Show>> for Loaded {
    fn from(shelf: Shelf<Show>) -> Self {
        Loaded::Series(shelf)
    }
}

/// The status line under the screen's title: the viewed provider's, or a
/// summary of all of them.
fn vod_status(state: &State, kind: Kind) -> String {
    let status = |slot: &super::Slot| match kind {
        Kind::Movies => slot.movies.status.clone(),
        Kind::Series => slot.shows.status.clone(),
    };
    match state.catalog.view() {
        View::One(id) => state
            .slots
            .iter()
            .find(|slot| &slot.provider.id == id)
            .map(status)
            .unwrap_or_default(),
        View::All => match state.slots.as_slice() {
            [] => String::new(),
            [slot] => status(slot),
            slots => {
                let total = state.shelves(kind).total();
                format!(
                    "{} providers · {} {}",
                    slots.len(),
                    thousands(total),
                    kind.noun(total)
                )
            }
        },
    }
}

/// Loads a shelf on a worker thread: from the cache first, then from the
/// provider when the cache is missing or older than [`MAX_AGE`].
fn load<T>(kind: Kind, provider: Provider, cache: Cache, epoch: u64)
where
    T: Listing + Send + 'static,
    Shelf<T>: Into<Loaded>,
{
    thread::spawn(move || {
        let id = provider.id.clone();
        let fresh = cache.age(T::TITLES).is_some_and(|age| age < MAX_AGE);
        let cached = Shelf::<T>::from_cache(&cache);
        let had_cache = cached.is_some();
        if let Some(shelf) = cached {
            let id = id.clone();
            let loaded = shelf.into();
            on_ui_thread(move |s| s.shelf_ready(&id, epoch, loaded, !fresh));
        }
        if had_cache && fresh {
            return;
        }
        let result = provider
            .credentials()
            .and_then(|credentials| Shelf::<T>::fetch(&Client::new(credentials), &cache));
        on_ui_thread(move |s| match result {
            Ok(shelf) => s.shelf_ready(&id, epoch, shelf.into(), false),
            Err(e) => s.shelf_failed(kind, &id, epoch, &e.to_string()),
        });
    });
}
