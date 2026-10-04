//! What a detail page holds: the title, the provider's details once known,
//! its pictures and how far it was watched, and what the window shows of it.

use std::collections::HashMap;
use std::rc::Rc;

use itele::history::{Entry, Kind as Watched, Progress};
use itele::xtream::{Credentials, Details, Movie, MovieInfo, Show, ShowInfo};
use slint::{Image, Model, SharedString, VecModel};

use super::timefmt::{minutes_left, runtime};
use super::vod::Kind;
use crate::art::Size;
use crate::names::tint;
use crate::ui::{EpisodeItem, ProgrammeInfo, TitleDetails};

/// The page showing.
pub(super) struct Page {
    /// Tells this page's background results from an earlier page's.
    pub(super) token: u64,
    pub(super) provider: String,
    pub(super) subject: Subject,
    poster: Option<Image>,
    backdrop: Option<Image>,
    /// Shown while details load, or when they fail.
    pub(super) note: String,
}

/// What the page is about, with the provider's details once known.
pub(super) enum Subject {
    Movie {
        movie: Movie,
        info: Option<MovieInfo>,
        /// How far it was watched.
        progress: Option<Progress>,
    },
    Show {
        show: Show,
        info: Option<ShowInfo>,
        /// Index into the seasons of `info`.
        season: usize,
        /// The selected season's episodes, updated as stills arrive.
        episodes: Rc<VecModel<EpisodeItem>>,
        /// Each episode's still URL, empty when it has none.
        stills: Vec<String>,
        /// How far each episode was watched, by id, and the one watched
        /// last.
        watched: HashMap<String, Progress>,
        last: Option<String>,
        /// Whether the episode to resume was selected already; until then
        /// the next fill selects it.
        targeted: bool,
    },
}

/// What playing a title needs: where, what to call it, the banner, where
/// its progress is kept, and where to resume.
pub(super) struct Feature {
    pub(super) url: String,
    pub(super) name: String,
    pub(super) programme: ProgrammeInfo,
    pub(super) entry: Entry,
    pub(super) resume: Option<f64>,
}

impl Page {
    /// A new page about `subject`, without pictures yet.
    pub(super) fn new(token: u64, provider: String, subject: Subject, note: String) -> Self {
        Self {
            token,
            provider,
            subject,
            poster: None,
            backdrop: None,
            note,
        }
    }

    pub(super) fn kind(&self) -> Kind {
        match self.subject {
            Subject::Movie { .. } => Kind::Movies,
            Subject::Show { .. } => Kind::Series,
        }
    }

    /// The details the page shows, from the list and the provider.
    fn shared(&self) -> Option<&Details> {
        match &self.subject {
            Subject::Movie { info, .. } => info.as_ref().map(|i| &i.details),
            Subject::Show { info, .. } => info.as_ref().map(|i| &i.details),
        }
    }

    /// The poster and backdrop not shown yet, to ask for.
    pub(super) fn missing_art(&self) -> Vec<(String, Size)> {
        let (poster, backdrop) = self.art_urls();
        [
            (poster, Size::Poster, self.poster.is_none()),
            (backdrop, Size::Backdrop, self.backdrop.is_none()),
        ]
        .into_iter()
        .filter(|(url, _, missing)| *missing && !url.is_empty())
        .map(|(url, size, _)| (url, size))
        .collect()
    }

    /// Shows `image` wherever the page uses `url` at `size`; `true` when
    /// the page details changed.
    pub(super) fn set_art(&mut self, url: &str, size: Size, image: &Image) -> bool {
        let (poster, backdrop) = self.art_urls();
        match (size, &self.subject) {
            (Size::Poster, _) if poster == url => self.poster = Some(image.clone()),
            (Size::Backdrop, _) if backdrop == url => self.backdrop = Some(image.clone()),
            (
                Size::Still,
                Subject::Show {
                    episodes, stills, ..
                },
            ) => {
                for (row, _) in stills.iter().enumerate().filter(|(_, u)| *u == url) {
                    if let Some(mut item) = episodes.row_data(row) {
                        item.still = image.clone();
                        item.has_still = true;
                        episodes.set_row_data(row, item);
                    }
                }
                return false;
            }
            _ => return false,
        }
        true
    }

