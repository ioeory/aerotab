//! In-process DX12 swapchain consuming the existing canonical engine snapshot.
use super::{HostError, Snapshot, ViewConfig};
use glyphon::{
    Attrs, Buffer, Cache, Color, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use std::num::NonZeroIsize;
use wgpu::util::DeviceExt;

#[cfg(debug_assertions)]
pub(super) struct PerfTrace {
    start: std::time::Instant,
    last: std::time::Instant,
    stages: Vec<(&'static str, f64)>,
    seq: u64,
    kind: &'static str,
    hits: u64,
    misses: u64,
}
#[cfg(debug_assertions)]
impl PerfTrace {
    pub(super) fn begin(kind: &'static str, seq: u64) -> Option<Self> {
        std::env::var_os("AEROTAB_WINDOWS_PERF_LOG")?;
        let now = std::time::Instant::now();
        Some(Self {
            start: now,
            last: now,
            stages: Vec::new(),
            seq,
            kind,
            hits: 0,
            misses: 0,
        })
    }
    pub(super) fn lap(trace: &mut Option<Self>, name: &'static str) {
        if let Some(t) = trace {
            let now = std::time::Instant::now();
            t.stages
                .push((name, now.duration_since(t.last).as_secs_f64() * 1000.));
            t.last = now;
        }
    }
}
#[cfg(debug_assertions)]
impl Drop for PerfTrace {
    fn drop(&mut self) {
        use std::io::Write;
        static LOG: std::sync::OnceLock<
            std::sync::Mutex<Option<std::io::BufWriter<std::fs::File>>>,
        > = std::sync::OnceLock::new();
        let log = LOG.get_or_init(|| {
            std::sync::Mutex::new(
                std::env::var_os("AEROTAB_WINDOWS_PERF_LOG")
                    .and_then(|p| {
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(p)
                            .ok()
                    })
                    .map(std::io::BufWriter::new),
            )
        });
        if let Ok(mut guard) = log.lock() {
            if let Some(file) = guard.as_mut() {
                let row = serde_json::json!({"kind":self.kind,"seq":self.seq,"cacheHits":self.hits,"cacheMisses":self.misses,"totalMs":self.start.elapsed().as_secs_f64()*1000.,"stages":self.stages,"unixMs":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()});
                let _ = writeln!(file, "{row}");
                let _ = file.flush();
            }
        }
    }
}

pub(super) struct Renderer {
    // Surface must drop before its owner HWND is destroyed.
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    fonts: FontSystem,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text: TextRenderer,
    rectangles: wgpu::RenderPipeline,
    glyphs: super::windows_glyph_cache::Slots<Buffer>,
}
fn failure(e: impl std::fmt::Display) -> HostError {
    HostError::new("dx12_surface", e.to_string())
}
impl Renderer {
    /// HWND must remain valid on the owning main thread until this renderer drops.
    pub fn new(hwnd: isize, width: u32, height: u32) -> Result<Self, HostError> {
        pollster::block_on(Self::create(hwnd, width, height))
    }
    async fn create(hwnd: isize, width: u32, height: u32) -> Result<Self, HostError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::DX12,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::Win32(Win32WindowHandle::new(
                    NonZeroIsize::new(hwnd).ok_or_else(|| failure("null HWND"))?,
                )),
            })
        }
        .map_err(failure)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await
            .map_err(failure)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .map_err(failure)?;
        let config = surface
            .get_default_config(&adapter, width.max(1), height.max(1))
            .ok_or_else(|| failure("no surface configuration"))?;
        surface.configure(&device, &config);
        let fonts = FontSystem::new();
        if fonts.db().faces().next().is_none() {
            return Err(failure("no system fonts"));
        }
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let text = TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("native terminal cells"), source: wgpu::ShaderSource::Wgsl("struct Out { @builtin(position) p: vec4<f32>, @location(0) c: vec3<f32> }; @vertex fn vs(@location(0) p: vec2<f32>, @location(1) c: vec3<f32>) -> Out { var o: Out; o.p = vec4<f32>(p, 0.0, 1.0); o.c = c; return o; } @fragment fn fs(o: Out) -> @location(0) vec4<f32> { return vec4<f32>(o.c, 1.0); }".into()) });
        let rectangles = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("native terminal rectangles"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3],
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            surface,
            device,
            queue,
            config,
            fonts,
            swash: SwashCache::new(),
            viewport,
            atlas,
            text,
            rectangles,
            glyphs: super::windows_glyph_cache::Slots::new(),
        })
    }
    pub fn metrics(&mut self, cfg: &ViewConfig) -> (u16, u16) {
        let family = super::font_family::resolve_font_family(&self.fonts, &cfg.font_family);
        let mut b = Buffer::new(
            &mut self.fonts,
            Metrics::new(
                cfg.font_size as f32,
                (cfg.font_size + cfg.line_padding) as f32,
            ),
        );
        let attrs = Attrs::new().family(family.as_glyphon());
        b.set_text("M", &attrs, Shaping::Advanced, None);
        b.shape_until_scroll(&mut self.fonts, false);
        let width = b
            .layout_runs()
            .next()
            .map_or(cfg.font_size as f32 * 0.6, |r| r.line_w);
        (
            width.ceil().max(1.) as u16,
            (cfg.font_size + cfg.line_padding).ceil() as u16,
        )
    }
    /// Returns true only after an actual surface acquisition, submission and GPU completion.
    pub fn draw(
        &mut self,
        snapshot: &Snapshot,
        cfg: &ViewConfig,
        size: [u32; 2],
        scale: f64,
        cell: (u16, u16),
    ) -> Result<bool, HostError> {
        #[cfg(debug_assertions)]
        let mut perf = PerfTrace::begin("renderer", u64::from(snapshot.seq));
        if self.config.width != size[0] || self.config.height != size[1] {
            self.config.width = size[0].max(1);
            self.config.height = size[1].max(1);
            self.surface.configure(&self.device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false)
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            other => return Err(failure(format!("surface acquisition: {other:?}"))),
        };
        #[cfg(debug_assertions)]
        PerfTrace::lap(&mut perf, "acquire");
        let cw = cell.0 as f32 * scale as f32;
        let ch = cell.1 as f32 * scale as f32;
        let padding = 12. * scale as f32;
        let family = super::font_family::resolve_font_family(&self.fonts, &cfg.font_family);
        self.glyphs.resize(snapshot.cells.len());
        let mut hits = 0u64;
        let mut misses = 0u64;
        let mut buffers = Vec::new();
        let mut vertices = Vec::<u8>::new();
        let width = self.config.width as f32;
        let height = self.config.height as f32;
        let rect = |v: &mut Vec<u8>, x: f32, y: f32, w: f32, h: f32, c: [u8; 3]| {
            for (a, b) in [
                (x, y),
                (x + w, y),
                (x, y + h),
                (x, y + h),
                (x + w, y),
                (x + w, y + h),
            ] {
                for f in [
                    a / width * 2. - 1.,
                    1. - b / height * 2.,
                    c[0] as f32 / 255.,
                    c[1] as f32 / 255.,
                    c[2] as f32 / 255.,
                ] {
                    v.extend_from_slice(&f.to_ne_bytes());
                }
            }
        };
        for (index, c) in snapshot.cells.iter().enumerate() {
            if c.width == 0 {
                self.glyphs.clear_slot(index);
                continue;
            }
            let x = padding + (index % snapshot.cols) as f32 * cw;
            let y = padding + (index / snapshot.cols) as f32 * ch;
            rect(&mut vertices, x, y, cw * c.width as f32, ch, c.background);
            let color = if c.flags & 128 != 0 {
                c.foreground.map(|v| (v as f32 * 0.6) as u8)
            } else {
                c.foreground
            };
            if c.flags & 8 != 0 {
                rect(
                    &mut vertices,
                    x,
                    y + ch - 2.,
                    cw * c.width as f32,
                    scale as f32,
                    color,
                );
            }
            if c.flags & 512 != 0 {
                rect(
                    &mut vertices,
                    x,
                    y + ch / 2.,
                    cw * c.width as f32,
                    scale as f32,
                    color,
                );
            }
            if c.text != " " {
                let key = super::windows_glyph_cache::LayoutKey {
                    text: c.text.clone(),
                    family: cfg.font_family.clone(),
                    font_size: cfg.font_size.to_bits(),
                    scale: scale.to_bits(),
                    cell,
                    width: c.width,
                    weight: if c.flags & 2 != 0 {
                        700
                    } else {
                        cfg.font_weight
                    },
                    italic: c.flags & 4 != 0,
                    grid: (snapshot.cols, snapshot.cells.len() / snapshot.cols),
                    shaping_policy: 0,
                };
                let family = family.clone();
                let (b, hit) = self.glyphs.layout(index, key, || {
                    let mut b = Buffer::new(
                        &mut self.fonts,
                        Metrics::new(cfg.font_size as f32 * scale as f32, ch),
                    );
                    b.set_size(Some(cw * c.width as f32), Some(ch));
                    b.set_wrap(glyphon::Wrap::None);
                    let attrs = Attrs::new()
                        .family(family.as_glyphon())
                        .weight(glyphon::Weight(if c.flags & 2 != 0 {
                            700
                        } else {
                            cfg.font_weight
                        }))
                        .style(if c.flags & 4 != 0 {
                            glyphon::Style::Italic
                        } else {
                            glyphon::Style::Normal
                        });
                    b.set_text(&c.text, &attrs, Shaping::Advanced, None);
                    b.shape_until_scroll(&mut self.fonts, false);
                    b
                });
                if hit {
                    hits += 1;
                } else {
                    misses += 1;
                }
                buffers.push((b, x, y, cw * c.width as f32, color));
            } else {
                self.glyphs.clear_slot(index);
            }
        }
        if snapshot.cursor_visible {
            let x = padding + snapshot.cursor.0 as f32 * cw;
            let y = padding + snapshot.cursor.1 as f32 * ch;
            let (y, w, h) = match snapshot.cursor_shape.as_str() {
                "Beam" => (y, 2. * scale as f32, ch),
                "Block" => (y, cw, ch),
                _ => (y + ch - 2. * scale as f32, cw, 2. * scale as f32),
            };
            rect(&mut vertices, x, y, w, h, cfg.foreground);
        }
        #[cfg(debug_assertions)]
        if let Some(trace) = perf.as_mut() {
            trace.hits = hits;
            trace.misses = misses;
        }
        let _ = (hits, misses);
        #[cfg(debug_assertions)]
        PerfTrace::lap(&mut perf, "cells_buffers_shape");
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.text
            .prepare(
                &self.device,
                &self.queue,
                &mut self.fonts,
                &mut self.atlas,
                &self.viewport,
                buffers.iter().map(|(b, x, y, w, c)| TextArea {
                    buffer: b,
                    left: *x,
                    top: *y,
                    scale: 1.,
                    bounds: TextBounds {
                        left: *x as i32,
                        top: *y as i32,
                        right: (x + w) as i32,
                        bottom: (y + ch) as i32,
                    },
                    default_color: Color::rgb(c[0], c[1], c[2]),
                    custom_glyphs: &[],
                }),
                &mut self.swash,
            )
            .map_err(failure)?;
        #[cfg(debug_assertions)]
        PerfTrace::lap(&mut perf, "text_prepare");
        let vertex = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &vertices,
                usage: wgpu::BufferUsages::VERTEX,
            });
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let bg = cfg.background.map(|v| v as f64 / 255.);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg[0],
                            g: bg[1],
                            b: bg[2],
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.rectangles);
            pass.set_vertex_buffer(0, vertex.slice(..));
            pass.draw(0..(vertices.len() / 20) as u32, 0..1);
            self.text
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(failure)?;
        }
        #[cfg(debug_assertions)]
        PerfTrace::lap(&mut perf, "vertex_encode");
        let submission = self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(2)),
            })
            .map_err(failure)?;
        #[cfg(debug_assertions)]
        PerfTrace::lap(&mut perf, "submit_present_poll");
        self.atlas.trim();
        Ok(true)
    }
}
