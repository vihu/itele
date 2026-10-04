//! Audio and subtitle tracks: mpv's `track-list`, read as JSON, as the
//! banner's labels and as the player's track menus.

use mpv_engine::Engine;
use serde_json::Value;

use crate::ui::TrackItem;

/// Which tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Subtitles,
}

/// One track as the menu lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// mpv's id, counted per kind from 1.
    pub id: i64,
    /// Title, else language, else `Track 3`.
    pub name: String,
    /// What else is known, for example `ENG · AC-3 5.1`.
    pub detail: String,
    pub selected: bool,
}

/// The menu's id for "Off"; mpv counts tracks from 1.
const OFF: i64 = 0;

/// The tracks of `kind` mpv has now.
pub fn read(engine: &Engine, kind: Kind) -> Vec<Track> {
    list(&track_list(engine), kind)
}

/// The rows of the `kind` menu: the tracks, after "Off" for subtitles.
pub fn menu(engine: &Engine, kind: Kind) -> Vec<TrackItem> {
    let tracks = read(engine, kind);
    let off = (kind == Kind::Subtitles).then(|| TrackItem {
        id: OFF as i32,
        name: "Off".into(),
        detail: Default::default(),
        selected: !tracks.iter().any(|t| t.selected),
    });
    off.into_iter()
        .chain(tracks.into_iter().map(|t| TrackItem {
            id: t.id as i32,
            name: t.name.into(),
            detail: t.detail.into(),
            selected: t.selected,
        }))
        .collect()
}

/// Switches to track `id` of `kind`; for subtitles, [`OFF`] turns them off.
pub fn select(engine: &Engine, kind: Kind, id: i64) {
    let result = match (kind, id) {
        (Kind::Audio, id) => engine.set_property("aid", id),
        (Kind::Subtitles, OFF) => engine.set_property("sid", "no"),
        (Kind::Subtitles, id) => engine.set_property("sid", id),
    };
    if let Err(e) = result {
        eprintln!("player tracks: {e}");
    }
}

