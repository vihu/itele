//! Loading each provider's movie and series shelves: from the cache on the
//! first visit of Movies or Series, then from the provider when the cache
//! is missing or old.

use std::thread;
use std::time::Duration;

use itele::provider::{Cache, Listing, Provider, Shelf};
use itele::xtream::{Client, Movie, Show};

use super::vod::Kind;
use super::{Session, on_ui_thread};
use crate::names::thousands;
use crate::vod::Source;

/// A shelf younger than this is not downloaded again.
const MAX_AGE: Duration = Duration::from_secs(12 * 3600);

/// How one of a provider's shelves is loading.
#[derive(Default)]
pub(super) struct ShelfState {
    started: bool,
    pub(super) status: String,
}

impl ShelfState {
    /// Whether loading started (it may have finished).
    pub(super) fn started(&self) -> bool {
        self.started
    }
}

/// A shelf that finished loading.
pub(super) enum Loaded {
    Movies(Shelf<Movie>),
    Series(Shelf<Show>),
}

impl Session {
    /// Starts loading `kind` for every provider that has not started yet.
    pub(super) fn load_shelves(&self, kind: Kind) {
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
        if self
            .app
            .upgrade()
            .is_some_and(|app| app.get_screen() == crate::ui::Screen::Search)
        {
            self.run_search();
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
}

impl super::Slot {
    fn shelf_state(&mut self, kind: Kind) -> &mut ShelfState {
        match kind {
            Kind::Movies => &mut self.movies,
            Kind::Series => &mut self.shows,
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
