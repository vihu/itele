//! The stream info overlay (`i`), after mpv's stats page: stream, display,
//! video, and audio, read from mpv properties.
//!
//! The stream URL carries the password, so the overlay names the channel
//! and provider instead of the file.

use mpv_engine::Engine;
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::ui::{InfoLine, InfoPair};

/// Builds the overlay for the stream playing `channel` from `provider`.
pub fn read(engine: &Engine, channel: &str, provider: &str) -> Vec<InfoLine> {
    let text = |name: &str| engine.get_property::<String>(name).unwrap_or_default();
    let int = |name: &str| engine.get_property::<i64>(name).ok();
    let float = |name: &str| engine.get_property::<f64>(name).ok();

    let cache_bytes = int("demuxer-cache-state/total-bytes")
        .or_else(|| total_bytes(&text("demuxer-cache-state")));
    let cache = match (cache_bytes, float("demuxer-cache-duration")) {
        (Some(bytes), Some(secs)) => format!("{}  ({secs:.1} sec)", bytes_text(bytes)),
        (None, Some(secs)) => format!("{secs:.1} sec"),
        _ => String::new(),
    };
    let resolution = match (int("video-params/w"), int("video-params/h")) {
        (Some(w), Some(h)) => format!("{w} x {h}"),
        _ => String::new(),
    };
    let hwdec = match text("hwdec-current") {
        current if current.is_empty() || current == "no" => "no".to_owned(),
        current => current,
    };

    vec![
        head(&[
            ("Stream", channel.to_owned()),
            ("Provider", provider.to_owned()),
        ]),
        line(&[
            ("Format", text("file-format")),
            ("Cache", cache),
            (
                "Speed",
                int("cache-speed")
                    .map(|b| format!("{}/s", bytes_text(b)))
                    .unwrap_or_default(),
            ),
        ]),
        head(&[
            ("Display", "libmpv export, wgpu".to_owned()),
            ("Hardware decoding", hwdec),
        ]),
        line(&[
            (
                "A-V",
                float("avsync")
                    .map(|s| format!("{s:+.3}"))
                    .unwrap_or_default(),
            ),
            (
                "Dropped frames",
                format!(
                    "{} (decoder)  {} (output)",
                    int("decoder-frame-drop-count").unwrap_or(0),
                    int("frame-drop-count").unwrap_or(0)
                ),
            ),
        ]),
        head(&[("Video", text("video-codec"))]),
        line(&[
            (
                "Frame rate",
                float("container-fps")
                    .map(|f| format!("{f:.3} fps").replace(".000", ""))
                    .unwrap_or_default(),
            ),
            ("Resolution", resolution),
        ]),
        line(&[
            (
                "Format",
                pixel_format(
                    &text("video-params/pixelformat"),
                    &text("video-params/hw-pixelformat"),
                ),
            ),
            ("Levels", text("video-params/levels")),
        ]),
        line(&[
            ("Colormatrix", text("video-params/colormatrix")),
            ("Primaries", text("video-params/primaries")),
            ("Transfer", text("video-params/gamma")),
        ]),
        line(&[(
            "Bitrate",
            float("video-bitrate").map(bitrate_text).unwrap_or_default(),
        )]),
        head(&[("Audio", text("audio-codec")), ("AO", text("current-ao"))]),
        line(&[
            (
                "Channels",
                int("audio-params/channel-count")
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            ),
            ("Format", text("audio-params/format")),
            (
                "Sample rate",
                int("audio-params/samplerate")
                    .map(|r| format!("{r} Hz"))
                    .unwrap_or_default(),
            ),
        ]),
        line(&[
            (
                "Bitrate",
                float("audio-bitrate").map(bitrate_text).unwrap_or_default(),
            ),
            (
                "Volume",
                engine
                    .volume()
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or_default(),
            ),
        ]),
    ]
    .into_iter()
    .filter(|l| l.pairs.row_count() > 0)
    .collect()
}

fn head(pairs: &[(&str, String)]) -> InfoLine {
    info_line(false, pairs)
}

fn line(pairs: &[(&str, String)]) -> InfoLine {
    info_line(true, pairs)
}

/// One overlay line; pairs without a value are left out.
fn info_line(indent: bool, pairs: &[(&str, String)]) -> InfoLine {
    let pairs: Vec<InfoPair> = pairs
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| InfoPair {
            label: SharedString::from(*label),
            value: value.as_str().into(),
        })
        .collect();
    InfoLine {
        indent,
        pairs: ModelRc::new(VecModel::from(pairs)),
    }
}

/// The pixel format, with the hardware surface type when decoding on the
/// GPU: `p010 (vaapi)`.
fn pixel_format(format: &str, hw_format: &str) -> String {
    if hw_format.is_empty() {
        format.to_owned()
    } else {
        format!("{hw_format} ({format})")
    }
}

/// `total-bytes` from a `demuxer-cache-state` node.
fn total_bytes(json: &str) -> Option<i64> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .get("total-bytes")?
        .as_i64()
}

/// `948224` as `926.00 KiB`.
fn bytes_text(bytes: i64) -> String {
    let bytes = bytes.max(0) as f64;
    match bytes {
        b if b >= 1024.0 * 1024.0 * 1024.0 => format!("{:.2} GiB", b / (1024.0 * 1024.0 * 1024.0)),
        b if b >= 1024.0 * 1024.0 => format!("{:.2} MiB", b / (1024.0 * 1024.0)),
        b if b >= 1024.0 => format!("{:.2} KiB", b / 1024.0),
        b => format!("{b:.0} B"),
    }
}

/// Bits per second as `11.889 Mbps` or `194 kbps`.
fn bitrate_text(bits: f64) -> String {
    if bits >= 1_000_000.0 {
        format!("{:.3} Mbps", bits / 1_000_000.0)
    } else {
        format!("{:.0} kbps", bits / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_use_binary_units() {
        assert_eq!(bytes_text(512), "512 B");
        assert_eq!(bytes_text(948_224), "926.00 KiB");
        assert_eq!(bytes_text(132_915_200), "126.76 MiB");
        assert_eq!(bytes_text(-1), "0 B");
    }

    #[test]
    fn bitrates_switch_units_at_a_megabit() {
        assert_eq!(bitrate_text(11_889_000.0), "11.889 Mbps");
        assert_eq!(bitrate_text(194_000.0), "194 kbps");
    }

    #[test]
    fn total_bytes_reads_the_cache_node() {
        assert_eq!(
            total_bytes(r#"{"total-bytes":4096,"cache-end":3.0}"#),
            Some(4096)
        );
        assert_eq!(total_bytes("nope"), None);
    }

    #[test]
    fn pixel_format_names_the_hardware_surface() {
        assert_eq!(pixel_format("vaapi", "p010"), "p010 (vaapi)");
        assert_eq!(pixel_format("yuv420p", ""), "yuv420p");
    }

    #[test]
    fn empty_values_leave_the_line() {
        let line = info_line(
            true,
            &[("Format", String::new()), ("Levels", "limited".into())],
        );
        assert_eq!(line.pairs.row_count(), 1);
        assert_eq!(line.pairs.row_data(0).unwrap().label, "Levels");
    }
}
