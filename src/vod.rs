//! Turns the providers' movies or series into the groups and posters the
//! Movies and Series screens show.
//!
//! Same shape as [`crate::live::Catalog`]: each signed-in provider whose
//! shelf has loaded is a [`Source`], and the shared [`View`] picks one
//! provider (its "All" group, then its categories) or all of them, tagged.

use std::collections::HashMap;

use itele::provider::Shelf;
use itele::xtream::{CategoryId, Movie, Show};
use slint::{Image, SharedString};

use crate::live::View;
use crate::names::{search_rank, thousands, tint};
use crate::ui::{GroupItem, PosterItem};

/// What the screens need from a movie or a series.
pub trait Tile {
    /// Name of the group listing every title of a provider.
    const ALL: &'static str;

    /// Display name.
    fn name(&self) -> &str;
    /// Poster URL, if any.
    fn poster(&self) -> Option<&str>;
    /// Release year, if known.
    fn year(&self) -> Option<u16>;
    /// Rating out of 10, if rated.
    fn rating(&self) -> Option<f32>;
    /// Category, if any.
    fn category(&self) -> Option<&CategoryId>;
    /// The provider's id, as the watch history keys it.
    fn key(&self) -> String;
}

/// A [`Catalog`] of either kind, for code that does not care which.
pub trait Shelves {
    /// Shows `view`; a provider not loaded yet shows no groups until it is.
    fn set_view(&mut self, view: View);
    /// Removes the provider with `id`.
    fn remove(&mut self, id: &str);
    /// Renames the provider with `id`.
    fn rename(&mut self, id: &str, name: &str);
    /// Titles across every loaded provider.
    fn total(&self) -> usize;
    /// Titles of the provider with `id`, if it has loaded.
    fn count_of(&self, id: &str) -> Option<usize>;
    /// Rows for the group list.
    fn group_items(&self) -> Vec<GroupItem>;
    /// Number of groups in the current view.
    fn group_count(&self) -> usize;
    /// The display name of `group`, or empty when out of range.
    fn group_name(&self, group: usize) -> &str;
    /// Posters for the titles in `group`, without pictures.
    fn poster_items(&self, group: usize) -> Vec<PosterItem>;
    /// Each title's poster URL in `group`, empty when it has none.
    fn posters(&self, group: usize) -> Vec<String>;
    /// The provider of `group` and each title's id, as the watch history
    /// keys it.
    fn keys(&self, group: usize) -> Option<(&str, Vec<String>)>;
}

/// Every loaded provider's titles of one kind, grouped for the [`View`].
pub struct Catalog<T> {
    sources: Vec<Source<T>>,
    view: View,
    groups: Vec<Group>,
}

/// One provider's titles of one kind.
pub struct Source<T> {
    /// The provider's id.
    pub id: String,
    /// The provider's display name.
    pub name: String,
    /// Categories and titles.
    pub shelf: Shelf<T>,
}

/// A title found by name.
pub struct TitleHit<'a, T> {
    /// The title's provider.
    pub source: &'a Source<T>,
    /// The title.
    pub title: &'a T,
}

struct Group {
    source: usize,
    name: String,
    titles: Vec<usize>,
}

impl<T> Default for Catalog<T> {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            view: View::default(),
            groups: Vec::new(),
        }
    }
}

// Public API
impl<T: Tile> Catalog<T> {
    /// Adds `source`, or replaces the provider with the same id in place.
    pub fn upsert(&mut self, source: Source<T>) {
        match self.sources.iter_mut().find(|s| s.id == source.id) {
            Some(slot) => *slot = source,
            None => self.sources.push(source),
        }
        self.regroup();
    }

    /// Titles whose name contains every word of `query`, across every
    /// loaded provider, best matches first and shorter names first within
    /// each.
    pub fn search(&self, query: &str, limit: usize) -> Vec<TitleHit<'_, T>> {
        let query = query.trim().to_lowercase();
        let mut hits: Vec<(u8, usize, TitleHit<'_, T>)> = Vec::new();
        for source in &self.sources {
            for title in &source.shelf.titles {
                let name = title.name().to_lowercase();
                if let Some(rank) = search_rank(&name, &query) {
                    hits.push((rank, name.len(), TitleHit { source, title }));
                }
            }
        }
        hits.sort_by_key(|(rank, len, _)| (*rank, *len));
        hits.into_iter()
            .take(limit)
            .map(|(_, _, hit)| hit)
            .collect()
    }

