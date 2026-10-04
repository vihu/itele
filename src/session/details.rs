//! The detail page of a movie or a series: what the list knows at once,
//! then the provider's details (cached per title, refreshed on every open),
//! and playing it. What a page holds is in `page.rs`; the series' side,
//! seasons and episodes, in `series.rs`.

use std::thread;

use itele::history::Kind as Watched;
use itele::provider::Provider;
use itele::xtream::{self, Action, Client, Credentials, Movie, MovieInfo};
use slint::Image;

use super::page::{Page, Subject, loading_note, movie_feature};
use super::vod::Kind;
use super::{Content, Playing, Session, Start, Title, on_ui_thread};
use crate::art::Size;
use crate::ui::Screen;

impl Session {
    /// Opens the page of `provider`'s `movie`, from the `origin` screen.
    pub(super) fn open_movie(&self, provider: String, movie: Movie, origin: Screen) {
        let Some((token, credentials, account)) = self.begin_page(&provider) else {
            return;
        };
        let cache = self.paths.cache(&account);
        let action = Action::MovieInfo(movie.id);
        let info = cache.load(action, xtream::parse_movie_info);
        let loading = info.is_none();
        let progress = self.history.as_ref().and_then(|h| {
            h.progress(&provider, Watched::Movie, &movie.id.0.to_string())
                .ok()
                .flatten()
        });
        let subject = Subject::Movie {
            movie,
            info,
            progress,
        };
        self.show_page(Page::new(
            token,
            provider,
            subject,
            loading_note(loading),
            origin,
        ));
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
        let Some(origin) = self.state.borrow_mut().page.take().map(|p| p.origin) else {
            return;
        };
        self.show(origin);
        if matches!(origin, Screen::Movies | Screen::Series) {
            self.apply_progress();
            app.invoke_reveal_vod();
        }
        if origin == Screen::Favorites {
            self.fill_favorite_titles();
        }
    }

    /// Plays the page's movie, or the selected episode of its series,
    /// from `start`.
    pub(super) fn details_play(&self, start: Start) {
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
                Subject::Movie {
                    movie,
                    info,
                    progress,
                } => movie_feature(page, movie, info, progress, credentials),
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
        let at = match start {
            Start::Resume => feature.resume,
            Start::Beginning => None,
        };
        if self.load(&feature.url, at).is_err() {
            return self.set_page_note("Could not start playback");
        }
        self.title_length.set(0.0);
        app.set_video_note("Loading…".into());
        app.set_provider_name(provider_name.into());
        app.set_programme(feature.programme);
        self.state.borrow_mut().playing = Some(Playing {
            provider,
            content: Content::Title(Title {
                name: feature.name,
                entry: feature.entry,
            }),
        });
        self.enter_player();
    }

    /// A picture for the page arrived.
    pub(super) fn page_art_ready(&self, url: &str, size: Size, image: &Image) {
        let changed = self
            .state
            .borrow_mut()
            .page
            .as_mut()
            .is_some_and(|page| page.set_art(url, size, image));
        if changed {
            self.push_page();
        }
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
    pub(super) fn show_page(&self, mut page: Page) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        page.resume_on = self.settings.borrow().resume;
        page.favorite = self.is_favorite(&page.as_favorite());
        let kind = page.kind();
        let origin = page.origin;
        self.state.borrow_mut().page = Some(page);
        self.request_page_art();
        self.push_page();
        app.set_details_rail(match (origin, kind) {
            (Screen::Favorites, _) => 7,
            (Screen::Home, _) => 0,
            (_, Kind::Movies) => 3,
            (_, Kind::Series) => 4,
        });
        app.set_details_series(kind == Kind::Series);
        self.show(Screen::Details);
    }

    /// Asks for the page's poster and backdrop unless they are shown.
    pub(super) fn request_page_art(&self) {
        let wanted = match &self.state.borrow().page {
            Some(page) => page.missing_art(),
            None => return,
        };
        let mut art = self.art.borrow_mut();
        for (url, size) in wanted {
            art.request(&url, size);
        }
    }

    /// Reads again how far the page's title, or its series' episodes, were
    /// watched; on a series, selects the episode to resume.
    pub(super) fn refresh_page_history(&self) {
        let Some(history) = &self.history else {
            return;
        };
        let resume_on = self.settings.borrow().resume;
        let series = {
            let mut state = self.state.borrow_mut();
            let Some(page) = state.page.as_mut() else {
                return;
            };
            page.resume_on = resume_on;
            let provider = page.provider.clone();
            match &mut page.subject {
                Subject::Movie {
                    movie, progress, ..
                } => {
                    *progress = history
                        .progress(&provider, Watched::Movie, &movie.id.0.to_string())
                        .ok()
                        .flatten();
                    false
                }
                Subject::Show {
                    show,
                    watched,
                    last,
                    targeted,
                    ..
                } => {
                    if let Ok((episodes, latest)) = history.episodes(&provider, show.id.0) {
                        *watched = episodes;
                        *last = latest;
                        *targeted = false;
                    }
                    true
                }
            }
        };
        self.push_page();
        if series {
            self.fill_season(0);
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
