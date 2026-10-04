//! A series' page: its seasons, the selected season's episodes with their
//! stills, and the episode to play.

use std::collections::HashMap;
use std::rc::Rc;
use std::thread;

use itele::history::{Entry, Kind as Watched, Progress};
use itele::xtream::{self, Action, Client, Credentials, Episode, Show, ShowInfo};
use slint::{Model, ModelRc, SharedString, VecModel};

use super::page::{Feature, Page, Subject, loading_note};
use super::timefmt::{minutes_left, runtime};
use super::{Session, Start, on_ui_thread};
use crate::art::Size;
use crate::ui::{EpisodeItem, ProgrammeInfo, Screen, SeasonItem};

impl Session {
    /// Opens the page of `provider`'s `show`, from the `origin` screen.
    pub(super) fn open_show(&self, provider: String, show: Show, origin: Screen) {
        let Some((token, credentials, account)) = self.begin_page(&provider) else {
            return;
        };
        let cache = self.paths.cache(&account);
        let action = Action::ShowInfo(show.id);
        let info = cache.load(action, xtream::parse_show_info);
        let loading = info.is_none();
        let (watched, last) = self
            .history
            .as_ref()
            .and_then(|h| h.episodes(&provider, show.id.0).ok())
            .unwrap_or_default();
        let subject = Subject::Show {
            show,
            info,
            season: 0,
            episodes: Rc::default(),
            stills: Vec::new(),
            watched,
            last,
            targeted: false,
        };
        self.show_page(Page::new(
            token,
            provider,
            subject,
            loading_note(loading),
            origin,
        ));
        self.fill_season(0);
        thread::spawn(move || {
            let result = credentials
                .map_or_else(|| account.credentials(), Ok)
                .and_then(|c| cache.refresh(&Client::new(c), action, xtream::parse_show_info))
                .map_err(|e| e.to_string());
            on_ui_thread(move |s| s.show_info_ready(token, result));
        });
    }

    /// Plays the episode the page selected after `finished` ended, unless
    /// `finished` was the last.
    pub(super) fn play_next_episode(&self, finished: &str) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let selected = {
            let state = self.state.borrow();
            let Some(Subject::Show { info, season, .. }) = state.page.as_ref().map(|p| &p.subject)
            else {
                return;
            };
            usize::try_from(app.get_details_episode_index())
                .ok()
                .zip(info.as_ref())
                .and_then(|(index, info)| {
                    season_episodes(info, *season)
                        .get(index)
                        .map(|e| e.id.0.clone())
                })
        };
        if selected.is_some_and(|id| id != finished) {
            self.details_play(Start::Resume);
        }
    }

    /// A season tab picked: its episodes, from the first.
    pub(super) fn details_season_selected(&self, index: i32) {
        {
            let mut state = self.state.borrow_mut();
            let Some(Subject::Show { info, season, .. }) =
                state.page.as_mut().map(|p| &mut p.subject)
            else {
                return;
            };
            let count = info.as_ref().map_or(0, |i| i.seasons.len());
            match usize::try_from(index) {
                Ok(index) if index < count => *season = index,
                _ => return,
            }
        }
        self.fill_season(0);
    }
}