    /// `provider`'s title with id `key`, as [`Tile::key`] gives it.
    pub fn find(&self, provider: &str, key: &str) -> Option<&T> {
        let source = self.sources.iter().find(|s| s.id == provider)?;
        source.shelf.titles.iter().find(|t| t.key() == key)
    }

    /// The provider and title at `index` of `group`.
    pub fn title(&self, group: usize, index: usize) -> Option<(&Source<T>, &T)> {
        let g = self.groups.get(group)?;
        let source = &self.sources[g.source];
        Some((source, source.shelf.titles.get(*g.titles.get(index)?)?))
    }
}

impl<T: Tile> Shelves for Catalog<T> {
    fn set_view(&mut self, view: View) {
        if self.view != view {
            self.view = view;
            self.regroup();
        }
    }

    fn remove(&mut self, id: &str) {
        self.sources.retain(|s| s.id != id);
        self.regroup();
    }

    fn rename(&mut self, id: &str, name: &str) {
        for source in self.sources.iter_mut().filter(|s| s.id == id) {
            name.clone_into(&mut source.name);
        }
    }

    fn total(&self) -> usize {
        self.sources.iter().map(|s| s.shelf.titles.len()).sum()
    }

    fn count_of(&self, id: &str) -> Option<usize> {
        self.sources
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.shelf.titles.len())
    }

    fn group_items(&self) -> Vec<GroupItem> {
        let tagged = self.view == View::All;
        self.groups
            .iter()
            .map(|g| GroupItem {
                name: g.name.as_str().into(),
                count: thousands(g.titles.len()).into(),
                provider: if tagged {
                    self.sources[g.source].name.as_str().into()
                } else {
                    SharedString::new()
                },
                favorites: false,
            })
            .collect()
    }

    fn group_count(&self) -> usize {
        self.groups.len()
    }

    fn group_name(&self, group: usize) -> &str {
        self.groups.get(group).map_or("", |g| g.name.as_str())
    }

    fn poster_items(&self, group: usize) -> Vec<PosterItem> {
        let Some(g) = self.groups.get(group) else {
            return Vec::new();
        };
        let source = &self.sources[g.source];
        let provider = if self.view == View::All {
            source.name.as_str()
        } else {
            ""
        };
        g.titles
            .iter()
            .map(|&i| poster_item(&source.shelf.titles[i], provider))
            .collect()
    }

    fn posters(&self, group: usize) -> Vec<String> {
        let Some(g) = self.groups.get(group) else {
            return Vec::new();
        };
        let titles = &self.sources[g.source].shelf.titles;
        g.titles
            .iter()
            .map(|&i| titles[i].poster().unwrap_or("").to_owned())
            .collect()
    }

    fn keys(&self, group: usize) -> Option<(&str, Vec<String>)> {
        let g = self.groups.get(group)?;
        let source = &self.sources[g.source];
        let titles = &source.shelf.titles;
        Some((
            source.id.as_str(),
            g.titles.iter().map(|&i| titles[i].key()).collect(),
        ))
    }
}

// Private API
impl<T: Tile> Catalog<T> {
    fn regroup(&mut self) {
        let mut groups = Vec::new();
        for (index, source) in self.sources.iter().enumerate() {
            if self.view == View::All || self.view == View::One(source.id.clone()) {
                groups.extend(source_groups(index, source));
            }
        }
        self.groups = groups;
    }
}

/// "All", then each category with titles, in provider order.
fn source_groups<T: Tile>(index: usize, source: &Source<T>) -> Vec<Group> {
    let titles = &source.shelf.titles;
    let mut by_category: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, title) in titles.iter().enumerate() {
        if let Some(category) = title.category() {
            by_category.entry(category.0.as_str()).or_default().push(i);
        }
    }
    let mut groups = vec![Group {
        source: index,
        name: T::ALL.to_owned(),
        titles: (0..titles.len()).collect(),
    }];
    for category in &source.shelf.categories {
        if let Some(titles) = by_category.remove(category.id.0.as_str()) {
            groups.push(Group {
                source: index,
                name: category.name.clone(),
                titles,
            });
        }
    }
    groups
}

fn poster_item<T: Tile>(title: &T, provider: &str) -> PosterItem {
    let year = title.year().map(|y| y.to_string()).unwrap_or_default();
    let detail = match (year.as_str(), provider) {
        ("", provider) => provider.to_owned(),
        (year, "") => year.to_owned(),
        (year, provider) => format!("{year} · {provider}"),
    };
    PosterItem {
        title: title.name().into(),
        detail: detail.into(),
        rating: title
            .rating()
            .map(|r| format!("{r:.1}").into())
            .unwrap_or_default(),
        tint: tint(title.name()),
        poster: Image::default(),
        has_poster: false,
        progress: -1.0,
        watched: false,
        provider: SharedString::new(),
        hue: slint::Color::default(),
    }
}

