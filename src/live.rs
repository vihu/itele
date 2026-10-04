//! Turns the providers' libraries into the rows the Live TV screen shows.
//!
//! Every signed-in provider is a [`Source`]. The [`View`] picks one
//! provider, whose groups are "All channels" and its categories, or all of
//! them, where every provider's groups are listed and tagged with its name.
//! A group is a list of rows, each a channel of its own provider.

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
    /// The provider the group belongs to.
    owner: usize,
    rows: Vec<Row>,
}

/// A channel in a group: indexes into the sources and that source's
/// streams.
#[derive(Clone, Copy)]
struct Row {
    source: usize,
    stream: usize,
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
                provider: if tagged {
                    self.sources[g.owner].name.as_str().into()
                } else {
                    SharedString::new()
                },
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
    /// every provider is listed.
    pub fn channel_items(&self, group: usize) -> Vec<ChannelItem> {
        let tagged = self.view == View::All;
        self.rows(group)
            .enumerate()
            .map(|(row, (source, stream))| {
                let tag = tagged.then(|| (source.name.as_str(), self.hue(source)));
                channel_item(source, stream, row, tag)
            })
            .collect()
    }

    /// The provider and channel at `row` of `group`.
    pub fn stream(&self, group: usize, row: usize) -> Option<(&Source, &LiveStream)> {
        let r = self.groups.get(group)?.rows.get(row)?;
        Some(self.at(*r))
    }

    /// Where each row of `group` finds its programmes.
    pub fn row_guides(&self, group: usize) -> Vec<GuideKey> {
        self.rows(group)
            .map(|(source, stream)| GuideKey {
                provider: source.id.clone(),
                channel: stream
                    .epg_channel_id
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase(),
            })
            .collect()
    }

    /// Days of catch-up each row in `group` keeps; 0 for none.
    pub fn row_archive(&self, group: usize) -> Vec<u32> {
        self.rows(group).map(|(_, s)| s.archive_days).collect()
    }

    /// Each row's logo URL in `group`, empty when the provider has none.
    pub fn row_logos(&self, group: usize) -> Vec<String> {
        self.rows(group)
            .map(|(_, s)| s.icon.clone().unwrap_or_default())
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
        self.rows(group)
            .position(|(source, s)| source.id == provider && s.id == stream)
    }
}

// Private API
impl Catalog {
    /// The provider and channel of each row of `group`.
    fn rows(&self, group: usize) -> impl Iterator<Item = (&Source, &LiveStream)> {
        self.groups
            .get(group)
            .into_iter()
            .flat_map(|g| g.rows.iter().map(|r| self.at(*r)))
    }

    fn at(&self, row: Row) -> (&Source, &LiveStream) {
        let source = &self.sources[row.source];
        (source, &source.library.streams[row.stream])
    }

    fn hue(&self, source: &Source) -> Color {
        self.hue_of(&source.id)
    }

