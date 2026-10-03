//! Turns a provider [`Library`] into the rows the Live TV screen shows.

use itele::provider::Library;
use itele::xtream::{Account, LiveStream};
use slint::{Color, Image, SharedString};

use crate::ui::{ChannelItem, GroupItem};

/// Name of the group that lists every channel.
const ALL_CHANNELS: &str = "All channels";
/// Longest short name drawn on a fallback logo tile.
const SHORT_NAME_MAX: usize = 8;
/// Fallback logo tile colours, from the approved mockups.
const TINTS: [u32; 9] = [
    0x1f5e3b, 0x2a3cc7, 0xb3261e, 0x0f6e8c, 0x3b3f4a, 0xc2185b, 0x5b6b2e, 0xd35400, 0x6b5ca5,
];

/// The provider's channels, grouped the way the screen lists them.
pub struct Catalog {
    library: Library,
    groups: Vec<Group>,
}

struct Group {
    name: String,
    channels: Vec<usize>,
}

// Public API
impl Catalog {
    /// Groups `library`: every channel first, then each category in
    /// provider order. Categories without channels are left out.
    pub fn new(library: Library) -> Self {
        let mut groups = vec![Group {
            name: ALL_CHANNELS.to_owned(),
            channels: (0..library.streams.len()).collect(),
        }];
        for category in &library.categories {
            let channels: Vec<usize> = library
                .streams
                .iter()
                .enumerate()
                .filter(|(_, s)| s.category_id.as_ref() == Some(&category.id))
                .map(|(i, _)| i)
                .collect();
            if !channels.is_empty() {
                groups.push(Group {
                    name: category.name.clone(),
                    channels,
                });
            }
        }
        Self { library, groups }
    }

    /// The account the catalog was fetched with.
    pub fn account(&self) -> &Account {
        &self.library.account
    }

    /// Rows for the group list.
    pub fn group_items(&self) -> Vec<GroupItem> {
        self.groups
            .iter()
            .map(|g| GroupItem {
                name: g.name.as_str().into(),
                count: thousands(g.channels.len()).into(),
            })
            .collect()
    }

    /// The display name of `group`, or empty when out of range.
    pub fn group_name(&self, group: usize) -> &str {
        self.groups.get(group).map_or("", |g| g.name.as_str())
    }

    /// Rows for the channels in `group`.
    pub fn channel_items(&self, group: usize) -> Vec<ChannelItem> {
        let Some(group) = self.groups.get(group) else {
            return Vec::new();
        };
        group
            .channels
            .iter()
            .enumerate()
            .map(|(row, &i)| self.channel_item(&self.library.streams[i], row))
            .collect()
    }

    /// The channel at `row` of `group`.
    pub fn stream(&self, group: usize, row: usize) -> Option<&LiveStream> {
        let index = *self.groups.get(group)?.channels.get(row)?;
        self.library.streams.get(index)
    }

    /// The row of `stream` in `group`, if the group lists it.
    pub fn row_of(&self, group: usize, stream: &LiveStream) -> Option<usize> {
        self.groups
            .get(group)?
            .channels
            .iter()
            .position(|&i| self.library.streams[i].id == stream.id)
    }

    /// Number of channels in `group`.
    pub fn group_len(&self, group: usize) -> usize {
        self.groups.get(group).map_or(0, |g| g.channels.len())
    }
}

// Private API
impl Catalog {
    fn channel_item(&self, stream: &LiveStream, row: usize) -> ChannelItem {
        let group = stream
            .category_id
            .as_ref()
            .and_then(|id| self.library.categories.iter().find(|c| &c.id == id))
            .map_or("", |c| c.name.as_str());
        ChannelItem {
            number: stream.number.unwrap_or(row as u64 + 1).to_string().into(),
            name: stream.name.as_str().into(),
            group: group.into(),
            short: short_name(&stream.name).into(),
            tint: tint(&stream.name),
            logo: Image::default(),
            has_logo: false,
            archive: match stream.archive_days {
                0 => SharedString::new(),
                1 => "Catch-up 1 day".into(),
                days => format!("Catch-up {days} days").into(),
            },
        }
    }
}

/// The first word of `name`, upper-cased and cut to fit a logo tile.
fn short_name(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(SHORT_NAME_MAX)
        .flat_map(char::to_uppercase)
        .collect()
}

/// A stable tile colour for `name`.
fn tint(name: &str) -> Color {
    // FNV-1a: stable across runs, unlike the std hasher.
    let hash = name.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let rgb = TINTS[(hash % TINTS.len() as u64) as usize];
    Color::from_rgb_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `1234567` as `1,234,567`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use itele::xtream;

    fn library() -> Library {
        Library {
            account: xtream::parse_account(r#"{"user_info":{"auth":1,"status":"Active"}}"#)
                .unwrap(),
            categories: xtream::parse_categories(
                r#"[{"category_id":"1","category_name":"News"},
                    {"category_id":"2","category_name":"Empty"},
                    {"category_id":"3","category_name":"Sports"}]"#,
            )
            .unwrap(),
            streams: xtream::parse_live_streams(
                r#"[{"num":10,"name":"Meridian News","stream_id":1,"category_id":"1"},
                    {"num":11,"name":"Volt Sports 1","stream_id":2,"category_id":"3",
                     "tv_archive":1,"tv_archive_duration":7},
                    {"name":"Loose Channel","stream_id":3}]"#,
            )
            .unwrap(),
        }
    }

    #[test]
    fn groups_list_all_then_non_empty_categories() {
        let catalog = Catalog::new(library());
        let names: Vec<_> = catalog
            .group_items()
            .iter()
            .map(|g| g.name.to_string())
            .collect();
        assert_eq!(names, ["All channels", "News", "Sports"]);
        assert_eq!(catalog.group_len(0), 3);
        assert_eq!(catalog.stream(2, 0).unwrap().name, "Volt Sports 1");
        assert!(catalog.stream(2, 1).is_none());
    }

    #[test]
    fn channel_rows_carry_number_group_and_archive() {
        let catalog = Catalog::new(library());
        let rows = catalog.channel_items(0);
        assert_eq!(rows[0].number, "10");
        assert_eq!(rows[1].group, "Sports");
        assert_eq!(rows[1].archive, "Catch-up 7 days");
        assert_eq!(
            rows[2].number, "3",
            "no provider number: falls back to the row"
        );
        assert_eq!(rows[2].group, "");
        let volt = catalog.stream(0, 1).unwrap();
        assert_eq!(catalog.row_of(2, volt), Some(0));
        assert_eq!(catalog.row_of(1, volt), None);
    }

    #[test]
    fn short_names_fit_a_tile() {
        assert_eq!(short_name("Volt Sports 1"), "VOLT");
        assert_eq!(short_name("Ciné+ Club"), "CINÉ");
        assert_eq!(short_name("Supercalifragilistic"), "SUPERCAL");
        assert_eq!(short_name(""), "");
    }

    #[test]
    fn tint_is_stable() {
        assert_eq!(tint("Atlas Nature HD"), tint("Atlas Nature HD"));
    }

    #[test]
    fn thousands_groups_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1284), "1,284");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
