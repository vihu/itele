//! The Settings screen: each provider's details, and renaming it.

use itele::provider::Provider;
use itele::xtream::Action;
use slint::{ModelRc, VecModel};

use super::timefmt::{ago, now};
use super::{Session, Slot, State, expiry};
use crate::names::thousands;
use crate::ui::{Fact, ProviderDetails, Screen};
use crate::vod::Shelves;

impl Session {
    pub(super) fn open_settings(&self) {
        self.show(Screen::Settings);
        self.push_settings();
    }

    /// Pushes every provider's details to the Settings screen.
    pub(super) fn push_settings(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let rows: Vec<ProviderDetails> = {
            let state = self.state.borrow();
            state
                .slots
                .iter()
                .map(|slot| self.details_of(&state, slot))
                .collect()
        };
        app.set_settings_providers(ModelRc::new(VecModel::from(rows)));
    }

    /// Renames the provider at `index`; an empty name brings back the
    /// automatic one.
    pub(super) fn rename_provider(&self, index: i32, name: &str) {
        let Some(provider) = usize::try_from(index)
            .ok()
            .and_then(|i| self.state.borrow().slots.get(i).map(|s| s.provider.clone()))
        else {
            return;
        };
        let name = match name.trim() {
            "" => self.automatic_name(&provider.server, &provider.username),
            name => name.to_owned(),
        };
        if name == provider.name {
            return;
        }
        let renamed = Provider {
            name: name.clone(),
            ..provider
        };
        if let Err(e) = self.paths.add_provider(&renamed) {
            eprintln!("rename provider: {e}");
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            let id = renamed.id.clone();
            if let Some(slot) = state.slots.iter_mut().find(|s| s.provider.id == id) {
                slot.provider = renamed;
            }
            state.catalog.rename(&id, &name);
            state.movies.rename(&id, &name);
            state.shows.rename(&id, &name);
        }
        self.refresh_lists();
    }
}

// Private API
impl Session {
    fn details_of(&self, state: &State, slot: &Slot) -> ProviderDetails {
        let provider = &slot.provider;
        let source = state.catalog.source(&provider.id);
        let account = source.map(|s| &s.library.account);
        let failed = [
            &slot.status,
            &slot.movies.status,
            &slot.shows.status,
            &slot.guide,
        ]
        .into_iter()
        .filter(|s| s.contains("ould not") || s.contains("failed"))
        .cloned()
        .collect::<Vec<_>>();
        let state_line = match account {
            Some(account) => {
                let status = if account.status.is_empty() {
                    "Active"
                } else {
                    &account.status
                };
                match (account.is_trial, expiry(account)) {
                    (true, Some(date)) => format!("Trial · until {date}"),
                    (false, Some(date)) => format!("{status} · until {date}"),
                    (_, None) => status.to_owned(),
                }
            }
            None => slot.status.clone(),
        };
        let active =
            account.is_some_and(|a| a.status.is_empty() || a.status.eq_ignore_ascii_case("active"));
        let connections = account
            .and_then(|a| match (a.active_connections, a.max_connections) {
                (Some(on), Some(max)) => Some(format!("{on} of {max} in use")),
                (None, Some(max)) => Some(format!("up to {max}")),
                _ => None,
            })
            .unwrap_or_else(|| "—".to_owned());
        let lists = self
            .paths
            .cache(provider)
            .age(Action::LiveStreams)
            .map_or_else(|| "not yet".to_owned(), |age| ago(age.as_secs() as i64));
        let count = |n: Option<usize>, started: bool| match (n, started) {
            (Some(n), _) => thousands(n),
            (None, true) => "loading…".to_owned(),
            (None, false) => "on first visit".to_owned(),
        };
        let guide = self
            .guide
            .borrow()
            .as_ref()
            .and_then(|store| store.imported_at(&provider.id).ok().flatten())
            .map_or_else(|| "not yet".to_owned(), |at| ago(now() - at));
        let fact = |label: &str, value: String| Fact {
            label: label.into(),
            value: value.into(),
        };
        let address = provider
            .server
            .split_once("://")
            .map_or(provider.server.as_str(), |(_, rest)| rest);
        ProviderDetails {
            name: provider.name.as_str().into(),
            placeholder: self
                .automatic_name(&provider.server, &provider.username)
                .into(),
            state: state_line.into(),
            warn: !active || !failed.is_empty(),
            notice: failed.join("\n").into(),
            facts_top: ModelRc::new(VecModel::from(vec![
                fact("Server", address.to_owned()),
                fact("Username", provider.username.clone()),
                fact("Connections", connections),
                fact("Lists updated", lists),
            ])),
            facts_bottom: ModelRc::new(VecModel::from(vec![
                fact(
                    "Channels",
                    count(source.map(|s| s.library.streams.len()), true),
                ),
                fact(
                    "Movies",
                    count(state.movies.count_of(&provider.id), slot.movies.started()),
                ),
                fact(
                    "Series",
                    count(state.shows.count_of(&provider.id), slot.shows.started()),
                ),
                fact("Guide", guide),
            ])),
        }
    }
}
