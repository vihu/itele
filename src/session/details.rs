//! The detail page of a movie or a series: what the list knows at once,
//! then the provider's details (cached per title, refreshed on every open),
//! and playing it. The series' side, seasons and episodes, is in
//! `series.rs`.

use std::rc::Rc;
use std::thread;

use itele::provider::Provider;
use itele::xtream::{self, Action, Client, Credentials, Details, Movie, MovieInfo, Show, ShowInfo};
use slint::{Image, Model, SharedString, VecModel};

use super::timefmt::runtime;
use super::vod::Kind;
use super::{Content, Playing, Session, Title, on_ui_thread};
use crate::art::Size;
use crate::names::tint;
use crate::ui::{EpisodeItem, ProgrammeInfo, Screen, TitleDetails};

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
    },
}

/// What playing a title needs: where, what to call it, and the banner.
pub(super) struct Feature {
    pub(super) url: String,
    pub(super) name: String,
    pub(super) programme: ProgrammeInfo,
}

impl Session {
    /// Opens the page of `provider`'s `movie`.
    pub(super) fn open_movie(&self, provider: String, movie: Movie) {
        let Some((token, credentials, account)) = self.begin_page(&provider) else {
            return;
        };
        let cache = self.paths.cache(&account);
        let action = Action::MovieInfo(movie.id);
        let info = cache.load(action, xtream::parse_movie_info);
        let loading = info.is_none();
        self.show_page(Page {
            token,
            provider,
            subject: Subject::Movie { movie, info },
            poster: None,
            backdrop: None,
            note: loading_note(loading),
        });
        thread::spawn(move || {
            let result = credentials
                .map_or_else(|| account.credentials(), Ok)
                .and_then(|c| cache.refresh(&Client::new(c), action, xtream::parse_movie_info))
                .map_err(|e| e.to_string());
            on_ui_thread(move |s| s.movie_info_ready(token, result));
        });
    }

    /// Back from the page to the grid it was opened from.
    pub(super) fn details_back(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let Some(kind) = self.state.borrow_mut().page.take().map(|p| p.kind()) else {
            return;
        };
        self.show(kind.screen());
        app.invoke_reveal_vod();
    }

    /// Plays the page's movie, or the selected episode of its series.
    pub(super) fn details_play(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let episode = usize::try_from(app.get_details_episode_index()).ok();
        let (provider, provider_name, feature) = {
            let state = self.state.borrow();
            let Some(page) = &state.page else {
                return;
            };
            let Some(slot) = state.slots.iter().find(|s| s.provider.id == page.provider) else {
                return;
            };
            let Some(credentials) = &slot.credentials else {
                drop(state);
                return self.set_page_note("Connecting to the provider…");
            };
            let feature = match &page.subject {
                Subject::Movie { movie, info } => movie_feature(page, movie, info, credentials),
                Subject::Show { .. } => {
                    match episode.and_then(|i| page.episode_feature(i, credentials)) {
                        Some(feature) => feature,
                        None => return,
                    }
                }
            };
            (page.provider.clone(), slot.provider.name.clone(), feature)
        };
        self.select_timer.stop();
        if self.engine.load(&feature.url).is_err() {
            return self.set_page_note("Could not start playback");
        }
        app.set_video_note("Loading…".into());
        app.set_provider_name(provider_name.into());
        app.set_programme(feature.programme);
        self.state.borrow_mut().playing = Some(Playing {
            provider,
            content: Content::Title(Title { name: feature.name }),
        });
        self.enter_player();
    }

    /// A picture for the page arrived.
    pub(super) fn page_art_ready(&self, url: &str, size: Size, image: &Image) {
        {
            let mut state = self.state.borrow_mut();
            let Some(page) = state.page.as_mut() else {
                return;
            };
            let (poster, backdrop) = page.art_urls();
            match (size, &page.subject) {
                (Size::Poster, _) if poster == url => page.poster = Some(image.clone()),
                (Size::Backdrop, _) if backdrop == url => page.backdrop = Some(image.clone()),
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
                    return;
                }
                _ => return,
            }
        }
        self.push_page();
    }

    /// Bumps the page token; `None` when `provider` is gone. Returns the
    /// token, the provider's login if ready, and the provider.
    pub(super) fn begin_page(
        &self,
        provider: &str,
    ) -> Option<(u64, Option<Credentials>, Provider)> {
        let mut state = self.state.borrow_mut();
        let slot = state.slots.iter().find(|s| s.provider.id == provider)?;
        let (credentials, account) = (slot.credentials.clone(), slot.provider.clone());
        state.next_page += 1;
        Some((state.next_page, credentials, account))
    }

    /// Shows `page`, asking for its pictures.
    pub(super) fn show_page(&self, page: Page) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let kind = page.kind();
        self.state.borrow_mut().page = Some(page);
        self.request_page_art();
        self.push_page();
        app.set_details_rail(match kind {
            Kind::Movies => 3,
            Kind::Series => 4,
        });
        app.set_details_series(kind == Kind::Series);
        self.show(Screen::Details);
    }

    /// Asks for the page's poster and backdrop unless they are shown.
    pub(super) fn request_page_art(&self) {
        let wanted: Vec<(String, Size)> = {
            let state = self.state.borrow();
            let Some(page) = &state.page else {
                return;
            };
            let (poster, backdrop) = page.art_urls();
            [
                (poster, Size::Poster, page.poster.is_none()),
                (backdrop, Size::Backdrop, page.backdrop.is_none()),
            ]
            .into_iter()
            .filter(|(url, _, missing)| *missing && !url.is_empty())
            .map(|(url, size, _)| (url, size))
            .collect()
        };
        let mut art = self.art.borrow_mut();
        for (url, size) in wanted {
            art.request(&url, size);
        }
    }

    pub(super) fn push_page(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let details = match &self.state.borrow().page {
            Some(page) => page.details(),
            None => return,
        };
        app.set_details(details);
    }
}

// Private API
impl Session {
    fn movie_info_ready(&self, token: u64, result: Result<MovieInfo, String>) {
        {
            let mut state = self.state.borrow_mut();
            let Some(page) = state.page.as_mut().filter(|p| p.token == token) else {
                return;
            };
            let Subject::Movie { info, .. } = &mut page.subject else {
                return;
            };
            match result {
                Ok(fresh) => {
                    *info = Some(fresh);
                    page.note.clear();
                }
                // A cached copy is still worth showing without comment.
                Err(e) if info.is_none() => page.note = format!("Could not load details: {e}"),
                Err(_) => {}
            }
        }
        self.request_page_art();
        self.push_page();
    }

    fn set_page_note(&self, note: &str) {
        if let Some(page) = self.state.borrow_mut().page.as_mut() {
            note.clone_into(&mut page.note);
        }
        self.push_page();
    }
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

    fn kind(&self) -> Kind {
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

    fn details(&self) -> TitleDetails {
        let details = self.shared();
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
            play_label: "Play".into(),
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

fn movie_feature(
    page: &Page,
    movie: &Movie,
    info: &Option<MovieInfo>,
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
