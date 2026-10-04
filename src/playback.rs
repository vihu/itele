//! The live player's controls and what its banner shows: pause, the
//! timeshift window in mpv's cache, volume, and audio and subtitle tracks.
//!
//! mpv returns node properties (`track-list`, `demuxer-cache-state`) as JSON
//! when read as strings, which is how they are parsed here.

use mpv_engine::Engine;
use serde_json::Value;
use slint::SharedString;

use crate::ui::PlayerState;

/// Seconds behind the newest cached frame that still count as live.
const LIVE_SLACK: f64 = 3.0;
/// Loudest volume mpv allows by default, in percent.
const VOLUME_MAX: f64 = 130.0;
/// Volume step for the volume keys, in percent.
pub const VOLUME_STEP: f64 = 5.0;
/// Jump for the seek keys and buttons, in seconds.
pub const SEEK_STEP: f64 = 10.0;

/// Reads what the player banner shows. When replaying a programme of
/// `replay` seconds, the timeline is the programme, not the cached window of
/// a live stream; the guide's length is used because a catch-up stream
/// rarely reports its own.
pub fn read(engine: &Engine, fullscreen: bool, replay: Option<f64>) -> PlayerState {
    if let Some(duration) = replay {
        let position = engine.position().unwrap_or(0.0);
        let tracks = engine
            .get_property::<String>("track-list")
            .unwrap_or_default();
        return PlayerState {
            paused: engine.is_paused(),
            muted: engine.is_muted(),
            volume: engine.volume().unwrap_or(100.0).round() as i32,
            behind: SharedString::new(),
            position: if duration > 0.0 {
                (position / duration).clamp(0.0, 1.0) as f32
            } else {
                0.0
            },
            audio: track_label(&tracks, "audio").into(),
            subtitles: track_label(&tracks, "sub").into(),
            fullscreen,
            replay: true,
            time: format!("{} / {}", clock(position), clock(duration)).into(),
        };
    }
    let window = engine
        .get_property::<String>("demuxer-cache-state")
        .ok()
        .and_then(|json| cache_window(&json));
    let (behind, position) = match (engine.position(), window) {
        (Some(pos), Some((start, end))) if end > start => (
            (end - pos).max(0.0),
            ((pos - start) / (end - start)).clamp(0.0, 1.0),
        ),
        _ => (0.0, 1.0),
    };
    let tracks = engine
        .get_property::<String>("track-list")
        .unwrap_or_default();
    PlayerState {
        paused: engine.is_paused(),
        muted: engine.is_muted(),
        volume: engine.volume().unwrap_or(100.0).round() as i32,
        behind: if behind > LIVE_SLACK {
            clock(behind).into()
        } else {
            SharedString::new()
        },
        position: position as f32,
        audio: track_label(&tracks, "audio").into(),
        subtitles: track_label(&tracks, "sub").into(),
        fullscreen,
        replay: false,
        time: SharedString::new(),
    }
}

/// Pauses or resumes. A paused live stream keeps filling the cache, so
/// resuming continues where it stopped.
pub fn toggle_pause(engine: &Engine) {
    report("pause", engine.set_paused(!engine.is_paused()));
}

/// Jumps `seconds` within the cached window (negative is back).
pub fn seek(engine: &Engine, seconds: f64) {
    report("seek", engine.seek_relative(seconds));
}

/// Jumps to `fraction` (0 to 1) of a replayed programme `length` seconds
/// long.
pub fn seek_to_part(engine: &Engine, fraction: f32, length: f64) {
    report(
        "seek",
        engine.seek_absolute(length * f64::from(fraction.clamp(0.0, 1.0))),
    );
}

/// Jumps to `fraction` (0 to 1) of the cached window.
pub fn seek_to(engine: &Engine, fraction: f32) {
    let window = engine
        .get_property::<String>("demuxer-cache-state")
        .ok()
        .and_then(|json| cache_window(&json));
    if let Some((start, end)) = window {
        let target = start + (end - start) * f64::from(fraction.clamp(0.0, 1.0));
        report("seek", engine.seek_absolute(target));
    }
}

/// Sets the volume to `percent`, unmuting.
pub fn set_volume(engine: &Engine, percent: f32) {
    report(
        "volume",
        engine.set_volume(f64::from(percent).clamp(0.0, VOLUME_MAX)),
    );
    if engine.is_muted() {
        report("unmute", engine.set_muted(false));
    }
}

/// Mutes or unmutes.
pub fn toggle_mute(engine: &Engine) {
    report("mute", engine.set_muted(!engine.is_muted()));
}

/// Changes the volume by `delta` percent, within 0 to 130.
pub fn change_volume(engine: &Engine, delta: f64) {
    let volume = (engine.volume().unwrap_or(100.0) + delta).clamp(0.0, VOLUME_MAX);
    report("volume", engine.set_volume(volume));
    if engine.is_muted() && delta > 0.0 {
        report("unmute", engine.set_muted(false));
    }
}

/// Switches to the next audio track. Unlike mpv's `cycle aid`, never
/// passes through "no audio", and does nothing with a single track.
pub fn next_audio(engine: &Engine) {
    let tracks = engine
        .get_property::<String>("track-list")
        .unwrap_or_default();
    if let Some(id) = next_audio_id(&tracks) {
        report("audio", engine.set_property("aid", id));
    }
}

/// Switches to the next subtitle track, passing through off.
pub fn next_subtitles(engine: &Engine) {
    report("subtitles", engine.command("cycle", &["sid"]));
}

fn report(what: &str, result: mpv_engine::Result<()>) {
    if let Err(e) = result {
        eprintln!("player {what}: {e}");
    }
}

