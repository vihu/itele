//! The Favorites group: the user's channels across providers, in their
//! order, first in every view. A favorite whose provider stops listing the
//! channel stays, dimmed, until it comes back or the user removes it.

use itele::favorites::{Favorite, Kind};
use itele::xtream::{LiveStream, StreamId};
use slint::SharedString;

use super::{Catalog, Entry, Group, Row, Source};
use crate::names::{short_name, tint};
use crate::ui::ChannelItem;

/// Name of the group of favorites.
const FAVORITES: &str = "Favorites";

// Public API
impl Catalog {
    /// Shows `favorites` (channels, in the user's order) as the first group.
    pub fn set_favorites(&mut self, favorites: Vec<Favorite>) {
        self.favorite_keys = favorites
            .iter()
            .map(|f| (f.provider.clone(), f.id.clone()))
            .collect();
        self.favorites = favorites;
        self.regroup();
    }

    /// Limits the Favorites group to `provider`'s; `None` shows every one.
    pub fn set_favorites_filter(&mut self, provider: Option<String>) {
        self.favorites_filter = provider;
        self.regroup();
    }

    /// The provider the Favorites group is limited to.
    pub fn favorites_filter(&self) -> Option<&str> {
        self.favorites_filter.as_deref()
    }

    /// The Favorites group, when there are favorites.
    pub fn favorites_group(&self) -> Option<usize> {
        self.groups
            .first()
            .is_some_and(|g| g.owner.is_none())
            .then_some(0)
    }

    /// Whether `provider`'s `stream` is a favorite.
    pub fn is_favorite(&self, provider: &str, stream: StreamId) -> bool {
        self.favorite_keys
            .contains(&(provider.to_owned(), stream.0.to_string()))
    }

    /// The favorite for the channel at `row` of `group`: its provider, id,
    /// name and logo now.
    pub fn favorite_at(&self, group: usize, row: usize) -> Option<Favorite> {
        let r = self.groups.get(group)?.rows.get(row)?;
        Some(match self.entry(*r) {
            Entry::Channel(source, stream) => favorite(source, stream),
            Entry::Gone(favorite) => favorite.clone(),
        })
    }

    /// The row of `favorite` in `group`, whether its channel is listed or
    /// gone.
    pub fn row_of_favorite(&self, group: usize, favorite: &Favorite) -> Option<usize> {
        self.rows(group).position(|entry| match entry {
            Entry::Channel(source, stream) => {
                source.id == favorite.provider && stream.id.0.to_string() == favorite.id
            }
            Entry::Gone(f) => f.provider == favorite.provider && f.id == favorite.id,
        })
    }
}

// Private API
impl Catalog {
    /// The Favorites group, limited by the filter; none without favorites.
    pub(super) fn favorites_group_rows(&self) -> Option<Group> {
        if self.favorites.is_empty() {
            return None;
        }
        let rows = self
            .favorites
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                self.favorites_filter
                    .as_ref()
                    .is_none_or(|p| &f.provider == p)
            })
            .map(|(index, f)| self.find(f).unwrap_or(Row::Gone(index)))
            .collect();
        Some(Group {
            name: FAVORITES.to_owned(),
            owner: None,
            rows,
        })
    }

    /// A favorite whose provider does not list its channel: its name and
    /// logo as they were, dimmed.
    pub(super) fn gone_item(&self, favorite: &Favorite, row: usize) -> ChannelItem {
        let source = self.source(&favorite.provider);
        let provider = source.map_or(favorite.provider.as_str(), |s| s.name.as_str());
        let note = match source {
            Some(_) => format!("No longer in {provider}\u{2019}s list"),
            None => format!("Waiting for {provider}\u{2026}"),
        };
        ChannelItem {
            number: (row + 1).to_string().into(),
            name: favorite.title.as_str().into(),
            group: note.into(),
            provider: provider.into(),
            hue: self.hue_of(&favorite.provider),
            short: short_name(&favorite.title).into(),
            tint: tint(&favorite.title),
            favorite: true,
            gone: true,
            now_progress: -1.0,
            now_title: SharedString::new(),
            ..ChannelItem::default()
        }
    }

    /// The row of `favorite`'s channel, when its provider lists it.
    fn find(&self, favorite: &Favorite) -> Option<Row> {
        let source = self
            .sources
            .iter()
            .position(|s| s.id == favorite.provider)?;
        let stream = self.sources[source]
            .library
            .streams
            .iter()
            .position(|s| s.id.0.to_string() == favorite.id)?;
        Some(Row::Channel { source, stream })
    }
}

fn favorite(source: &Source, stream: &LiveStream) -> Favorite {
    Favorite {
        provider: source.id.clone(),
        kind: Kind::Channel,
        id: stream.id.0.to_string(),
        title: stream.name.clone(),
        poster: stream.icon.clone().unwrap_or_default(),
        year: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::View;
    use crate::live::tests::{NORTH, SOUTH, source};

    fn kept(provider: &str, id: &str, title: &str) -> Favorite {
        Favorite {
            provider: provider.into(),
            kind: Kind::Channel,
            id: id.into(),
            title: title.into(),
            poster: String::new(),
            year: None,
        }
    }

    #[test]
    fn favorites_come_first_across_providers_in_order() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source("south", SOUTH));
        catalog.set_view(View::One("north".into()));
        assert_eq!(catalog.favorites_group(), None);
        assert_eq!(catalog.first_group(), 0);

        catalog.set_favorites(vec![
            kept("south", "1", "Rivage 1"),
            kept("north", "2", "Volt Sports 1"),
            kept("north", "99", "Tempo Music"),
        ]);
        assert_eq!(catalog.favorites_group(), Some(0));
        assert_eq!(catalog.first_group(), 1);
        let groups = catalog.group_items();
        assert!(groups[0].favorites && !groups[1].favorites);

        let rows = catalog.channel_items(0);
        let shown: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.number.as_str(),
                    r.name.as_str(),
                    r.provider.as_str(),
                    r.gone,
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("1", "Rivage 1", "SOUTH", false),
                ("2", "Volt Sports 1", "NORTH", false),
                ("3", "Tempo Music", "NORTH", true),
            ],
            "tagged even when one provider is viewed"
        );
        assert_eq!(catalog.stream(0, 0).unwrap().0.id, "south");
        assert!(catalog.stream(0, 2).is_none(), "a gone channel cannot play");
        assert_eq!(catalog.row_guides(0)[2], super::super::GuideKey::default());
        assert_eq!(catalog.favorite_at(0, 2).unwrap().title, "Tempo Music");

        let all = catalog.channel_items(1);
        assert!(all[1].favorite && !all[0].favorite, "hearts on Volt only");
        assert_eq!(catalog.row_of(0, "north", StreamId(2)), Some(1));
    }

    #[test]
    fn the_filter_keeps_one_providers_favorites() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source("south", SOUTH));
        catalog.set_favorites(vec![
            kept("south", "1", "Rivage 1"),
            kept("north", "1", "News"),
        ]);
        catalog.set_favorites_filter(Some("north".into()));
        let rows = catalog.channel_items(0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Meridian News");
        catalog.set_favorites(Vec::new());
        assert_eq!(catalog.favorites_group(), None);
    }
}