    /// The page's poster and backdrop URLs, empty when it has none.
    fn art_urls(&self) -> (String, String) {
        let listed = match &self.subject {
            Subject::Movie { movie, .. } => movie.poster.clone(),
            Subject::Show { show, .. } => show.poster.clone(),
        };
        let details = self.shared();
        let poster = listed
            .or_else(|| details.and_then(|d| d.poster.clone()))
            .unwrap_or_default();
        let backdrop = details.and_then(|d| d.backdrop.clone()).unwrap_or_default();
        (poster, backdrop)
    }

    pub(super) fn details(&self) -> TitleDetails {
        let details = self.shared();
        let resume = match &self.subject {
            Subject::Movie { progress, .. } => {
                progress.as_ref().filter(|p| p.resume_at().is_some())
            }
            Subject::Show { .. } => None,
        };
        let (name, year, rating, extent, back_label) = match &self.subject {
            Subject::Movie { movie, .. } => (
                &movie.name,
                movie.year,
                movie.rating,
                details
                    .and_then(|d| d.duration)
                    .filter(|&secs| secs >= 60)
                    .map(|secs| runtime(i64::from(secs))),
                "Movies",
            ),
            Subject::Show { show, info, .. } => (
                &show.name,
                show.year,
                show.rating,
                info.as_ref().map(|i| match i.seasons.len() {
                    1 => "1 season".to_owned(),
                    n => format!("{n} seasons"),
                }),
                "Series",
            ),
        };
        let meta = [
            year.map(|y| y.to_string()),
            extent,
            details.map(|d| d.genre.clone()).filter(|g| !g.is_empty()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        TitleDetails {
            title: name.as_str().into(),
            meta: meta.into(),
            rating: rating
                .or(details.and_then(|d| d.rating))
                .map(|r| format!("{r:.1}").into())
                .unwrap_or_default(),
            plot: details.map(|d| d.plot.as_str()).unwrap_or("").into(),
            credits: details.map(credits).unwrap_or_default().into(),
            tint: tint(name),
            has_poster: self.poster.is_some(),
            poster: self.poster.clone().unwrap_or_default(),
            has_backdrop: self.backdrop.is_some(),
            backdrop: self.backdrop.clone().unwrap_or_default(),
            note: self.note.as_str().into(),
            back_label: back_label.into(),
            play_label: if resume.is_some() { "Resume" } else { "Play" }.into(),
            left: resume
                .map(|p| minutes_left(p.left() as i64))
                .unwrap_or_default()
                .into(),
            restart: resume.is_some(),
        }
    }
}

/// "Loading details…" while nothing is cached.
pub(super) fn loading_note(loading: bool) -> String {
    if loading {
        "Loading details…".to_owned()
    } else {
        String::new()
    }
}

pub(super) fn movie_feature(
    page: &Page,
    movie: &Movie,
    info: &Option<MovieInfo>,
    progress: &Option<Progress>,
    credentials: &Credentials,
) -> Feature {
    let details = page.details();
    let extension = info
        .as_ref()
        .and_then(|i| i.extension.as_deref())
        .unwrap_or(&movie.extension);
    Feature {
        url: credentials.movie_url(movie.id, extension),
        name: movie.name.clone(),
        programme: ProgrammeInfo {
            title: details.title,
            time: details.meta,
            left: SharedString::new(),
            description: details.plot,
            progress: 0.0,
        },
        entry: Entry {
            provider: page.provider.clone(),
            kind: Watched::Movie,
            id: movie.id.0.to_string(),
            series: None,
            title: movie.name.clone(),
        },
        resume: progress.and_then(|p| p.resume_at()),
    }
}

/// For example `With Ana Dimas, Teo Larsen · Directed by Ida Sund`.
fn credits(details: &Details) -> String {
    let cast = (!details.cast.is_empty()).then(|| format!("With {}", details.cast));
    let director =
        (!details.director.is_empty()).then(|| format!("Directed by {}", details.director));
    [cast, director]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_join_what_is_known() {
        let mut details = Details {
            cast: "Ana Dimas".into(),
            director: "Ida Sund".into(),
            ..Details::default()
        };
        assert_eq!(credits(&details), "With Ana Dimas · Directed by Ida Sund");
        details.cast.clear();
        assert_eq!(credits(&details), "Directed by Ida Sund");
        details.director.clear();
        assert_eq!(credits(&details), "");
    }
}
