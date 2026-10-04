//! Settings' About section: what plays the video, how much each of
//! itele's places on disk holds, and clearing the cache.

use std::fs;
use std::thread;

use camino::Utf8Path;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::refresh::Force;
use super::{Session, on_ui_thread};
use crate::ui::{SettingsData, StorageRow};

impl Session {
    /// Fills the About section's versions; once, at start.
    pub(super) fn fill_about(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let version = |property: &str| {
            self.engine
                .get_property::<String>(property)
                .unwrap_or_default()
        };
        // Reported as `mpv v0.41.0` and `n9.0.2`.
        let mpv = version("mpv-version");
        let mpv = mpv.trim_start_matches("mpv ").trim_start_matches('v');
        let ffmpeg = version("ffmpeg-version");
        let ffmpeg = ffmpeg.trim_start_matches('n');
        let data = app.global::<SettingsData>();
        data.set_player(format!("libmpv {mpv} with FFmpeg {ffmpeg}").into());
        let api = if cfg!(target_os = "macos") {
            "Metal"
        } else {
            "Vulkan"
        };
        data.set_video(format!("{api}, frames handed from mpv on the GPU").into());
    }

    /// Measures the settings, the watch history and the cache on a worker
    /// thread, then shows them.
    pub(super) fn measure_storage(&self) {
        let paths = self.paths.clone();
        thread::spawn(move || {
            let history = paths.history_path();
            let places = [
                ("Settings", paths.config_dir().to_owned(), None),
                ("Watch history", history.clone(), None),
                (
                    "Cache",
                    paths.cache_dir().to_owned(),
                    Some("lists, posters, guide"),
                ),
            ];
            let rows: Vec<(String, String, String)> = places
                .into_iter()
                .map(|(label, path, note)| {
                    let shown = match note {
                        Some(note) => format!("{} · {note}", home_relative(&path)),
                        None => home_relative(&path),
                    };
                    (label.to_owned(), shown, bytes(size_of(&path)))
                })
                .collect();
            on_ui_thread(move |s| s.storage_ready(rows));
        });
    }

    /// Clears the cache: empties the guide store, deletes the rest on a
    /// worker thread, then downloads everything again.
    pub(super) fn clear_cache(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.global::<SettingsData>().set_clearing(true);
        let ids: Vec<String> = self
            .state
            .borrow()
            .slots
            .iter()
            .map(|s| s.provider.id.clone())
            .collect();
        if let Some(store) = self.guide.borrow_mut().as_mut() {
            for id in &ids {
                if let Err(e) = store.remove(id) {
                    eprintln!("clear guide: {e}");
                }
            }
        }
        let paths = self.paths.clone();
        thread::spawn(move || {
            let result = paths.clear_cache().map_err(|e| e.to_string());
            on_ui_thread(move |s| s.cache_cleared(result));
        });
    }
}

// Private API
impl Session {
    fn storage_ready(&self, rows: Vec<(String, String, String)>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let rows: Vec<StorageRow> = rows
            .into_iter()
            .map(|(label, path, size)| StorageRow {
                label: label.into(),
                path: path.into(),
                size: size.into(),
            })
            .collect();
        app.global::<SettingsData>()
            .set_storage(ModelRc::new(VecModel::from(rows)));
    }

    fn cache_cleared(&self, result: Result<(), String>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.global::<SettingsData>().set_clearing(false);
        if let Err(e) = result {
            eprintln!("clear cache: {e}");
        }
        self.refresh_all(Force::Yes);
        self.measure_storage();
    }
}

/// The bytes under `path`, a file or a directory.
fn size_of(path: &Utf8Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| camino::Utf8PathBuf::from_path_buf(e.path()).ok())
                .map(|p| size_of(&p))
                .sum()
        })
        .unwrap_or(0)
}

/// `path` with the home directory written as `~`.
fn home_relative(path: &Utf8Path) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => match path.as_str().strip_prefix(&home) {
            Some(rest) => format!("~{rest}"),
            None => path.to_string(),
        },
        _ => path.to_string(),
    }
}

/// For example `4 KB`, `212 KB`, `1.3 GB`.
fn bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    let n = n as f64;
    if n < KB * KB {
        format!("{} KB", (n / KB).ceil() as u64)
    } else if n < KB * KB * KB {
        format!("{:.0} MB", n / (KB * KB))
    } else {
        format!("{:.1} GB", n / (KB * KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_read_naturally() {
        assert_eq!(bytes(0), "0 KB");
        assert_eq!(bytes(3000), "3 KB");
        assert_eq!(bytes(212 * 1024 * 1024), "212 MB");
        assert_eq!(bytes(1_400_000_000), "1.3 GB");
    }
}
