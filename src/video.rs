//! Moves mpv's exported frames onto the screen.
//!
//! mpv renders on its own thread into exported GPU frames (DMA-BUF on
//! Linux, IOSurface on macOS). Each frame is imported into Slint's wgpu
//! device and blitted to RGBA on the GPU, with no CPU copy. mpv renders at
//! the size of the visible video area, so the preview gets small frames and
//! fullscreen gets full ones.

use std::sync::Arc;

use mpv_engine::{Engine, ExportOptions};
use slint::{ComponentHandle, GraphicsAPI, Image, RenderingState, SetRenderingNotifierError};
use wgpu::util::TextureBlitter;

use crate::ui::AppWindow;

/// Frame size until the video area reports its real size.
const INITIAL_SIZE: (u32, u32) = (1280, 720);
/// The only texture format Slint imports from wgpu.
const SLINT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Attaches mpv's renderer and shows its frames in `app`.
///
/// # Errors
///
/// Fails when mpv cannot create its hidden GPU context or Slint refuses
/// the rendering notifier.
pub fn install(app: &AppWindow, engine: &Arc<Engine>) -> Result<(), Box<dyn std::error::Error>> {
    let redraw = app.as_weak();
    let (width, height) = INITIAL_SIZE;
    engine.attach_exported_render(ExportOptions::new(width, height), move || {
        // Err only once the event loop has quit.
        let _ = redraw.upgrade_in_event_loop(|app| app.window().request_redraw());
    })?;

    let mut video = None;
    let weak = app.as_weak();
    let engine = Arc::clone(engine);
    app.window()
        .set_rendering_notifier(move |state, api| match (state, api) {
            (RenderingState::RenderingSetup, GraphicsAPI::WGPU30 { device, queue, .. }) => {
                video = Some(Video::new(device.clone(), queue.clone()));
            }
            (RenderingState::BeforeRendering, _) => {
                if let (Some(video), Some(app)) = (video.as_mut(), weak.upgrade()) {
                    video.show_next_frame(&engine, &app);
                }
            }
            (RenderingState::RenderingTeardown, _) => video = None,
            _ => {}
        })
        .map_err(|e: SetRenderingNotifierError| format!("rendering notifier: {e:?}"))?;
    Ok(())
}

struct Video {
    device: wgpu::Device,
    queue: wgpu::Queue,
    blitter: TextureBlitter,
    target: Option<wgpu::Texture>,
    export_size: (u32, u32),
}

impl Video {
    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let blitter = TextureBlitter::new(&device, SLINT_FORMAT);
        Self {
            device,
            queue,
            blitter,
            target: None,
            export_size: INITIAL_SIZE,
        }
    }

    /// Shows the newest mpv frame, if one was published since the last call.
    fn show_next_frame(&mut self, engine: &Engine, app: &AppWindow) {
        let scale = app.window().scale_factor();
        let size = (
            (app.get_video_width() * scale).round() as u32,
            (app.get_video_height() * scale).round() as u32,
        );
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
            Ok(image) => app.set_frame(image),
            Err(e) => return eprintln!("hand frame to slint: {e}"),
        }
        // The target texture is reused, so the property may not change:
        // redraw explicitly. Bounded: the next pass finds no new frame.
        app.window().request_redraw();
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
}
