//! Turns the providers' libraries into the rows the Live TV screen shows.
//!
//! Every signed-in provider is a [`Source`]. The [`View`] picks one
//! provider, whose groups are "All channels" and its categories, or all of
//! them, where every provider's groups are listed and tagged with its name.
//! A group is a list of rows, each a channel of its own provider. The
//! Favorites group comes first in every view, across providers.

mod favorites;

use std::collections::HashSet;

use itele::favorites::Favorite;
use itele::provider::Library;
use itele::xtream::{LiveStream, StreamId};
use slint::{Color, Image, SharedString};

use crate::names::{provider_hue, search_rank, short_name, thousands, tint};
use crate::ui::{ChannelItem, GroupItem};

/// Name of the group that lists every channel of a provider.
const ALL_CHANNELS: &str = "All channels";

/// Every provider's channels, grouped for the current [`View`].
#[derive(Default)]
pub struct Catalog {
    sources: Vec<Source>,
    view: View,
    groups: Vec<Group>,
    /// The channel favorites, in the user's order.
    favorites: Vec<Favorite>,
    /// Each favorite's provider and stream id, for the hearts on rows.
    favorite_keys: HashSet<(String, String)>,
    /// The provider the Favorites group shows; `None` shows every one.
    favorites_filter: Option<String>,
}

/// One signed-in provider's channels.
pub struct Source {
    /// The provider's id.
    pub id: String,
    /// The provider's display name.
    pub name: String,
    /// Account, categories, and channels.
    pub library: Library,
}

/// A channel found by name.
pub struct ChannelHit<'a> {
    /// The channel's provider.
    pub source: &'a Source,
    /// The channel.
    pub stream: &'a LiveStream,
    /// Its category name, or empty.
    pub category: &'a str,
}

/// Which providers the group list shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum View {
    /// Every provider's groups, tagged with the provider.
    #[default]
    All,
    /// One provider's groups, by provider id.
    One(String),
}

/// Where a row's programmes are in the guide.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuideKey {
    /// The row's provider.
    pub provider: String,
    /// The channel's XMLTV id, lowercased; empty without one.
    pub channel: String,
}

struct Group {
    name: String,
    /// The provider the group belongs to; `None` for Favorites.
    owner: Option<usize>,
    rows: Vec<Row>,
}

/// A row of a group.
#[derive(Clone, Copy)]
enum Row {
    /// A provider's channel: indexes into the sources and that source's
    /// streams.
    Channel { source: usize, stream: usize },
    /// A favorite whose provider does not list its channel (now): an index
    /// into the favorites.
    Gone(usize),
}

/// What a row shows.
enum Entry<'a> {
    Channel(&'a Source, &'a LiveStream),
    Gone(&'a Favorite),
}

// Public API
impl Catalog {
    /// The providers, in the order they were added.
    #[cfg(test)]
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// The view the groups are built for.
    pub fn view(&self) -> &View {
        &self.view
    }

    /// Shows `view`. A provider whose channels have not arrived yet shows
    /// no groups until they do.
    pub fn set_view(&mut self, view: View) {
        self.view = view;
        self.regroup();
    }

    /// Adds `source`, or replaces the provider with the same id in place.
    pub fn upsert(&mut self, source: Source) {
        match self.sources.iter_mut().find(|s| s.id == source.id) {
            Some(slot) => *slot = source,
            None => self.sources.push(source),
        }
        self.regroup();
    }

    /// The provider with `id`, once its channels have loaded.
    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    /// Renames the provider with `id`.
    pub fn rename(&mut self, id: &str, name: &str) {
        for source in self.sources.iter_mut().filter(|s| s.id == id) {
            name.clone_into(&mut source.name);
        }
    }

    /// Removes the provider with `id`; a view of it falls back to all.
    pub fn remove(&mut self, id: &str) {
        self.sources.retain(|s| s.id != id);
        if self.view == View::One(id.to_owned()) {
            self.view = View::All;
        }
        self.regroup();
    }

    /// Channels across every provider.
    pub fn total_channels(&self) -> usize {
        self.sources.iter().map(|s| s.library.streams.len()).sum()
    }

