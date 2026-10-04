//! Channel logos: downloaded in the background on demand, cached on disk
//! under hashed names, and loaded into Slint images on the UI thread.
//!
//! Only rows the user can see ask for their logo, so a provider with tens
//! of thousands of channels never downloads them all.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use slint::Image;

/// Parallel downloads.
const WORKERS: usize = 4;
/// Per-logo timeout.
const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest logo accepted, in bytes.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Extensions kept from the URL, so Slint picks the right decoder.
const EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "svg", "webp", "gif"];

/// Logos known so far, and the download queue.
pub struct Logos {
    dir: Utf8PathBuf,
    queue: Sender<String>,
    loaded: HashMap<String, Option<Image>>,
    pending: HashSet<String>,
}

// Public API
impl Logos {
    /// Starts the download workers; `done` runs on a worker thread with the
    /// URL and the cached file, or `None` when the download failed.
    pub fn start(
        dir: Utf8PathBuf,
        done: impl Fn(String, Option<Utf8PathBuf>) + Send + Sync + 'static,
    ) -> Self {
        let (queue, requests) = mpsc::channel::<String>();
        let requests = Arc::new(Mutex::new(requests));
        let done = Arc::new(done);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        for _ in 0..WORKERS {
            let (requests, done, agent, dir) = (
                Arc::clone(&requests),
                Arc::clone(&done),
                agent.clone(),
                dir.clone(),
            );
            thread::spawn(move || work(&requests, &agent, &dir, &*done));
        }
        Self {
            dir,
            queue,
            loaded: HashMap::new(),
            pending: HashSet::new(),
        }
    }

    /// The logo for `url`, if it is loaded.
    pub fn get(&self, url: &str) -> Option<Image> {
        self.loaded.get(url).cloned().flatten()
    }

    /// Queues `url` unless it is loaded, failed, or already queued. A logo
    /// already on disk loads at once and returns `true`.
    pub fn request(&mut self, url: &str) -> bool {
        if url.is_empty() || self.loaded.contains_key(url) || self.pending.contains(url) {
            return false;
        }
        let path = cache_path(&self.dir, url);
        if path.exists() {
            self.finish(url.to_owned(), Some(path));
            return true;
        }
        self.pending.insert(url.to_owned());
        // Err only once every worker is gone; the logo stays a tile.
        let _ = self.queue.send(url.to_owned());
        false
    }

    /// Records a finished download; returns the image when it loaded.
    pub fn finish(&mut self, url: String, path: Option<Utf8PathBuf>) -> Option<Image> {
        self.pending.remove(&url);
        let image = path.and_then(|p| Image::load_from_path(p.as_std_path()).ok());
        self.loaded.insert(url, image.clone());
        image
    }
}

fn work(
    requests: &Mutex<Receiver<String>>,
    agent: &ureq::Agent,
    dir: &Utf8Path,
    done: &(dyn Fn(String, Option<Utf8PathBuf>) + Send + Sync),
) {
    loop {
        // Hold the lock only to take one request.
        let Ok(url) = requests.lock().expect("no worker panics holding it").recv() else {
            return;
        };
        let path = download(agent, dir, &url, MAX_BYTES);
        done(url, path);
    }
}

/// Downloads `url` into the cache, reading at most `max_bytes`; `None` on
/// any failure.
pub(crate) fn download(
    agent: &ureq::Agent,
    dir: &Utf8Path,
    url: &str,
    max_bytes: u64,
) -> Option<Utf8PathBuf> {
    let mut response = agent.get(url).call().ok()?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(max_bytes)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() {
        return None;
    }
    fs::create_dir_all(dir).ok()?;
    let path = cache_path(dir, url);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, &bytes).ok()?;
    fs::rename(&tmp, &path).ok()?;
    Some(path)
}

/// `dir/<hash of url>.<extension from url>`; `png` when the URL has none.
pub(crate) fn cache_path(dir: &Utf8Path, url: &str) -> Utf8PathBuf {
    // FNV-1a: stable across runs, unlike the std hasher.
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let extension = path
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .filter(|ext| EXTENSIONS.contains(&ext.as_str()))
        .unwrap_or_else(|| "png".to_owned());
    dir.join(format!("{hash:016x}.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_keeps_known_extensions() {
        let dir = Utf8Path::new("/cache/logos");
        let svg = cache_path(dir, "http://x.tv/logo/1.SVG?size=big");
        assert_eq!(svg.extension(), Some("svg"));
        assert_eq!(svg.parent(), Some(dir));
        assert_eq!(cache_path(dir, "http://x.tv/logo").extension(), Some("png"));
        assert_eq!(
            cache_path(dir, "http://x.tv/a.php?id=3").extension(),
            Some("png")
        );
    }

    #[test]
    fn cache_path_is_stable_and_distinct() {
        let dir = Utf8Path::new("/c");
        assert_eq!(
            cache_path(dir, "http://a/1.png"),
            cache_path(dir, "http://a/1.png")
        );
        assert_ne!(
            cache_path(dir, "http://a/1.png"),
            cache_path(dir, "http://a/2.png")
        );
    }
}