/// The selected track of `kind` for the banner, for example
/// `ENG · AC-3 5.1`; empty when none is selected.
pub fn label(json: &str, kind: Kind) -> String {
    let Some(track) = tracks(json, kind).into_iter().find(selected) else {
        return String::new();
    };
    let name = name(&track);
    match kind {
        Kind::Subtitles => name,
        Kind::Audio => [
            name,
            [codec(&track), channels(&track)]
                .join(" ")
                .trim()
                .to_owned(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · "),
    }
}

/// mpv's `track-list` node as JSON; empty when there is none.
pub fn track_list(engine: &Engine) -> String {
    engine
        .get_property::<String>("track-list")
        .unwrap_or_default()
}

/// The tracks of `kind` in a `track-list` node.
fn list(json: &str, kind: Kind) -> Vec<Track> {
    tracks(json, kind)
        .iter()
        .filter_map(|track| {
            let lang = text(track, "lang").map(str::to_uppercase);
            let detail = match kind {
                Kind::Audio => vec![
                    lang.filter(|_| text(track, "title").is_some()),
                    Some(codec(track)),
                    Some(channels(track)),
                ],
                Kind::Subtitles => vec![
                    lang.filter(|_| text(track, "title").is_some()),
                    Some(codec(track)),
                    flag(track, "forced").then(|| "Forced".to_owned()),
                    flag(track, "external").then(|| "External".to_owned()),
                ],
            };
            Some(Track {
                id: track.get("id")?.as_i64()?,
                name: name(track),
                detail: detail
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "),
                selected: selected(track),
            })
        })
        .collect()
}

/// The tracks of `kind` in a `track-list` node, in mpv's order.
fn tracks(json: &str, kind: Kind) -> Vec<Value> {
    let Ok(Value::Array(all)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let wanted = match kind {
        Kind::Audio => "audio",
        Kind::Subtitles => "sub",
    };
    all.into_iter()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some(wanted))
        .collect()
}

fn text<'a>(track: &'a Value, key: &str) -> Option<&'a str> {
    track
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn flag(track: &Value, key: &str) -> bool {
    track.get(key).and_then(Value::as_bool) == Some(true)
}

fn selected(track: &Value) -> bool {
    flag(track, "selected")
}

/// Title, else language, else `Track 3`.
fn name(track: &Value) -> String {
    match (text(track, "title"), text(track, "lang")) {
        (Some(title), _) => title.to_owned(),
        (None, Some(lang)) => lang.to_uppercase(),
        (None, None) => format!(
            "Track {}",
            track.get("id").and_then(Value::as_u64).unwrap_or(0)
        ),
    }
}

fn codec(track: &Value) -> String {
    match text(track, "codec") {
        Some("ac3") => "AC-3".to_owned(),
        Some("eac3") => "E-AC-3".to_owned(),
        Some("opus") => "Opus".to_owned(),
        Some("subrip") => "SRT".to_owned(),
        Some("ass" | "ssa") => "ASS".to_owned(),
        Some("hdmv_pgs_subtitle") => "PGS".to_owned(),
        Some("dvd_subtitle") => "VobSub".to_owned(),
        Some("dvb_subtitle") => "DVB".to_owned(),
        Some("dvb_teletext") => "Teletext".to_owned(),
        Some("webvtt") => "WebVTT".to_owned(),
        Some("mov_text") => "Text".to_owned(),
        Some(other) => other.to_uppercase(),
        None => String::new(),
    }
}

fn channels(track: &Value) -> String {
    match track.get("demux-channel-count").and_then(Value::as_u64) {
        Some(1) => "Mono".to_owned(),
        Some(2) => "Stereo".to_owned(),
        Some(6) => "5.1".to_owned(),
        Some(8) => "7.1".to_owned(),
        Some(n) => format!("{n} ch"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"[{"id":1,"type":"video","selected":true},
        {"id":1,"type":"audio","lang":"eng","codec":"ac3","demux-channel-count":6,"selected":true},
        {"id":2,"type":"audio","lang":"fra","title":"Commentary","codec":"aac","demux-channel-count":2},
        {"id":1,"type":"sub","title":"English SDH","codec":"subrip","selected":true},
        {"id":2,"type":"sub","lang":"nld","codec":"hdmv_pgs_subtitle","forced":true},
        {"id":3,"type":"sub","codec":"webvtt","external":true}]"#;

    #[test]
    fn menus_list_each_kind_with_details() {
        let audio = list(LIST, Kind::Audio);
        assert_eq!(audio.len(), 2);
        assert_eq!(
            (audio[0].name.as_str(), audio[0].detail.as_str()),
            ("ENG", "AC-3 · 5.1")
        );
        assert!(audio[0].selected);
        assert_eq!(
            (audio[1].name.as_str(), audio[1].detail.as_str()),
            ("Commentary", "FRA · AAC · Stereo"),
            "a titled track keeps its language in the detail"
        );
        let subs = list(LIST, Kind::Subtitles);
        let names: Vec<_> = subs
            .iter()
            .map(|t| (t.name.as_str(), t.detail.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("English SDH", "SRT"),
                ("NLD", "PGS · Forced"),
                ("Track 3", "WebVTT · External")
            ]
        );
        assert!(list("not json", Kind::Audio).is_empty());
    }

    #[test]
    fn labels_name_the_selected_track() {
        assert_eq!(label(LIST, Kind::Audio), "ENG · AC-3 5.1");
        assert_eq!(label(LIST, Kind::Subtitles), "English SDH");
        let none = r#"[{"id":3,"type":"audio","codec":"mp2","demux-channel-count":2,"selected":true},
            {"id":1,"type":"sub","lang":"deu","selected":false}]"#;
        assert_eq!(label(none, Kind::Audio), "Track 3 · MP2 Stereo");
        assert_eq!(label(none, Kind::Subtitles), "");
        assert_eq!(label("", Kind::Audio), "");
    }
}