    /// Rows for the group list.
    pub fn group_items(&self) -> Vec<GroupItem> {
        let tagged = self.view == View::All;
        self.groups
            .iter()
            .map(|g| GroupItem {
                name: g.name.as_str().into(),
                count: thousands(g.rows.len()).into(),
                provider: match g.owner {
                    Some(owner) if tagged => self.sources[owner].name.as_str().into(),
                    _ => SharedString::new(),
                },
                favorites: g.owner.is_none(),
            })
            .collect()
    }

    /// Number of groups in the current view.
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// The display name of `group`, or empty when out of range.
    pub fn group_name(&self, group: usize) -> &str {
        self.groups.get(group).map_or("", |g| g.name.as_str())
    }

    /// Number of channels in `group`.
    pub fn group_len(&self, group: usize) -> usize {
        self.groups.get(group).map_or(0, |g| g.rows.len())
    }

    /// Rows for the channels in `group`; tagged with their provider when
    /// rows from several providers show. Favorites are numbered in order.
    pub fn channel_items(&self, group: usize) -> Vec<ChannelItem> {
        let across = self.groups.get(group).is_some_and(|g| g.owner.is_none());
        let tagged = across || self.view == View::All;
        self.rows(group)
            .enumerate()
            .map(|(row, entry)| match entry {
                Entry::Channel(source, stream) => {
                    let tag = tagged.then(|| (source.name.as_str(), self.hue(source)));
                    let mut item = channel_item(source, stream, row, tag);
                    item.favorite = self.is_favorite(&source.id, stream.id);
                    if across {
                        item.number = (row + 1).to_string().into();
                    }
                    item
                }
                Entry::Gone(favorite) => self.gone_item(favorite, row),
            })
            .collect()
    }

    /// The provider and channel at `row` of `group`; `None` for a favorite
    /// its provider does not list.
    pub fn stream(&self, group: usize, row: usize) -> Option<(&Source, &LiveStream)> {
        let r = self.groups.get(group)?.rows.get(row)?;
        match self.entry(*r) {
            Entry::Channel(source, stream) => Some((source, stream)),
            Entry::Gone(_) => None,
        }
    }

    /// Where each row of `group` finds its programmes.
    pub fn row_guides(&self, group: usize) -> Vec<GuideKey> {
        self.rows(group)
            .map(|entry| match entry {
                Entry::Channel(source, stream) => GuideKey {
                    provider: source.id.clone(),
                    channel: stream
                        .epg_channel_id
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase(),
                },
                Entry::Gone(_) => GuideKey::default(),
            })
            .collect()
    }

    /// Days of catch-up each row in `group` keeps; 0 for none.
    pub fn row_archive(&self, group: usize) -> Vec<u32> {
        self.rows(group)
            .map(|entry| match entry {
                Entry::Channel(_, stream) => stream.archive_days,
                Entry::Gone(_) => 0,
            })
            .collect()
    }

    /// Each row's logo URL in `group`, empty when the provider has none.
    pub fn row_logos(&self, group: usize) -> Vec<String> {
        self.rows(group)
            .map(|entry| match entry {
                Entry::Channel(_, stream) => stream.icon.clone().unwrap_or_default(),
                Entry::Gone(favorite) => favorite.poster.clone(),
            })
            .collect()
    }

    /// The hue of the provider with `id`, for its tags.
    pub fn hue_of(&self, id: &str) -> Color {
        let index = self.sources.iter().position(|s| s.id == id);
        provider_hue(index.unwrap_or(0))
    }

    /// Channels whose name contains every word of `query`, across all
    /// providers: names that start with the query first, then names with a
    /// word that does, then the rest; shorter names first within each.
    pub fn search_channels(&self, query: &str, limit: usize) -> Vec<ChannelHit<'_>> {
        let query = query.trim().to_lowercase();
        let mut hits: Vec<(u8, usize, ChannelHit<'_>)> = Vec::new();
        for source in &self.sources {
            for stream in &source.library.streams {
                let name = stream.name.to_lowercase();
                let Some(rank) = search_rank(&name, &query) else {
                    continue;
                };
                let category = stream
                    .category_id
                    .as_ref()
                    .and_then(|id| source.library.categories.iter().find(|c| &c.id == id))
                    .map_or("", |c| c.name.as_str());
                hits.push((
                    rank,
                    name.len(),
                    ChannelHit {
                        source,
                        stream,
                        category,
                    },
                ));
            }
        }
        hits.sort_by_key(|(rank, len, _)| (*rank, *len));
        hits.into_iter()
            .take(limit)
            .map(|(_, _, hit)| hit)
            .collect()
    }

