//! An app played without a window: its frames drawn by egui's renderer
//! (and the app's own paint callbacks) into a texture on a GPU without a
//! surface, read back where it asks for a screenshot, until it closes. The
//! pictures have the same size everywhere, and each frame is made as fast
//! as the GPU makes it: a [`crate::Player`]'s clock is the frames', so its
//! video keeps the plan's pace.

use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use eframe::egui::{self, ViewportCommand, ViewportId};
use eframe::egui_wgpu::{self, wgpu};

/// What it draws into, as a window's on macOS: egui paints gamma-encoded.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The window played in.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    /// Its size in points.
    pub points: [f32; 2],
    /// Pixels per point.
    pub scale: f32,
    /// A play that never closes the app stops after so many frames.
    pub most_frames: u64,
}

impl Default for Window {
    /// 1920 × 1080 pixels of a 1280 × 720 window; ten minutes at 60.
    fn default() -> Window {
        Window {
            points: [1280.0, 720.0],
            scale: 1.5,
            most_frames: 36_000,
        }
    }
}

struct Nothing;
impl Wake for Nothing {
    fn wake(self: Arc<Self>) {}
}

/// A future of wgpu's run to its end: on native they are ready at once
/// or after the device is polled.
fn block_on<F: Future>(f: F, device: Option<&wgpu::Device>) -> F::Output {
    let waker = Waker::from(Arc::new(Nothing));
    let mut cx = Context::from_waker(&waker);
    let mut f = std::pin::pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        if let Some(d) = device {
            let _ = d.poll(wgpu::PollType::wait_indefinitely());
        } else {
            std::thread::yield_now();
        }
    }
}

/// The GPU and what draws on it.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    egui: egui_wgpu::Renderer,
    target: wgpu::Texture,
    size: [u32; 2],
}

impl Gpu {
    fn new(
        size: [u32; 2],
        setup: impl FnOnce(&wgpu::Adapter, &wgpu::Device, wgpu::TextureFormat, &mut egui_wgpu::Renderer),
    ) -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }),
            None,
        )
        .map_err(|e| format!("no GPU: {e}"))?;
        let (device, queue) = block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default()),
            None,
        )
        .map_err(|e| format!("the GPU: {e}"))?;
        let mut egui =
            egui_wgpu::Renderer::new(&device, FORMAT, egui_wgpu::RendererOptions::default());
        setup(&adapter, &device, FORMAT, &mut egui);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("opensolve_headless"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Gpu {
            device,
            queue,
            egui,
            target,
            size,
        })
    }

    /// A frame's output drawn into the target.
    fn draw(&mut self, ctx: &egui::Context, out: egui::FullOutput) {
        for (id, deltas) in &out.textures_delta.set {
            for delta in deltas {
                self.egui
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let jobs = ctx.tessellate(out.shapes, out.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: out.pixels_per_point,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut commands =
            self.egui
                .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        let view = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("opensolve_headless"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.egui
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        commands.push(encoder.finish());
        self.queue.submit(commands);
        for id in &out.textures_delta.free {
            self.egui.free_texture(id);
        }
    }

    /// The target's pixels.
    fn read(&self) -> egui::ColorImage {
        let [w, h] = self.size;
        // Rows padded to what a copy needs.
        let row = (w * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("opensolve_headless_read"),
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = rx.recv().ok().and_then(Result::ok).is_some();
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        if mapped && let Ok(data) = slice.get_mapped_range() {
            for y in 0..h as usize {
                let start = y * row as usize;
                rgba.extend_from_slice(&data[start..start + (w * 4) as usize]);
            }
        }
        rgba.resize((w * h * 4) as usize, 255);
        buffer.unmap();
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)
    }
}

/// Runs the app `make` makes until it closes its viewport. `setup` puts
/// the app's paint callbacks' resources into egui's renderer (as eframe's
/// `wgpu_render_state` would offer them).
///
/// # Errors
///
/// No GPU, or the app did not close within `window.most_frames`.
pub fn run<A: eframe::App>(
    window: &Window,
    setup: impl FnOnce(&wgpu::Adapter, &wgpu::Device, wgpu::TextureFormat, &mut egui_wgpu::Renderer),
    make: impl FnOnce(&eframe::CreationContext<'_>) -> A,
) -> Result<(), String> {
    let size = [
        (window.points[0] * window.scale).round() as u32,
        (window.points[1] * window.scale).round() as u32,
    ];
    let mut gpu = Gpu::new(size, setup)?;
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = make(&cc);
    let mut frame = eframe::Frame::_new_kittest();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, window.points.into());
    for n in 0..window.most_frames {
        let mut raw = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(n as f64 / 60.0),
            focused: true,
            ..Default::default()
        };
        let root = raw.viewports.entry(ViewportId::ROOT).or_default();
        root.native_pixels_per_point = Some(window.scale);
        root.inner_rect = Some(screen);
        root.focused = Some(true);
        app.raw_input_hook(&ctx, &mut raw);
        let mut out = ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
        let commands = out
            .viewport_output
            .remove(&ViewportId::ROOT)
            .map(|v| v.commands)
            .unwrap_or_default();
        gpu.draw(&ctx, out);
        let mut image = None;
        for c in commands {
            match c {
                ViewportCommand::Screenshot(callback) => {
                    let pixels = image.get_or_insert_with(|| Arc::new(gpu.read()));
                    callback.complete(pixels.clone());
                }
                ViewportCommand::Close => {
                    app.on_exit();
                    return Ok(());
                }
                _ => {}
            }
        }
    }
    Err(format!(
        "the play did not end in {} frames",
        window.most_frames
    ))
}