// Private API
impl Session {
    fn show_info_ready(&self, token: u64, result: Result<ShowInfo, String>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let Some(page) = state.page.as_mut().filter(|p| p.token == token) else {
                return;
            };
            let Subject::Show { info, season, .. } = &mut page.subject else {
                return;
            };
            match result {
                Ok(fresh) => {
                    if *season >= fresh.seasons.len() {
                        *season = 0;
                    }
                    *info = Some(fresh);
                    page.note.clear();
                }
                // A cached copy is still worth showing without comment.
                Err(e) if info.is_none() => page.note = format!("Could not load episodes: {e}"),
                Err(_) => return,
            }
        }
        self.request_page_art();
        self.push_page();
        self.fill_season(app.get_details_episode_index());
    }

    /// Lists the selected season's episodes with episode `index` selected,
    /// and asks for their stills.
    pub(super) fn fill_season(&self, index: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (seasons, season, episodes, stills, index) = {
            let mut state = self.state.borrow_mut();
            let resume_on = state.page.as_ref().is_some_and(|p| p.resume_on);
            let Some(Subject::Show {
                info,
                season,
                episodes,
                stills,
                watched,
                last,
                targeted,
                ..
            }) = state.page.as_mut().map(|p| &mut p.subject)
            else {
                return;
            };
            let mut index = index;
            if let Some(info) = info.as_ref().filter(|_| !*targeted) {
                *targeted = true;
                if let Some((target_season, target_index)) =
                    resume_target(info, watched, last.as_deref())
                {
                    *season = target_season;
                    index = target_index as i32;
                }
            }
            let (seasons, listed) = match info {
                Some(info) => {
                    let seasons = info
                        .seasons
                        .iter()
                        .map(|s| SeasonItem {
                            name: s.name.as_str().into(),
                            count: info
                                .episodes
                                .iter()
                                .filter(|e| e.season == s.number)
                                .count()
                                .to_string()
                                .into(),
                        })
                        .collect();
                    (seasons, season_episodes(info, *season))
                }
                None => (Vec::new(), Vec::new()),
            };
            *stills = listed
                .iter()
                .map(|e| e.still.clone().unwrap_or_default())
                .collect();
            *episodes = Rc::new(VecModel::from(
                listed
                    .into_iter()
                    .map(|e| episode_item(e, watched.get(&e.id.0), resume_on))
                    .collect::<Vec<_>>(),
            ));
            (seasons, *season, Rc::clone(episodes), stills.clone(), index)
        };
        let len = episodes.row_count() as i32;
        app.set_details_seasons(ModelRc::new(VecModel::from(seasons)));
        app.set_details_season_index(season as i32);
        app.set_details_episodes(ModelRc::from(episodes));
        app.set_details_episode_index(if len == 0 {
            -1
        } else {
            index.clamp(0, len - 1)
        });
        app.invoke_reveal_episode();
        let mut art = self.art.borrow_mut();
        for url in stills.iter().filter(|u| !u.is_empty()) {
            art.request(url, Size::Still);
        }
    }
}

