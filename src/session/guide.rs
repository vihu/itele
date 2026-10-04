//! The programme guide's side of a provider refresh: importing XMLTV in the
//! background, and recording how it went.

use std::collections::HashSet;
use std::io::BufReader;

use camino::Utf8Path;
use itele::epg::Store;
use itele::provider::Library;
use itele::xtream::Client;

use super::Session;
use crate::live::thousands;

/// A guide younger than this is not downloaded again.
const MAX_AGE: i64 = 12 * 3600;
/// Programmes kept before now: a week of catch-up, plus a day.
const KEEP_BEHIND: i64 = 8 * 86_400;
/// Programmes kept after now.
const KEEP_AHEAD: i64 = 8 * 86_400;

/// The XMLTV ids of `library`'s channels, lowercased.
pub(super) fn wanted_channels(library: &Library) -> HashSet<String> {
    library
        .streams
        .iter()
        .filter_map(|s| s.epg_channel_id.as_deref())
        .map(str::to_lowercase)
        .collect()
}

/// Imports `provider`'s guide into the store at `path` unless it is fresh.
/// Runs on a worker thread. `Ok(None)` means the stored guide was fresh.
pub(super) fn refresh(
    client: &Client,
    path: &Utf8Path,
    provider: &str,
    wanted: &HashSet<String>,
) -> Result<Option<usize>, String> {
    let mut store = Store::open(path).map_err(|e| e.to_string())?;
    let now = jiff::Timestamp::now().as_second();
    let imported = store.imported_at(provider).map_err(|e| e.to_string())?;
    if imported.is_some_and(|at| now - at < MAX_AGE) {
        return Ok(None);
    }
    let xml = client.xmltv().map_err(|e| e.to_string())?;
    let keep = now - KEEP_BEHIND..now + KEEP_AHEAD;
    store
        .import(provider, wanted, keep, BufReader::new(xml), now)
        .map(Some)
        .map_err(|e| e.to_string())
}

impl Session {
    /// Records how `provider`'s guide refresh went.
    pub(super) fn guide_ready(&self, id: &str, epoch: u64, result: Result<Option<usize>, String>) {
        {
            let mut state = self.state.borrow_mut();
            let Some(slot) = state
                .slots
                .iter_mut()
                .find(|s| s.provider.id == id && s.epoch == epoch)
            else {
                return;
            };
            match result {
                Ok(Some(count)) => slot.guide = format!("guide: {} programmes", thousands(count)),
                Ok(None) => {}
                Err(e) => slot.guide = format!("guide failed: {e}"),
            }
        }
        self.refresh_lists();
    }
}