    /// The first channel of `provider` with XMLTV id `guide_id`.
    pub fn by_guide_id(&self, provider: &str, guide_id: &str) -> Option<(&Source, &LiveStream)> {
        let source = self.sources.iter().find(|s| s.id == provider)?;
        let stream = source.library.streams.iter().find(|s| {
            s.epg_channel_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(guide_id))
        })?;
        Some((source, stream))
    }

    /// The row of a provider's channel in `group`, if the group lists it.
    pub fn row_of(&self, group: usize, provider: &str, stream: StreamId) -> Option<usize> {
        self.rows(group).position(
            |entry| matches!(entry, Entry::Channel(source, s) if source.id == provider && s.id == stream),
        )
    }

    /// The first group of a provider, after Favorites.
    pub fn first_group(&self) -> usize {
        usize::from(self.favorites_group().is_some())
    }
}

// Private API
impl Catalog {
    /// What each row of `group` shows.
    fn rows(&self, group: usize) -> impl Iterator<Item = Entry<'_>> {
        self.groups
            .get(group)
            .into_iter()
            .flat_map(|g| g.rows.iter().map(|r| self.entry(*r)))
    }

    fn entry(&self, row: Row) -> Entry<'_> {
        match row {
            Row::Channel { source, stream } => {
                let source = &self.sources[source];
                Entry::Channel(source, &source.library.streams[stream])
            }
            Row::Gone(favorite) => Entry::Gone(&self.favorites[favorite]),
        }
    }

    fn hue(&self, source: &Source) -> Color {
        self.hue_of(&source.id)
    }

    fn regroup(&mut self) {
        let mut groups: Vec<Group> = self.favorites_group_rows().into_iter().collect();
        for (index, source) in self.sources.iter().enumerate() {
            if self.view == View::One(source.id.clone()) || self.view == View::All {
                groups.extend(source_groups(index, source));
            }
        }
        self.groups = groups;
    }
}

/// "All channels", then each category with channels, in provider order.
fn source_groups(index: usize, source: &Source) -> Vec<Group> {
    let streams = &source.library.streams;
    let row = |stream| Row::Channel {
        source: index,
        stream,
    };
    let mut groups = vec![Group {
        name: ALL_CHANNELS.to_owned(),
        owner: Some(index),
        rows: (0..streams.len()).map(row).collect(),
    }];
    for category in &source.library.categories {
        let rows: Vec<Row> = streams
            .iter()
            .enumerate()
            .filter(|(_, s)| s.category_id.as_ref() == Some(&category.id))
            .map(|(i, _)| row(i))
            .collect();
        if !rows.is_empty() {
            groups.push(Group {
                name: category.name.clone(),
                owner: Some(index),
                rows,
            });
        }
    }
    groups
}

/// A channel's row; `tag` is its provider's name and hue, when shown.
fn channel_item(
    source: &Source,
    stream: &LiveStream,
    row: usize,
    tag: Option<(&str, Color)>,
) -> ChannelItem {
    let category = stream
        .category_id
        .as_ref()
        .and_then(|id| source.library.categories.iter().find(|c| &c.id == id))
        .map_or("", |c| c.name.as_str());
    let (provider, hue) = tag.unwrap_or_default();
    ChannelItem {
        number: stream.number.unwrap_or(row as u64 + 1).to_string().into(),
        name: stream.name.as_str().into(),
        group: category.into(),
        provider: provider.into(),
        hue,
        favorite: false,
        gone: false,
        short: short_name(&stream.name).into(),
        tint: tint(&stream.name),
        logo: Image::default(),
        has_logo: false,
        archive: match stream.archive_days {
            0 => SharedString::new(),
            1 => "Catch-up 1 day".into(),
            days => format!("Catch-up {days} days").into(),
        },
        now_title: SharedString::new(),
        now_progress: -1.0,
        next_time: SharedString::new(),
        next_title: SharedString::new(),
    }
}

#[cfg(test)]
mod tests;
