//! itele: a native IPTV player.
//!
//! M0 spike: plays one stream URL in a Slint window. mpv renders on its own
//! thread into exported GPU frames (DMA-BUF on Linux, IOSurface on macOS),
//! which are imported into Slint's wgpu device and blitted to RGBA on the
//! GPU, with no CPU copy.

#![deny(unsafe_code, missing_docs, rustdoc::broken_intra_doc_links)]

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use mpv_engine::{Engine, ExportOptions, PlaybackEvent};
use slint::wgpu_30::{WGPUConfiguration, WGPUSettings};
use slint::{ComponentHandle, GraphicsAPI, Image, RenderingState};
use ui::Player;
use wgpu::util::TextureBlitter;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// Frame size until the window reports its real size on first render.
const INITIAL_EXPORT_SIZE: (u32, u32) = (1280, 720);
/// How often playback stats go to stderr.
const STATS_INTERVAL: Duration = Duration::from_secs(1);
/// The only texture format Slint imports from wgpu.
const SLINT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The Slint UI: one window showing the current video frame.
mod ui {
    slint::slint! {
        export component Player inherits Window {
            title: "itele";
            preferred-width: 1280px;
            preferred-height: 720px;
            background: black;
            in property <image> frame;
            Image {
                width: 100%;
                height: 100%;
                source: root.frame;
                // mpv letterboxes into the export size, which tracks the
                // window.
                image-fit: fill;
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args().nth(1).ok_or("usage: itele <stream-url>")?;

    let mut settings = WGPUSettings::default();
    settings.backends = mpv_engine::WGPU_BACKEND;
    settings.device_required_features = mpv_engine::REQUIRED_WGPU_FEATURES;
    slint::BackendSelector::new()
        .require_wgpu_30(WGPUConfiguration::Automatic(settings))
        .select()?;

    let player = Player::new()?;
    let engine = Arc::new(Engine::video().build()?);

    let redraw = player.as_weak();
    let (width, height) = INITIAL_EXPORT_SIZE;
    engine.attach_exported_render(ExportOptions::new(width, height), move || {
        // Err only once the event loop has quit.
        let _ = redraw.upgrade_in_event_loop(|player| player.window().request_redraw());
    })?;
    engine.set_wakeup_callback(drain_events(Arc::downgrade(&engine)));
    engine.load_when_ready(&url)?;

    let mut video = None;
    let frames = player.as_weak();
    let render_engine = Arc::clone(&engine);
    player
        .window()
        .set_rendering_notifier(move |state, api| match (state, api) {
            (RenderingState::RenderingSetup, GraphicsAPI::WGPU30 { device, queue, .. }) => {
                video = Some(Video::new(device.clone(), queue.clone()));
            }
            (RenderingState::BeforeRendering, _) => {
                if let (Some(video), Some(player)) = (video.as_mut(), frames.upgrade()) {
                    video.show_next_frame(&render_engine, &player);
                }
            }
            (RenderingState::RenderingTeardown, _) => video = None,
            _ => {}
        })?;

    player.run()?;
    engine.detach_render();
    Ok(())
}

/// Builds mpv's wakeup callback, which drains playback events on the UI
/// thread.
///
/// mpv forbids client API calls inside the wakeup callback itself.
fn drain_events(engine: Weak<Engine>) -> impl Fn() + Send + Sync + 'static {
    move || {
        let engine = engine.clone();
        // Err only once the event loop has quit.
        let _ = slint::invoke_from_event_loop(move || {
            let Some(engine) = engine.upgrade() else {
                return;
            };
            for event in engine.pump_events() {
                match event {
                    PlaybackEvent::Failed { message, .. } => {
                        eprintln!("playback failed: {message}");
                    }
                    PlaybackEvent::Ended { reason } => eprintln!("playback ended: {reason:?}"),
                    _ => {}
                }
            }
        });
    }
}

/// Moves mpv's exported frames into a texture Slint can display.
struct Video {
    device: wgpu::Device,
    queue: wgpu::Queue,
    blitter: TextureBlitter,
    target: Option<wgpu::Texture>,
    export_size: (u32, u32),
    stats_since: Instant,
    stats_frames: u32,
}

impl Video {
    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let blitter = TextureBlitter::new(&device, SLINT_FORMAT);
        Self {
            device,
            queue,
            blitter,
            target: None,
            export_size: INITIAL_EXPORT_SIZE,
            stats_since: Instant::now(),
            stats_frames: 0,
        }
    }

    /// Shows the newest mpv frame, if one was published since the last call.
    fn show_next_frame(&mut self, engine: &Engine, player: &Player) {
        let size = player.window().size();
        let size = (size.width, size.height);
        if size != self.export_size {
            self.export_size = size;
            if let Err(e) = engine.set_export_size(size.0, size.1) {
                eprintln!("resize video: {e}");
            }
        }

        let frame = match engine.acquire_frame() {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(e) => return eprintln!("acquire frame: {e}"),
        };
        let source = match frame.into_wgpu_texture(&self.device) {
            Ok(texture) => texture,
            Err(e) => return eprintln!("import frame: {e}"),
        };

        // mpv exports BGRA; Slint only takes RGBA, so convert on the GPU.
        let target = self.target_sized(source.width(), source.height());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.blitter.copy(
            &self.device,
            &mut encoder,
            &source.create_view(&Default::default()),
            &target.create_view(&Default::default()),
        );
        self.queue.submit([encoder.finish()]);

        match Image::try_from(target) {
            Ok(image) => player.set_frame(image),
            Err(e) => return eprintln!("hand frame to slint: {e}"),
        }
        // The target texture is reused, so the property may not change:
        // redraw explicitly. Bounded: the next pass finds no new frame.
        player.window().request_redraw();
        self.count_frame(engine);
    }

    /// Returns the RGBA target, reallocated when the frame size changes.
    fn target_sized(&mut self, width: u32, height: u32) -> wgpu::Texture {
        if let Some(target) = &self.target
            && target.width() == width
            && target.height() == height
        {
            return target.clone();
        }
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("itele video frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SLINT_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.target = Some(target.clone());
        target
    }

    /// Prints displayed fps, the active hardware decoder, and mpv's drop
    /// count once per [`STATS_INTERVAL`].
    fn count_frame(&mut self, engine: &Engine) {
        self.stats_frames += 1;
        let elapsed = self.stats_since.elapsed();
        if elapsed < STATS_INTERVAL {
            return;
        }
        let hwdec = engine
            .get_property::<String>("hwdec-current")
            .unwrap_or_else(|_| "none".into());
        let dropped = engine.get_property::<i64>("frame-drop-count").unwrap_or(0);
        let fps = f64::from(self.stats_frames) / elapsed.as_secs_f64();
        eprintln!("fps {fps:.1}  hwdec {hwdec}  dropped {dropped}");
        self.stats_since = Instant::now();
        self.stats_frames = 0;
    }
}
