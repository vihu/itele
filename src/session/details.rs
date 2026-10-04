//! The detail page of a movie: what the list knows at once, then the
//! provider's details (cached per title, refreshed on every open), and
//! playing it.

use std::thread;

use itele::xtream::{self, Action, Client, Details, Movie, MovieInfo};
use slint::{Image, SharedString};

use super::timefmt::runtime;
use super::vod::Kind;
use super::{Content, Playing, Session, Title, on_ui_thread};
use crate::art::Size;
use crate::names::tint;
use crate::ui::{ProgrammeInfo, TitleDetails};

/// The page showing.
pub(super) struct Page {
    /// Tells this page's background results from an earlier page's.
    token: u64,
    provider: String,
    kind: Kind,
    subject: Subject,
    poster: Option<Image>,
    backdrop: Option<Image>,
    /// Shown while details load, or when they fail.
    note: String,
}

/// What the page is about, with the provider's details once known.
pub(super) enum Subject {
    Movie {
        movie: Movie,
        info: Option<MovieInfo>,
    },
}

impl Session {
    /// Opens the page of `provider`'s `movie`.
    pub(super) fn open_movie(&self, provider: String, movie: Movie) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (token, credentials, account) = {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state.slots.iter().find(|s| s.provider.id == provider) else {
                return;
            };
            let (credentials, account) = (slot.credentials.clone(), slot.provider.clone());
            state.next_page += 1;
            (state.next_page, credentials, account)
        };
        let cache = self.paths.cache(&account);
        let action = Action::MovieInfo(movie.id);
        let info = cache.load(action, xtream::parse_movie_info);
        let note = if info.is_some() {
            String::new()
        } else {
            "Loading details…".to_owned()
        };
        self.state.borrow_mut().page = Some(Page {
            token,
            provider,
            kind: Kind::Movies,
            subject: Subject::Movie { movie, info },
            poster: None,
            backdrop: None,
            note,
        });
        self.request_page_art();
        self.push_page();
        app.set_details_rail(3);
        self.show(crate::ui::Screen::Details);

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
        let Some(kind) = self.state.borrow_mut().page.take().map(|p| p.kind) else {
            return;
        };
        self.show(kind.screen());
        app.invoke_reveal_vod();
    }

    /// Plays the page's title in the player.
    pub(super) fn details_play(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (provider, url, name, programme) = {
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
            let details = page_details(page);
            let Subject::Movie { movie, info } = &page.subject;
            let extension = info
                .as_ref()
                .and_then(|i| i.extension.as_deref())
                .unwrap_or(&movie.extension);
            let programme = ProgrammeInfo {
                title: details.title.clone(),
                time: details.meta.clone(),
                left: SharedString::new(),
                description: details.plot.clone(),
                progress: 0.0,
            };
            (
                page.provider.clone(),
                credentials.movie_url(movie.id, extension),
                movie.name.clone(),
                programme,
            )
        };
        self.select_timer.stop();
        if self.engine.load(&url).is_err() {
            return self.set_page_note("Could not start playback");
        }
        let provider_name = self
            .state
            .borrow()
            .slots
            .iter()
            .find(|s| s.provider.id == provider)
            .map(|s| s.provider.name.clone())
            .unwrap_or_default();
        app.set_video_note("Loading…".into());
        app.set_provider_name(provider_name.into());
        app.set_programme(programme);
        self.state.borrow_mut().playing = Some(Playing {
            provider,
            content: Content::Title(Title { name }),
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
            let (poster, backdrop) = art_urls(page);
            match size {
                Size::Poster if poster == url => page.poster = Some(image.clone()),
                Size::Backdrop if backdrop == url => page.backdrop = Some(image.clone()),
                _ => return,
            }
        }
        self.push_page();
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
            let Subject::Movie { info, .. } = &mut page.subject;
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

    /// Asks for the page's poster and backdrop unless they are shown.
    fn request_page_art(&self) {
        let wanted: Vec<(String, Size)> = {
            let state = self.state.borrow();
            let Some(page) = &state.page else {
                return;
            };
            let (poster, backdrop) = art_urls(page);
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

    fn push_page(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let details = match &self.state.borrow().page {
            Some(page) => page_details(page),
            None => return,
        };
        app.set_details(details);
    }

    fn set_page_note(&self, note: &str) {
        if let Some(page) = self.state.borrow_mut().page.as_mut() {
            note.clone_into(&mut page.note);
        }
        self.push_page();
    }
}

/// The page's poster and backdrop URLs, empty when it has none.
fn art_urls(page: &Page) -> (String, String) {
    let Subject::Movie { movie, info } = &page.subject;
    let details = info.as_ref().map(|i| &i.details);
    let poster = movie
        .poster
        .clone()
        .or_else(|| details.and_then(|d| d.poster.clone()))
        .unwrap_or_default();
    let backdrop = details.and_then(|d| d.backdrop.clone()).unwrap_or_default();
    (poster, backdrop)
}

fn page_details(page: &Page) -> TitleDetails {
    let Subject::Movie { movie, info } = &page.subject;
    let details = info.as_ref().map(|i| &i.details);
    let meta = [
        movie.year.map(|y| y.to_string()),
        details
            .and_then(|d| d.duration)
            .filter(|&secs| secs >= 60)
            .map(|secs| runtime(i64::from(secs))),
        details.map(|d| d.genre.clone()).filter(|g| !g.is_empty()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    TitleDetails {
        title: movie.name.as_str().into(),
        meta: meta.into(),
        rating: movie
            .rating
            .or(details.and_then(|d| d.rating))
            .map(|r| format!("{r:.1}").into())
            .unwrap_or_default(),
        plot: details.map(|d| d.plot.as_str()).unwrap_or("").into(),
        credits: details.map(credits).unwrap_or_default().into(),
        tint: tint(&movie.name),
        has_poster: page.poster.is_some(),
        poster: page.poster.clone().unwrap_or_default(),
        has_backdrop: page.backdrop.is_some(),
        backdrop: page.backdrop.clone().unwrap_or_default(),
        note: page.note.as_str().into(),
        back_label: "Movies".into(),
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
