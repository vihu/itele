//! Favorite channels: `F` or a heart wherever a channel is selected, and
//! the Favorites screen, which lists the Favorites group with a provider
//! filter and the user's order.

use itele::favorites::{Favorite, Kind};
use slint::{ModelRc, VecModel};

use super::timefmt::now;
use super::{Session, State};
use crate::ui::Screen;

impl Session {
    /// Shows the saved favorites; once, at start.
    pub(super) fn load_favorites(&self) {
        self.refresh_favorites();
    }

    /// `F` or a heart: adds the channel at `row` of the listed group to the
    /// favorites, or removes it.
    pub(super) fn toggle_favorite(&self, row: i32) {
        let favorite = {
            let state = self.state.borrow();
            usize::try_from(row)
                .ok()
                .and_then(|row| state.catalog.favorite_at(state.group, row))
        };
        let Some(favorite) = favorite else {
            return;
        };
        if let Some(store) = self.favorites.borrow().as_ref() {
            let result = match store.contains(&favorite.provider, Kind::Channel, &favorite.id) {
                Ok(true) => store.remove(&favorite.provider, Kind::Channel, &favorite.id),
                Ok(false) => store.add(&favorite, now()),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                eprintln!("favorites: {e}");
            }
        }
        self.refresh_favorites();
    }

    /// `Alt+↑` or `Alt+↓` on the Favorites screen: moves the selected
    /// favorite past the next one shown.
    pub(super) fn move_favorite(&self, delta: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (favorite, filter) = {
            let state = self.state.borrow();
            let favorite = usize::try_from(app.get_channel_index())
                .ok()
                .and_then(|row| state.catalog.favorite_at(state.group, row));
            (
                favorite,
                state.catalog.favorites_filter().map(str::to_owned),
            )
        };
        let Some(favorite) = favorite else {
            return;
        };
        if let Some(store) = self.favorites.borrow_mut().as_mut()
            && let Err(e) = move_past_hidden(store, &favorite, filter.as_deref(), delta)
        {
            eprintln!("favorites: {e}");
        }
        self.refresh_favorites();
    }

    /// Opens the Favorites screen on the Favorites group; Live TV gets its
    /// own group back when it shows again.
    pub(super) fn open_favorites(&self) {
        let group = {
            let mut state = self.state.borrow_mut();
            let group = state.catalog.favorites_group();
            if group.is_some_and(|g| g != state.group) {
                state.live_group = Some(state.group);
            }
            group
        };
        if let Some(group) = group {
            self.select_group(group);
        }
        self.fill_favorite_titles();
        self.show(Screen::Favorites);
        self.resume_preview();
        if let Some(app) = self.app.upgrade() {
            app.invoke_reveal_current();
        }
    }

    /// Gives Live TV and the guide back the group they showed before the
    /// Favorites screen.
    pub(super) fn leave_favorites(&self) {
        let group = self.state.borrow_mut().live_group.take();
        if let Some(group) = group {
            self.select_group(group);
        }
    }

    /// The provider filter on the Favorites screen: `index` into the
    /// providers, or -1 for every one.
    pub(super) fn filter_favorites(&self, index: i32) {
        let provider = usize::try_from(index).ok().and_then(|i| {
            self.state
                .borrow()
                .slots
                .get(i)
                .map(|s| s.provider.id.clone())
        });
        if let Some(app) = self.app.upgrade() {
            app.set_favorites_filter(index);
        }
        self.refresh_favorites_with(|state| state.catalog.set_favorites_filter(provider));
        self.fill_favorite_titles();
    }

    /// Forgets `provider`'s favorites, when it signs out; the filter,
    /// which counts providers, shows every one again.
    pub(super) fn forget_favorites(&self, provider: &str) {
        if let Some(store) = self.favorites.borrow().as_ref()
            && let Err(e) = store.remove_provider(provider)
        {
            eprintln!("favorites: {e}");
        }
        self.state.borrow_mut().catalog.set_favorites_filter(None);
        if let Some(app) = self.app.upgrade() {
            app.set_favorites_filter(-1);
        }
        self.fill_favorite_titles();
    }

    /// Reads the favorites again and shows them, keeping the group and the
    /// selected channel (or, when it went, the row it was on).
    pub(super) fn refresh_favorites(&self) {
        self.refresh_favorites_with(|_| {});
    }

    /// Changes the state with `change`, then reads the favorites again as
    /// [`Self::refresh_favorites`] does.
    fn refresh_favorites_with(&self, change: impl FnOnce(&mut State)) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let list = self
            .favorites
            .borrow()
            .as_ref()
            .and_then(|store| store.list(Kind::Channel).ok())
            .unwrap_or_default();
        let selected = usize::try_from(app.get_channel_index()).ok();
        let (group, kept, groups, listed) = {
            let mut state = self.state.borrow_mut();
            let kept = selected.and_then(|row| state.catalog.favorite_at(state.group, row));
            // The Favorites group was listed: its rows change.
            let listed = state.catalog.favorites_group() == Some(state.group);
            change(&mut state);
            let had = state.catalog.favorites_group().is_some();
            state.catalog.set_favorites(list);
            let has = state.catalog.favorites_group().is_some();
            // The Favorites group came or went: the others moved by one.
            let shift = |group: usize| match (had, has) {
                (false, true) => group + 1,
                (true, false) => group.saturating_sub(1),
                _ => group,
            };
            state.live_group = state.live_group.map(shift);
            state.group = shift(state.group);
            (state.group, kept, state.catalog.group_items(), listed)
        };
        self.push_favorite_counts();
        app.set_groups(ModelRc::new(VecModel::from(groups)));
        self.select_group(group);
        let row = {
            let state = self.state.borrow();
            let len = state.catalog.group_len(group);
            kept.and_then(|f| state.catalog.row_of_favorite(group, &f))
                .or_else(|| selected.filter(|_| len > 0).map(|row| row.min(len - 1)))
        };
        if let Some(row) = row {
            app.set_channel_index(row as i32);
            self.refresh_programme();
            app.invoke_reveal_current();
        }
        if listed && app.get_screen() == Screen::Guide {
            let len = self.state.borrow().catalog.group_len(group);
            let row = app.get_guide_row().min(len as i32 - 1);
            app.set_channel_index(row);
            self.open_guide();
        }
    }
}

/// Moves `favorite` one place in the list shown: past hidden favorites of
/// other providers when the list is limited to `filter`'s.
fn move_past_hidden(
    store: &mut itele::favorites::Favorites,
    favorite: &Favorite,
    filter: Option<&str>,
    delta: i32,
) -> rusqlite::Result<()> {
    let list = store.list(Kind::Channel)?;
    let Some(mut at) = list
        .iter()
        .position(|f| f.provider == favorite.provider && f.id == favorite.id)
    else {
        return Ok(());
    };
    while store.move_by(&favorite.provider, Kind::Channel, &favorite.id, delta)? {
        // The favorite it swapped with now sits where it was.
        let passed = if delta < 0 { at - 1 } else { at + 1 };
        let shown = filter.is_none_or(|p| list[passed].provider == p);
        at = passed;
        if shown {
            break;
        }
    }
    Ok(())
}
