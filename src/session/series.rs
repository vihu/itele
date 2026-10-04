//! A series' page: its seasons, the selected season's episodes with their
//! stills, and the episode to play.

use std::rc::Rc;
use std::thread;

use itele::xtream::{self, Action, Client, Credentials, Episode, Show, ShowInfo};
use slint::{Model, ModelRc, SharedString, VecModel};

use super::details::{Feature, Page, Subject, loading_note};
use super::timefmt::runtime;
use super::{Session, on_ui_thread};
use crate::art::Size;
use crate::ui::{EpisodeItem, ProgrammeInfo, SeasonItem};

impl Session {
    /// Opens the page of `provider`'s `show`.
    pub(super) fn open_show(&self, provider: String, show: Show) {
        let Some((token, credentials, account)) = self.begin_page(&provider) else {
            return;
        };
        let cache = self.paths.cache(&account);
        let action = Action::ShowInfo(show.id);
        let info = cache.load(action, xtream::parse_show_info);
        let loading = info.is_none();
        let subject = Subject::Show {
            show,
            info,
            season: 0,
            episodes: Rc::default(),
            stills: Vec::new(),
        };
        self.show_page(Page::new(token, provider, subject, loading_note(loading)));
        self.fill_season(0);
        thread::spawn(move || {
            let result = credentials
                .map_or_else(|| account.credentials(), Ok)
                .and_then(|c| cache.refresh(&Client::new(c), action, xtream::parse_show_info))
                .map_err(|e| e.to_string());
            on_ui_thread(move |s| s.show_info_ready(token, result));
        });
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
    fn fill_season(&self, index: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let (seasons, season, episodes, stills) = {
            let mut state = self.state.borrow_mut();
            let Some(Subject::Show {
                info,
                season,
                episodes,
                stills,
                ..
            }) = state.page.as_mut().map(|p| &mut p.subject)
            else {
                return;
            };
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
                listed.into_iter().map(episode_item).collect::<Vec<_>>(),
            ));
            (seasons, *season, Rc::clone(episodes), stills.clone())
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
            show, info, season, ..
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
        Some(Feature {
            url: credentials.episode_url(&episode.id, &episode.extension),
            name: format!("{} · {code}", show.name),
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

fn episode_item(episode: &Episode) -> EpisodeItem {
    EpisodeItem {
        number: episode.number.to_string().into(),
        title: episode.title.as_str().into(),
        plot: episode.plot.as_str().into(),
        duration: duration(episode).unwrap_or_default().into(),
        code: code(episode).into(),
        ..EpisodeItem::default()
    }
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
}
