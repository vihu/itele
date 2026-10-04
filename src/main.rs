//! itele: a native IPTV player.
//!
//! Opens the saved Xtream provider (or the login screen), lists its live
//! channels, and plays them through libmpv with zero-copy frames.

#![deny(unsafe_code, missing_docs, rustdoc::broken_intra_doc_links)]

mod art;
mod info;
mod live;
mod logos;
mod names;
mod playback;
mod session;
mod tracks;
mod video;
mod vod;

/// The compiled Slint UI from `ui/`.
mod ui {
    slint::include_modules!();
}

use std::sync::Arc;

use itele::provider::Paths;
use mpv_engine::Engine;
use slint::ComponentHandle;
use slint::wgpu_30::{WGPUConfiguration, WGPUSettings};

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// The application id: the desktop entry, the icon and the Flatpak.
const APP_ID: &str = "io.github.vihu.itele";
/// ffmpeg options that reconnect a live HTTP stream after a network drop.
const RECONNECT: &str = "reconnect=1,reconnect_streamed=1,reconnect_delay_max=5";
/// Stream data mpv may buffer ahead of the playhead.
const CACHE_AHEAD: &str = "512MiB";
/// Stream data mpv keeps behind the playhead, for pause and rewind.
const CACHE_BEHIND: &str = "512MiB";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut settings = WGPUSettings::default();
    settings.backends = mpv_engine::WGPU_BACKEND;
    settings.device_required_features = mpv_engine::REQUIRED_WGPU_FEATURES;
    slint::BackendSelector::new()
        .require_wgpu_30(WGPUConfiguration::Automatic(settings))
        .select()?;
    // Matches the desktop entry, so docks show itele's icon for the window.
    slint::set_xdg_app_id(APP_ID)?;

    let app = ui::AppWindow::new()?;
    let mut mpv = Engine::video()
        // A dropped live stream should end, not freeze on its last frame.
        .property("keep-open", "no")
        .property("stream-lavf-o", RECONNECT)
        // Keep a window of the live stream to pause and rewind within.
        .property("cache", "yes")
        .property("demuxer-seekable-cache", "yes")
        .property("demuxer-max-bytes", CACHE_AHEAD)
        .property("demuxer-max-back-bytes", CACHE_BEHIND);
    // Without NVIDIA's driver, loading the CUDA interop only prints
    // "Cannot load libcuda.so.1"; VA-API is the path on AMD and Intel.
    if cfg!(target_os = "linux") && !std::path::Path::new("/proc/driver/nvidia/version").exists() {
        mpv = mpv.property("gpu-hwdec-interop", "vaapi");
    }
    let engine = Arc::new(mpv.build()?);
    video::install(&app, &engine)?;
    session::start(&app, Arc::clone(&engine), Paths::system()?);

    app.run()?;
    session::finish();
    engine.detach_render();
    Ok(())
}