/// The audio track after the selected one in a `track-list` node, wrapping
/// around; `None` with fewer than two audio tracks.
fn next_audio_id(json: &str) -> Option<i64> {
    let Ok(Value::Array(tracks)) = serde_json::from_str::<Value>(json) else {
        return None;
    };
    let audio: Vec<(i64, bool)> = tracks
        .iter()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("audio"))
        .filter_map(|t| {
            let selected = t.get("selected").and_then(Value::as_bool) == Some(true);
            Some((t.get("id")?.as_i64()?, selected))
        })
        .collect();
    if audio.len() < 2 {
        return None;
    }
    let current = audio
        .iter()
        .position(|&(_, selected)| selected)
        .unwrap_or(0);
    Some(audio[(current + 1) % audio.len()].0)
}

/// The cached window in a `demuxer-cache-state` node, as `(start, end)` in
/// stream seconds. The end is `cache-end`, the newest demuxed data, which
/// follows the live stream; the seekable range only grows at keyframes. The
/// start is where the last seekable range begins.
fn cache_window(json: &str) -> Option<(f64, f64)> {
    let state: Value = serde_json::from_str(json).ok()?;
    let end = state.get("cache-end")?.as_f64()?;
    let start = state
        .get("seekable-ranges")
        .and_then(Value::as_array)
        .and_then(|ranges| ranges.last()?.get("start")?.as_f64())
        .unwrap_or(end);
    Some((start.min(end), end))
}

/// The selected track of `kind` (`audio` or `sub`) in a `track-list` node,
/// for example `ENG · AC-3 5.1`. Empty when none is selected.
fn track_label(json: &str, kind: &str) -> String {
    let Ok(Value::Array(tracks)) = serde_json::from_str::<Value>(json) else {
        return String::new();
    };
    let Some(track) = tracks.iter().find(|t| {
        t.get("type").and_then(Value::as_str) == Some(kind)
            && t.get("selected").and_then(Value::as_bool) == Some(true)
    }) else {
        return String::new();
    };
    let text = |key| {
        track
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let name = match (text("title"), text("lang")) {
        (Some(title), _) => title.to_owned(),
        (None, Some(lang)) => lang.to_uppercase(),
        (None, None) => format!(
            "Track {}",
            track.get("id").and_then(Value::as_u64).unwrap_or(0)
        ),
    };
    if kind != "audio" {
        return name;
    }
    let codec = match text("codec") {
        Some("ac3") => "AC-3".to_owned(),
        Some("eac3") => "E-AC-3".to_owned(),
        Some("opus") => "Opus".to_owned(),
        Some(other) => other.to_uppercase(),
        None => String::new(),
    };
    let channels = match track.get("demux-channel-count").and_then(Value::as_u64) {
        Some(1) => "Mono".to_owned(),
        Some(2) => "Stereo".to_owned(),
        Some(6) => "5.1".to_owned(),
        Some(8) => "7.1".to_owned(),
        Some(n) => format!("{n} ch"),
        None => String::new(),
    };
    [name, [codec, channels].join(" ").trim().to_owned()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `42.0` as `0:42`, `3725.0` as `1:02:05`.
fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_window_ends_at_cache_end() {
        // Shape from mpv 0.41 while paused on a live stream: the seekable
        // range lags at the last keyframe, cache-end keeps growing.
        let json = r#"{"cache-end":17.113111,"reader-pts":9.404078,"cache-duration":7.709033,
            "ts-per-stream":[{"type":"video","cache-end":18.6}],
            "seekable-ranges":[{"start":10.0,"end":12.0},{"start":-0.0,"end":9.983222}]}"#;
        assert_eq!(cache_window(json), Some((-0.0, 17.113111)));
        let no_ranges = r#"{"cache-end":8.5,"seekable-ranges":[]}"#;
        assert_eq!(cache_window(no_ranges), Some((8.5, 8.5)));
        assert_eq!(cache_window(r#"{"seekable-ranges":[]}"#), None);
        assert_eq!(cache_window("not json"), None);
    }

    #[test]
    fn audio_label_names_language_codec_and_layout() {
        let json = r#"[{"id":1,"type":"video","selected":true},
            {"id":1,"type":"audio","lang":"eng","codec":"ac3","demux-channel-count":6,"selected":true},
            {"id":2,"type":"audio","lang":"fra","codec":"aac","selected":false},
            {"id":1,"type":"sub","title":"English SDH","selected":true}]"#;
        assert_eq!(track_label(json, "audio"), "ENG · AC-3 5.1");
        assert_eq!(track_label(json, "sub"), "English SDH");
    }

    #[test]
    fn labels_fall_back_and_handle_nothing_selected() {
        let json = r#"[{"id":3,"type":"audio","codec":"mp2","demux-channel-count":2,"selected":true},
            {"id":1,"type":"sub","lang":"deu","selected":false}]"#;
        assert_eq!(track_label(json, "audio"), "Track 3 · MP2 Stereo");
        assert_eq!(track_label(json, "sub"), "");
        assert_eq!(track_label("", "audio"), "");
    }

    #[test]
    fn next_audio_skips_no_audio_and_wraps() {
        let two = r#"[{"id":1,"type":"audio","selected":false},
            {"id":2,"type":"audio","selected":true},{"id":1,"type":"sub","selected":false}]"#;
        assert_eq!(next_audio_id(two), Some(1));
        let one = r#"[{"id":1,"type":"audio","selected":true}]"#;
        assert_eq!(next_audio_id(one), None);
        assert_eq!(next_audio_id("[]"), None);
    }

    #[test]
    fn clock_formats_minutes_and_hours() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(42.4), "0:42");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(-3.0), "0:00");
    }
}