impl Page {
    /// Episode `index` of the selected season, ready to play.
    pub(super) fn episode_feature(
        &self,
        index: usize,
        credentials: &Credentials,
    ) -> Option<Feature> {
        let Subject::Show {
            show,
            info,
            season,
            watched,
            ..
        } = &self.subject
        else {
            return None;
        };
        let episode = *season_episodes(info.as_ref()?, *season).get(index)?;
        let code = code(episode);
        let subtitle = [
            Some(show.name.clone()),
            Some(code.clone()),
            duration(episode),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let name = format!("{} · {code}", show.name);
        Some(Feature {
            url: credentials.episode_url(&episode.id, &episode.extension),
            entry: Entry {
                provider: self.provider.clone(),
                kind: Watched::Episode,
                id: episode.id.0.clone(),
                series: Some(show.id.0),
                title: name.clone(),
            },
            resume: watched
                .get(&episode.id.0)
                .and_then(Progress::resume_at)
                .filter(|_| self.resume_on),
            name,
            programme: ProgrammeInfo {
                title: episode.title.as_str().into(),
                time: subtitle.into(),
                left: SharedString::new(),
                description: episode.plot.as_str().into(),
                progress: 0.0,
            },
        })
    }
}

/// The episodes of the season at index `season`.
fn season_episodes(info: &ShowInfo, season: usize) -> Vec<&Episode> {
    let Some(number) = info.seasons.get(season).map(|s| s.number) else {
        return Vec::new();
    };
    info.episodes
        .iter()
        .filter(|e| e.season == number)
        .collect()
}

fn episode_item(episode: &Episode, progress: Option<&Progress>, resume_on: bool) -> EpisodeItem {
    let resume = progress.filter(|p| resume_on && p.resume_at().is_some());
    EpisodeItem {
        number: episode.number.to_string().into(),
        title: episode.title.as_str().into(),
        plot: episode.plot.as_str().into(),
        duration: duration(episode).unwrap_or_default().into(),
        code: code(episode).into(),
        progress: match progress {
            Some(p) if p.watched => 1.0,
            Some(p) if resume.is_some() => p.fraction(),
            _ => -1.0,
        },
        watched: progress.is_some_and(|p| p.watched),
        resumable: resume.is_some(),
        left: resume
            .map(|p| minutes_left(p.left() as i64))
            .unwrap_or_default()
            .into(),
        ..EpisodeItem::default()
    }
}

/// The season and episode to select on opening: the episode watched last,
/// or the one after it when it was finished. Indexes are into the seasons
/// and into that season's episodes.
fn resume_target(
    info: &ShowInfo,
    watched: &HashMap<String, Progress>,
    last: Option<&str>,
) -> Option<(usize, usize)> {
    let at = info
        .episodes
        .iter()
        .position(|e| Some(e.id.0.as_str()) == last)?;
    let finished = watched
        .get(&info.episodes[at].id.0)
        .is_some_and(|p| p.watched);
    let target = match info.episodes.get(at + 1) {
        Some(next) if finished => next,
        _ => &info.episodes[at],
    };
    let season = info
        .seasons
        .iter()
        .position(|s| s.number == target.season)?;
    let index = info
        .episodes
        .iter()
        .filter(|e| e.season == target.season)
        .position(|e| e.id == target.id)?;
    Some((season, index))
}

/// For example `S2 E4`.
fn code(episode: &Episode) -> String {
    format!("S{} E{}", episode.season, episode.number)
}

fn duration(episode: &Episode) -> Option<String> {
    episode
        .duration
        .filter(|&secs| secs >= 60)
        .map(|secs| runtime(i64::from(secs)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seasons_pick_their_episodes() {
        let info = xtream::parse_show_info(
            r#"{"seasons":[{"season_number":1},{"season_number":3}],
                "episodes":{"1":[{"id":"a","episode_num":1}],
                            "3":[{"id":"b","episode_num":1},{"id":"c","episode_num":2,"title":"Late"}]}}"#,
        )
        .unwrap();
        let ids = |season| {
            season_episodes(&info, season)
                .iter()
                .map(|e| e.id.0.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(0), ["a"]);
        assert_eq!(ids(1), ["b", "c"]);
        assert!(ids(2).is_empty());
        assert_eq!(code(season_episodes(&info, 1)[1]), "S3 E2");
    }

    #[test]
    fn resume_targets_the_last_episode_or_the_next() {
        let info = xtream::parse_show_info(
            r#"{"episodes":{"1":[{"id":"a","episode_num":1},{"id":"b","episode_num":2}],
                            "2":[{"id":"c","episode_num":1}]}}"#,
        )
        .unwrap();
        let progress = |position, watched| Progress {
            position,
            duration: 1500.0,
            watched,
            updated: 0,
        };
        let mut watched = HashMap::new();
        assert_eq!(resume_target(&info, &watched, None), None);
        watched.insert("b".to_owned(), progress(600.0, false));
        assert_eq!(resume_target(&info, &watched, Some("b")), Some((0, 1)));
        watched.insert("b".to_owned(), progress(1500.0, true));
        assert_eq!(
            resume_target(&info, &watched, Some("b")),
            Some((1, 0)),
            "finished: the next, in the next season"
        );
        watched.insert("c".to_owned(), progress(1500.0, true));
        assert_eq!(
            resume_target(&info, &watched, Some("c")),
            Some((1, 0)),
            "the last stays"
        );
    }
}
