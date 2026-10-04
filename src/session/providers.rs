//! Signed-in providers: opening the saved ones, login, sign out, the
//! background refresh, and the provider switcher.

use std::thread;

use itele::provider::{self, Library, Provider};
use itele::xtream::{self, Action, Client, Credentials};

use super::refresh::Force;
use slint::{Image, ModelRc, SharedString, VecModel};

use super::{Session, Slot, host_of, initials, library_status, on_ui_thread};
use crate::live::{Source, View};
use crate::ui::{ProviderItem, Screen};
use crate::vod::Shelves;

impl Session {
    /// Opens every saved provider, or the login screen when there is none.
    pub(super) fn open_saved(&self) {
        match self.paths.load_providers() {
            Ok(providers) if !providers.is_empty() => {
                for provider in providers {
                    self.open(provider);
                }
                self.refresh_lists();
                self.show(Screen::Live);
            }
            Ok(_) => self.show_login(""),
            Err(e) => self.show_login(&e.to_string()),
        }
    }

    pub(super) fn login(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let credentials = match Credentials::new(
            &app.get_login_server(),
            app.get_login_username().trim(),
            app.get_login_password(),
        ) {
            Ok(credentials) => credentials,
            Err(e) => return app.set_login_error(e.to_string().into()),
        };
        app.set_login_error(SharedString::new());
        app.set_login_busy(true);

        // Only the account is checked here; the channel list, which can be
        // tens of megabytes, loads in the background once Live TV shows.
        let name = self.automatic_name(credentials.server(), credentials.username());
        let paths = self.paths.clone();
        thread::spawn(move || {
            let provider = Provider::new(name, &credentials);
            let result = Client::new(credentials.clone())
                .fetch(Action::Account)
                .and_then(|body| xtream::parse_account(&body))
                .map_err(provider::Error::Xtream)
                .and_then(|_| {
                    provider.save_password(credentials.password())?;
                    paths.add_provider(&provider)?;
                    Ok(provider)
                });
            on_ui_thread(move |s| s.logged_in(result));
        });
    }

    pub(super) fn add_provider(&self) {
        if let Some(app) = self.app.upgrade() {
            app.set_login_server(SharedString::new());
            app.set_login_username(SharedString::new());
            app.set_login_password(SharedString::new());
        }
        self.show_login("");
    }

    pub(super) fn cancel_login(&self) {
        if !self.state.borrow().slots.is_empty() {
            self.show(Screen::Live);
        }
    }

    /// Forgets the provider at `index`: password, cache, and saved entry.
    pub(super) fn sign_out(&self, index: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (slot, was_playing) = {
            let mut state = self.state.borrow_mut();
            let Some(index) = usize::try_from(index)
                .ok()
                .filter(|&i| i < state.slots.len())
            else {
                return;
            };
            let slot = state.slots.remove(index);
            state.catalog.remove(&slot.provider.id);
            state.movies.remove(&slot.provider.id);
            state.shows.remove(&slot.provider.id);
            let was_playing = state
                .playing
                .as_ref()
                .is_some_and(|p| p.provider == slot.provider.id);
            if was_playing {
                state.playing = None;
            }
            (slot, was_playing)
        };
        if was_playing {
            self.select_timer.stop();
            self.poll_timer.stop();
            if let Err(e) = self.engine.stop() {
                eprintln!("stop playback: {e}");
            }
            app.set_frame(Image::default());
            app.set_video_note(SharedString::new());
        }
        if let Some(store) = self.guide.borrow_mut().as_mut()
            && let Err(e) = store.remove(&slot.provider.id)
        {
            eprintln!("remove guide: {e}");
        }
        if let Some(history) = &self.history
            && let Err(e) = history.remove(&slot.provider.id)
        {
            eprintln!("remove watch history: {e}");
        }
        let cleanup = slot
            .provider
            .forget_password()
            .and_then(|()| self.paths.remove_provider(&slot.provider));
        let error = cleanup
            .err()
            .map(|e| format!("Signed out, but cleanup failed: {e}"));
        self.refresh_lists();
        if self.state.borrow().slots.is_empty() {
            app.set_login_server(slot.provider.server.as_str().into());
            app.set_login_username(SharedString::new());
            self.show_login(error.as_deref().unwrap_or(""));
        } else {
            if let Some(error) = error {
                app.set_status(error.into());
            }
            if was_playing && app.get_screen() == Screen::Player {
                self.back();
            }
        }
    }

    /// Shows one provider's groups (`index`), or every provider's (-1).
    pub(super) fn select_view(&self, index: i32) {
        {
            let mut state = self.state.borrow_mut();
            let view = usize::try_from(index)
                .ok()
                .and_then(|i| state.slots.get(i))
                .map_or(View::All, |slot| View::One(slot.provider.id.clone()));
            state.catalog.set_view(view);
            state.group = 0;
        }
        self.refresh_lists();
    }

