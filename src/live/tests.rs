//! Tests of the catalog: groups, rows, views and search.

use super::*;
use itele::xtream;

pub(super) fn source(id: &str, streams: &str) -> Source {
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

pub(super) const NORTH: &str = r#"[{"num":10,"name":"Meridian News","stream_id":1,"category_id":"1"},
    {"num":11,"name":"Volt Sports 1","stream_id":2,"category_id":"3",
     "tv_archive":1,"tv_archive_duration":7},
    {"name":"Loose Channel","stream_id":3}]"#;
pub(super) const SOUTH: &str = r#"[{"num":1,"name":"Rivage 1","stream_id":1,"category_id":"1"}]"#;

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