    fn regroup(&mut self) {
        let mut groups = Vec::new();
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
    let row = |stream| Row {
        source: index,
        stream,
    };
    let mut groups = vec![Group {
        name: ALL_CHANNELS.to_owned(),
        owner: index,
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
                owner: index,
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
mod tests {
    use super::*;
    use itele::xtream;

    fn source(id: &str, streams: &str) -> Source {
        Source {
            id: id.into(),
            name: id.to_uppercase(),
            library: Library {
                account: xtream::parse_account(r#"{"user_info":{"auth":1,"status":"Active"}}"#)
                    .unwrap(),
                categories: xtream::parse_categories(
                    r#"[{"category_id":"1","category_name":"News"},
                        {"category_id":"2","category_name":"Empty"},
                        {"category_id":"3","category_name":"Sports"}]"#,
                )
                .unwrap(),
                streams: xtream::parse_live_streams(streams).unwrap(),
            },
        }
    }

    const NORTH: &str = r#"[{"num":10,"name":"Meridian News","stream_id":1,"category_id":"1"},
        {"num":11,"name":"Volt Sports 1","stream_id":2,"category_id":"3",
         "tv_archive":1,"tv_archive_duration":7},
        {"name":"Loose Channel","stream_id":3}]"#;
    const SOUTH: &str = r#"[{"num":1,"name":"Rivage 1","stream_id":1,"category_id":"1"}]"#;

    fn names(catalog: &Catalog) -> Vec<String> {
        catalog
            .group_items()
            .iter()
            .map(|g| g.name.to_string())
            .collect()
    }

    #[test]
    fn one_provider_lists_all_then_non_empty_categories() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source("south", SOUTH));
        catalog.set_view(View::One("north".into()));
        assert_eq!(names(&catalog), ["All channels", "News", "Sports"]);
        assert!(catalog.group_items().iter().all(|g| g.provider.is_empty()));
        assert_eq!(catalog.group_len(0), 3);
        let (source, volt) = catalog.stream(2, 0).unwrap();
        assert_eq!(
            (source.id.as_str(), volt.name.as_str()),
            ("north", "Volt Sports 1")
        );
        assert!(catalog.stream(2, 1).is_none());
    }

    #[test]
    fn all_view_tags_every_providers_groups() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source("south", SOUTH));
        assert_eq!(catalog.view(), &View::All);
        assert_eq!(
            names(&catalog),
            ["All channels", "News", "Sports", "All channels", "News"]
        );
        let items = catalog.group_items();
        assert_eq!(
            (items[0].provider.as_str(), items[3].provider.as_str()),
            ("NORTH", "SOUTH")
        );
        assert_eq!(catalog.total_channels(), 4);
        let rows = catalog.channel_items(4);
        assert_eq!(
            (rows[0].group.as_str(), rows[0].provider.as_str()),
            ("News", "SOUTH")
        );
        assert_ne!(rows[0].hue, catalog.channel_items(0)[0].hue, "a hue each");
        let guides = catalog.row_guides(4);
        assert_eq!(guides[0].provider, "south");
    }

    #[test]
    fn channel_rows_carry_number_group_and_archive() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.set_view(View::One("north".into()));
        let rows = catalog.channel_items(0);
        assert_eq!(rows[0].number, "10");
        assert_eq!(rows[0].provider, "", "one provider: no tags");
        assert_eq!(rows[1].group, "Sports");
        assert_eq!(rows[1].archive, "Catch-up 7 days");
        assert_eq!(
            rows[2].number, "3",
            "no provider number: falls back to the row"
        );
        assert_eq!(rows[2].group, "");
        assert_eq!(catalog.row_of(2, "north", StreamId(2)), Some(0));
        assert_eq!(catalog.row_of(1, "north", StreamId(2)), None);
        assert_eq!(
            catalog.row_of(2, "south", StreamId(2)),
            None,
            "same id, other provider"
        );
    }

    #[test]
    fn removing_the_viewed_provider_shows_all() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source("south", SOUTH));
        catalog.set_view(View::One("south".into()));
        catalog.remove("south");
        assert_eq!(catalog.view(), &View::All);
        assert_eq!(names(&catalog), ["All channels", "News", "Sports"]);
    }

    #[test]
    fn a_provider_still_loading_can_be_viewed() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.set_view(View::One("south".into()));
        assert_eq!(catalog.group_count(), 0);
        catalog.upsert(source("south", SOUTH));
        assert_eq!(catalog.view(), &View::One("south".into()));
        assert_eq!(names(&catalog), ["All channels", "News"]);
    }

    #[test]
    fn upsert_replaces_in_place() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source("south", SOUTH));
        catalog.upsert(source("north", SOUTH));
        let ids: Vec<_> = catalog.sources().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["north", "south"]);
        assert_eq!(catalog.total_channels(), 2);
    }

    #[test]
    fn search_ranks_prefixes_then_word_starts() {
        let mut catalog = Catalog::default();
        catalog.upsert(source(
            "north",
            r#"[{"name":"Sports Extra","stream_id":1},{"name":"Volt Sports 1","stream_id":2},
                {"name":"Esports Arena","stream_id":3},{"name":"Meridian News","stream_id":4}]"#,
        ));
        catalog.upsert(source("south", r#"[{"name":"SPORTS","stream_id":1}]"#));
        let names: Vec<_> = catalog
            .search_channels("sports", 10)
            .iter()
            .map(|h| (h.source.id.as_str(), h.stream.name.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("south", "SPORTS"),
                ("north", "Sports Extra"),
                ("north", "Volt Sports 1"),
                ("north", "Esports Arena")
            ]
        );
        assert_eq!(
            catalog.search_channels("volt 1", 10).len(),
            1,
            "every word must match"
        );
        assert!(catalog.search_channels("  ", 10).is_empty());
        assert_eq!(catalog.search_channels("s", 2).len(), 2, "limit");
    }

    #[test]
    fn by_guide_id_ignores_case() {
        let mut catalog = Catalog::default();
        catalog.upsert(source(
            "north",
            r#"[{"name":"One","stream_id":7,"epg_channel_id":"One.UK"}]"#,
        ));
        assert_eq!(
            catalog.by_guide_id("north", "one.uk").unwrap().1.id,
            StreamId(7)
        );
        assert!(catalog.by_guide_id("south", "one.uk").is_none());
        assert!(catalog.by_guide_id("north", "two.uk").is_none());
    }
}