    /// Pushes the providers, the view, its status and groups to the window,
    /// then selects the current group again.
    pub(super) fn refresh_lists(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (providers, view_index, view_name, avatar, groups, group) = {
            let mut state = self.state.borrow_mut();
            // A single provider needs no tags: view it directly.
            if state.slots.len() == 1 && state.catalog.view() == &View::All {
                let id = state.slots[0].provider.id.clone();
                state.catalog.set_view(View::One(id));
            }
            let providers: Vec<ProviderItem> = state
                .slots
                .iter()
                .map(|slot| ProviderItem {
                    name: slot.provider.name.as_str().into(),
                    user: slot.provider.username.as_str().into(),
                    status: if slot.guide.is_empty() {
                        slot.status.as_str().into()
                    } else {
                        format!("{} · {}", slot.status, slot.guide).into()
                    },
                })
                .collect();
            let viewed = match state.catalog.view() {
                View::All => None,
                View::One(id) => state.slots.iter().position(|s| &s.provider.id == id),
            };
            let (view_index, view_name, avatar) = match viewed {
                Some(i) => (
                    i as i32,
                    state.slots[i].provider.name.clone(),
                    initials(&state.slots[i].provider.username),
                ),
                None => (-1, "All providers".to_owned(), "ALL".to_owned()),
            };
            (
                providers,
                view_index,
                view_name,
                avatar,
                state.catalog.group_items(),
                state.group,
            )
        };
        app.set_providers(ModelRc::new(VecModel::from(providers)));
        app.set_view_index(view_index);
        app.set_view_name(view_name.into());
        app.set_avatar(avatar.into());
        app.set_groups(ModelRc::new(VecModel::from(groups)));
        app.set_status(self.view_status());
        self.select_group(group);
        match app.get_screen() {
            Screen::Movies | Screen::Series => self.refresh_vod(),
            Screen::Settings => self.push_settings(),
            _ => {}
        }
    }
}

// Private API
impl Session {
    /// Adds `provider` (replacing an earlier slot with the same id), shows
    /// its cached channels at once, and refreshes them in the background.
    fn open(&self, provider: Provider) {
        let cache = self.paths.cache(&provider);
        {
            let mut state = self.state.borrow_mut();
            state.next_epoch += 1;
            let epoch = state.next_epoch;
            let status = match Library::from_cache(&cache) {
                Some(library) => {
                    let status = format!("{} · updating…", library_status(&library));
                    state.catalog.upsert(Source {
                        id: provider.id.clone(),
                        name: provider.name.clone(),
                        library,
                    });
                    status
                }
                None => "Loading channels…".to_owned(),
            };
            state.slots.retain(|slot| slot.provider.id != provider.id);
            state.slots.push(Slot {
                provider: provider.clone(),
                credentials: None,
                status,
                guide: String::new(),
                movies: Default::default(),
                shows: Default::default(),
                refreshing: false,
                epoch,
            });
        }
        self.refresh_provider(&provider.id, Force::No, Force::No);
    }

    fn logged_in(&self, result: provider::Result<Provider>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.set_login_busy(false);
        match result {
            Ok(provider) => {
                app.set_login_password(SharedString::new());
                let id = provider.id.clone();
                self.open(provider);
                {
                    let mut state = self.state.borrow_mut();
                    state.catalog.set_view(View::One(id));
                    state.group = 0;
                }
                self.refresh_lists();
                self.show(Screen::Live);
            }
            Err(e) => app.set_login_error(e.to_string().into()),
        }
    }

    pub(super) fn credentials_ready(&self, id: &str, epoch: u64, credentials: Credentials) {
        {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.provider.id == id && s.epoch == epoch)
            else {
                return;
            };
            slot.credentials = Some(credentials);
        }
        // Screens without a preview keep it off until Live TV shows again.
        let previewing = self
            .app
            .upgrade()
            .is_some_and(|app| matches!(app.get_screen(), Screen::Live | Screen::Guide));
        if previewing && self.state.borrow().playing.is_none() {
            self.play_selected();
        }
    }

    pub(super) fn library_ready(&self, id: &str, epoch: u64, library: Library) {
        {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.provider.id == id && s.epoch == epoch)
            else {
                return;
            };
            slot.status = library_status(&library);
            let source = Source {
                id: id.to_owned(),
                name: slot.provider.name.clone(),
                library,
            };
            state.catalog.upsert(source);
        }
        self.refresh_lists();
    }

    pub(super) fn refresh_failed(&self, id: &str, epoch: u64, error: &provider::Error) {
        {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.provider.id == id && s.epoch == epoch)
            else {
                return;
            };
            slot.status = format!("Could not update: {error}");
        }
        self.refresh_lists();
    }

    /// The name a provider gets unless the user names it: its host,
    /// widened to the whole address and then the username while another
    /// provider has the same name.
    pub(super) fn automatic_name(&self, server: &str, username: &str) -> String {
        let state = self.state.borrow();
        let others: Vec<&Provider> = state
            .slots
            .iter()
            .map(|slot| &slot.provider)
            .filter(|p| p.server != server || p.username != username)
            .collect();
        let host = host_of(server);
        if !others.iter().any(|p| host_of(&p.server) == host) {
            return host.to_owned();
        }
        let address = server.split_once("://").map_or(server, |(_, rest)| rest);
        if others.iter().any(|p| p.server == server) {
            format!("{address} · {username}")
        } else {
            address.to_owned()
        }
    }
}