impl Tile for Movie {
    const ALL: &'static str = "All movies";

    fn name(&self) -> &str {
        &self.name
    }

    fn poster(&self) -> Option<&str> {
        self.poster.as_deref()
    }

    fn year(&self) -> Option<u16> {
        self.year
    }

    fn rating(&self) -> Option<f32> {
        self.rating
    }

    fn category(&self) -> Option<&CategoryId> {
        self.category_id.as_ref()
    }

    fn key(&self) -> String {
        self.id.0.to_string()
    }
}

impl Tile for Show {
    const ALL: &'static str = "All series";

    fn name(&self) -> &str {
        &self.name
    }

    fn poster(&self) -> Option<&str> {
        self.poster.as_deref()
    }

    fn year(&self) -> Option<u16> {
        self.year
    }

    fn rating(&self) -> Option<f32> {
        self.rating
    }

    fn category(&self) -> Option<&CategoryId> {
        self.category_id.as_ref()
    }

    fn key(&self) -> String {
        self.id.0.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use itele::xtream;

    fn source(id: &str, movies: &str) -> Source<Movie> {
        Source {
            id: id.into(),
            name: id.to_uppercase(),
            shelf: Shelf {
                categories: xtream::parse_categories(
                    r#"[{"category_id":"1","category_name":"Action"},
                        {"category_id":"2","category_name":"Empty"},
                        {"category_id":"3","category_name":"Comedy"}]"#,
                )
                .unwrap(),
                titles: xtream::parse_movies(movies).unwrap(),
            },
        }
    }

    const NORTH: &str = r#"[{"name":"Red Fury (2025)","stream_id":1,"category_id":"1",
            "rating":"6.9","stream_icon":"http://p/1.jpg"},
        {"name":"Cottontail","stream_id":2,"category_id":"3"},
        {"name":"Loose","stream_id":3,"category_id":"9"}]"#;

    fn names(catalog: &dyn Shelves) -> Vec<String> {
        catalog
            .group_items()
            .iter()
            .map(|g| g.name.to_string())
            .collect()
    }

    #[test]
    fn groups_list_all_then_categories_with_titles() {
        let mut catalog = Catalog::default();
        catalog.set_view(View::One("north".into()));
        assert_eq!(catalog.group_count(), 0, "not loaded yet");
        catalog.upsert(source("north", NORTH));
        assert_eq!(names(&catalog), ["All movies", "Action", "Comedy"]);
        let items = catalog.poster_items(1);
        assert_eq!(items[0].title, "Red Fury (2025)");
        assert_eq!(
            (items[0].detail.as_str(), items[0].rating.as_str()),
            ("2025", "6.9")
        );
        assert_eq!(catalog.posters(0), ["http://p/1.jpg", "", ""]);
        assert_eq!(catalog.title(2, 0).unwrap().1.name, "Cottontail");
        assert!(catalog.title(2, 1).is_none());
        assert!(catalog.poster_items(9).is_empty());
    }

    #[test]
    fn all_view_tags_providers_and_remove_drops_them() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source(
            "south",
            r#"[{"name":"Shade","stream_id":1,"category_id":"1"}]"#,
        ));
        assert_eq!(
            names(&catalog),
            ["All movies", "Action", "Comedy", "All movies", "Action"]
        );
        assert_eq!(catalog.group_items()[3].provider, "SOUTH");
        assert_eq!(catalog.poster_items(1)[0].detail, "2025 · NORTH");
        assert_eq!((catalog.total(), catalog.count_of("south")), (4, Some(1)));
        catalog.remove("north");
        assert_eq!(names(&catalog), ["All movies", "Action"]);
        assert_eq!(catalog.count_of("north"), None);
    }

    #[test]
    fn search_finds_titles_across_providers() {
        let mut catalog = Catalog::default();
        catalog.upsert(source("north", NORTH));
        catalog.upsert(source(
            "south",
            r#"[{"name":"Fury Road","stream_id":1},{"name":"The Fury","stream_id":2}]"#,
        ));
        let found: Vec<_> = catalog
            .search("fury", 10)
            .iter()
            .map(|h| (h.source.id.as_str(), h.title.name.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("south", "Fury Road"),
                ("south", "The Fury"),
                ("north", "Red Fury (2025)")
            ]
        );
        assert_eq!(catalog.search("fury", 1).len(), 1);
        assert!(catalog.search(" ", 10).is_empty());
    }
}
