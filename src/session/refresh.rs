//! Keeping each provider fresh: its account on every refresh, and its
//! channel lists, guide and visited movie and series shelves when they are
//! due, at start, every [`CHECK_EVERY`] while itele runs, and when asked.

use std::thread;
use std::time::Duration;

use camino::Utf8PathBuf;
use itele::provider::{Cache, Library, Listing, Provider, Shelf};
use itele::settings::Refresh;
use itele::xtream::{Action, Client, Credentials, Movie, Show};

use super::guide::{self, Plan};
use super::shelves::Loaded;
use super::vod::Kind;
use super::{Session, on_ui_thread};

/// How often refreshes that fell due are looked for while itele runs.
pub(super) const CHECK_EVERY: Duration = Duration::from_secs(15 * 60);

/// Whether a refresh downloads even what is fresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Force {
    No,
    Yes,
}

/// When something is downloaded again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    /// Now, however fresh.
    Now,
    /// Once older than this.
    After(Duration),
    /// Only when it was never downloaded (refresh by hand).
    Missing,
}

/// What one provider's refresh does, decided on the UI thread.
struct Job {
    provider: Provider,
    credentials: Option<Credentials>,
    epoch: u64,
    lists: Due,
    guide: Plan,
    /// Movies and series refresh only once their screen was visited.
    shelves: Vec<Kind>,
}

impl Session {
    /// Refreshes every provider that is not refreshing already.
    pub(super) fn refresh_all(&self, force: Force) {
        let ids: Vec<String> = self
            .state
            .borrow()
            .slots
            .iter()
            .map(|s| s.provider.id.clone())
            .collect();
        for id in ids {
            self.refresh_provider(&id, force, force);
        }
    }

    /// Refreshes `id`'s account, then its lists and guide when due or
    /// forced, and its visited shelves when the lists are.
    pub(super) fn refresh_provider(&self, id: &str, lists: Force, guide: Force) {
        let settings = self.settings.borrow().clone();
        let job = {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state.slots.iter_mut().find(|s| s.provider.id == id) else {
                return;
            };
            if slot.refreshing {
                return;
            }
            slot.refreshing = true;
            let shelves = [Kind::Movies, Kind::Series]
                .into_iter()
                .filter(|&kind| slot.shelf_started(kind))
                .collect();
            Job {
                provider: slot.provider.clone(),
                credentials: slot.credentials.clone(),
                epoch: slot.epoch,
                lists: Due::of(settings.list_refresh, lists),
                guide: Plan {
                    due: Due::of(settings.guide_refresh, guide),
                    keep_days: settings.keep_days,
                    shift_hours: slot.provider.guide_shift,
                },
                shelves,
            }
        };
        let cache = self.paths.cache(&job.provider);
        let guide_path = self.paths.guide_path();
        thread::spawn(move || run(job, &cache, guide_path));
        self.refresh_lists();
    }

    fn refresh_done(&self, id: &str, epoch: u64) {
        if let Some(slot) = self
            .state
            .borrow_mut()
            .slots
            .iter_mut()
            .find(|s| s.provider.id == id && s.epoch == epoch)
        {
            slot.refreshing = false;
        }
        self.refresh_lists();
    }
}

impl Due {
    pub(super) fn of(refresh: Refresh, force: Force) -> Self {
        match (force, refresh.max_age()) {
            (Force::Yes, _) => Due::Now,
            (Force::No, Some(age)) => Due::After(age),
            (Force::No, None) => Due::Missing,
        }
    }

    /// Whether something `age` old is due; `None` is never downloaded.
    pub(super) fn is_due(self, age: Option<Duration>) -> bool {
        match (self, age) {
            (_, None) | (Due::Now, _) => true,
            (Due::After(max), Some(age)) => age >= max,
            (Due::Missing, Some(_)) => false,
        }
    }
}

/// Runs a refresh on a worker thread, reporting each step to the UI thread.
fn run(job: Job, cache: &Cache, guide_path: Utf8PathBuf) {
    let id = job.provider.id.clone();
    let epoch = job.epoch;
    let done = |id: String| on_ui_thread(move |s| s.refresh_done(&id, epoch));
    let credentials = match job.credentials {
        Some(credentials) => credentials,
        None => match job.provider.credentials() {
            Ok(credentials) => {
                let (id, ready) = (id.clone(), credentials.clone());
                on_ui_thread(move |s| s.credentials_ready(&id, epoch, ready));
                credentials
            }
            Err(e) => {
                let failed = id.clone();
                on_ui_thread(move |s| s.refresh_failed(&failed, epoch, &e));
                return done(id);
            }
        },
    };
    let client = Client::new(credentials);

    let cached = Library::from_cache(cache);
    let lists_due = job.lists.is_due(cache.age(Action::LiveStreams));
    let result = match cached {
        Some(cached) if !lists_due => cached.with_fresh_account(&client, cache),
        _ => Library::fetch(&client, cache),
    };
    let wanted = result
        .as_ref()
        .ok()
        .map(guide::wanted_channels)
        .or_else(|| {
            Library::from_cache(cache)
                .as_ref()
                .map(guide::wanted_channels)
        });
    let library_id = id.clone();
    on_ui_thread(move |s| match result {
        Ok(library) => s.library_ready(&library_id, epoch, library),
        Err(e) => s.refresh_failed(&library_id, epoch, &e),
    });

    if let Some(wanted) = wanted {
        let imported = guide::refresh(&client, &guide_path, &id, &wanted, job.guide);
        let guide_id = id.clone();
        on_ui_thread(move |s| s.guide_ready(&guide_id, epoch, imported));
    }

    for kind in job.shelves {
        let fetched = match kind {
            Kind::Movies => fetch_due::<Movie>(job.lists, &client, cache),
            Kind::Series => fetch_due::<Show>(job.lists, &client, cache),
        };
        let shelf_id = id.clone();
        match fetched {
            None => {}
            Some(Ok(loaded)) => {
                on_ui_thread(move |s| s.shelf_ready(&shelf_id, epoch, loaded, false))
            }
            Some(Err(e)) => {
                on_ui_thread(move |s| s.shelf_failed(kind, &shelf_id, epoch, &e.to_string()));
            }
        }
    }
    done(id);
}

/// Fetches `T`'s shelf when its cached list is due; `None` when fresh.
fn fetch_due<T>(due: Due, client: &Client, cache: &Cache) -> Option<itele::provider::Result<Loaded>>
where
    T: Listing,
    Shelf<T>: Into<Loaded>,
{
    due.is_due(cache.age(T::TITLES))
        .then(|| Shelf::<T>::fetch(client, cache).map(Into::into))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_follows_the_setting_and_force() {
        let hour = Duration::from_secs(3600);
        assert_eq!(Due::of(Refresh::Every6Hours, Force::Yes), Due::Now);
        let six = Due::of(Refresh::Every6Hours, Force::No);
        assert!(!six.is_due(Some(5 * hour)));
        assert!(six.is_due(Some(7 * hour)));
        assert!(six.is_due(None), "never downloaded");
        let manual = Due::of(Refresh::Manual, Force::No);
        assert!(!manual.is_due(Some(1000 * hour)));
        assert!(manual.is_due(None));
        assert!(Due::Now.is_due(Some(Duration::ZERO)));
    }
}
