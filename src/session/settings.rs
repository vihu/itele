//! The Settings screen: each provider's details and name, and its guide
//! (the preferences are in `preferences.rs`).

use itele::provider::Provider;
use itele::xtream::Action;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::refresh::Force;
use super::timefmt::{ago, now};
use super::{Session, Slot, State, expiry};
use crate::names::thousands;
use crate::ui::{Fact, GuideStatus, ProviderDetails, Screen, SettingsData};
use crate::vod::Shelves;

/// The largest guide time shift, in hours either way.
const MAX_SHIFT: i32 = 12;

impl Session {
    pub(super) fn open_settings(&self) {
        self.show(Screen::Settings);
        self.push_settings();
    }

    /// Pushes the providers, the guide rows and the setting values to the
    /// Settings screen.
    pub(super) fn push_settings(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (providers, guide): (Vec<ProviderDetails>, Vec<GuideStatus>) = {
            let state = self.state.borrow();
            state
                .slots
                .iter()
                .map(|slot| (self.details_of(&state, slot), self.guide_of(slot)))
                .unzip()
        };
        let data = app.global::<SettingsData>();
        data.set_providers(ModelRc::new(VecModel::from(providers)));
        data.set_guide_rows(ModelRc::new(VecModel::from(guide)));
        data.set_values(self.setting_values());
    }

    /// Renames the provider at `index`; an empty name brings back the
    /// automatic one.
    pub(super) fn rename_provider(&self, index: i32, name: &str) {
        let Some(provider) = self.provider_at(index) else {
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
        if !self.save_provider(renamed) {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            let id = self.provider_at_id(&state, index);
            state.catalog.rename(&id, &name);
            state.movies.rename(&id, &name);
            state.shows.rename(&id, &name);
        }
        self.refresh_lists();
    }

    /// Moves the guide of the provider at `index` by `sign` hours and
    /// downloads it again.
    pub(super) fn shift_guide(&self, index: i32, sign: i32) {
        let Some(provider) = self.provider_at(index) else {
            return;
        };
        let shift = (provider.guide_shift + sign.signum()).clamp(-MAX_SHIFT, MAX_SHIFT);
        if shift == provider.guide_shift {
            return;
        }
        let id = provider.id.clone();
        if self.save_provider(Provider {
            guide_shift: shift,
            ..provider
        }) {
            self.refresh_provider(&id, Force::No, Force::Yes);
        }
    }

    /// Refreshes the provider at `index`: its guide, and with `lists` its
    /// channel and title lists too.
    pub(super) fn refresh_provider_at(&self, index: i32, lists: Force) {
        if let Some(provider) = self.provider_at(index) {
            self.refresh_provider(&provider.id, lists, Force::Yes);
        }
    }
}

// Private API
impl Session {
    fn provider_at(&self, index: i32) -> Option<Provider> {
        let state = self.state.borrow();
        let slot = state.slots.get(usize::try_from(index).ok()?)?;
        Some(slot.provider.clone())
    }

    fn provider_at_id(&self, state: &State, index: i32) -> String {
        usize::try_from(index)
            .ok()
            .and_then(|i| state.slots.get(i))
            .map(|s| s.provider.id.clone())
            .unwrap_or_default()
    }

    /// Saves `provider` and puts it in its slot; `false` when saving failed.
    fn save_provider(&self, provider: Provider) -> bool {
        if let Err(e) = self.paths.add_provider(&provider) {
            eprintln!("save provider: {e}");
            return false;
        }
        let mut state = self.state.borrow_mut();
        if let Some(slot) = state
            .slots
            .iter_mut()
            .find(|s| s.provider.id == provider.id)
        {
            slot.provider = provider;
        }
        true
    }

    fn guide_of(&self, slot: &Slot) -> GuideStatus {
        let imported = self
            .guide
            .borrow()
            .as_ref()
            .and_then(|store| store.imported_at(&slot.provider.id).ok().flatten());
        let count = slot
            .guide
            .strip_prefix("guide: ")
            .map(|programmes| format!(" · {programmes}"))
            .unwrap_or_default();
        let status = if slot.refreshing {
            "Updating…".to_owned()
        } else if slot.guide.starts_with("guide failed") {
            slot.guide.replacen("guide failed", "Could not update", 1)
        } else {
            match imported {
                Some(at) => format!("Updated {}{count}", ago(now() - at)),
                None => "Not downloaded yet".to_owned(),
            }
        };
        let shift = match slot.provider.guide_shift {
            0 => "0 h".to_owned(),
            h => format!("{h:+} h"),
        };
        GuideStatus {
            name: slot.provider.name.as_str().into(),
            status: status.into(),
            busy: slot.refreshing,
            shift: shift.into(),
        }
    }

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
            busy: slot.refreshing,
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
