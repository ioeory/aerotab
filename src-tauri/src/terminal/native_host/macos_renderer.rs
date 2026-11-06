//! Metal-only in-process surface. Owns neither a parser nor a PTY.
use super::{HostError, Snapshot, ViewConfig};
use glyphon::{
    Attrs, Buffer, Cache, Color, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport,
};
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use std::{ffi::c_void, ptr::NonNull};
use wgpu::util::DeviceExt;
pub(super) struct Renderer {
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
    pub layout: super::macos_policy::PreeditLayout,
    composition_origin: (f64, f64),
}
fn failure(e: impl std::fmt::Display) -> HostError {
    HostError::new("metal_surface", e.to_string())
}
impl Renderer {
    /// SAFETY: the NSView must outlive this surface, and all calls run on its main thread.
    pub unsafe fn new(view: NonNull<c_void>, size: [u32; 2]) -> Result<Self, HostError> {
        pollster::block_on(Self::create(view, size))
    }
    async fn create(view: NonNull<c_void>, size: [u32; 2]) -> Result<Self, HostError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(RawDisplayHandle::AppKit(AppKitDisplayHandle::new())),
                raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(view)),
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
            .get_default_config(&adapter, size[0].max(1), size[1].max(1))
            .ok_or_else(|| failure("no Metal surface configuration"))?;
        surface.configure(&device, &config);
        let fonts = FontSystem::new();
        if fonts.db().faces().next().is_none() {
            return Err(failure("no system fonts"));
        }
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let text = TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("AppKit terminal cells"), source: wgpu::ShaderSource::Wgsl("struct Out { @builtin(position) p: vec4<f32>, @location(0) c: vec3<f32> }; @vertex fn vs(@location(0) p: vec2<f32>, @location(1) c: vec3<f32>) -> Out { var o: Out; o.p = vec4<f32>(p, 0.0, 1.0); o.c = c; return o; } @fragment fn fs(o: Out) -> @location(0) vec4<f32> { return vec4<f32>(o.c, 1.0); }".into()) });
        let rectangles = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("AppKit terminal rectangles"),
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
            layout: Default::default(),
            composition_origin: (0., 0.),
        })
    }
    /// One shaping path supplies drawing, candidate rectangles and screen hit tests.
    pub fn preedit_buffer(
        &mut self,
        preedit: &super::macos_policy::Preedit,
        cfg: &ViewConfig,
        cell: (u16, u16),
        cursor: (usize, usize),
        size: [f64; 2],
    ) -> Buffer {
        let text = &preedit.text;
        let cw = f64::from(cell.0);
        let ch = f64::from(cell.1);
        let x = 12. + cursor.0 as f64 * cw;
        let available = (size[0] - 12. - x).max(cw);
        let mut buffer = Buffer::new(
            &mut self.fonts,
            Metrics::new(cfg.font_size as f32, ch as f32),
        );
        buffer.set_size(Some(available as f32), None);
        buffer.set_wrap(glyphon::Wrap::Glyph);
        let family = super::font_family::resolve_font_family(&self.fonts, &cfg.font_family);
        let attrs = Attrs::new()
            .family(family.as_glyphon())
            .weight(glyphon::Weight(cfg.font_weight));
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let count = buffer.layout_runs().count();
        // Move the overlay upwards near the bottom so wrapped candidates remain visible.
        let y = (12. + cursor.1 as f64 * ch).min((size[1] - 12. - count as f64 * ch).max(12.));
        self.composition_origin = (x, y);
        self.layout = super::macos_policy::PreeditLayout {
            glyphs: Vec::new(),
            end: text.encode_utf16().count(),
        };
        let lines: Vec<_> = text.split('\n').collect();
        for run in buffer.layout_runs() {
            let prefix: usize = lines
                .iter()
                .take(run.line_i)
                .map(|s| s.encode_utf16().count() + 1)
                .sum();
            let line = lines.get(run.line_i).copied().unwrap_or("");
            if run.glyphs.is_empty() {
                self.layout.glyphs.push(super::macos_policy::GlyphBox {
                    start: prefix,
                    end: prefix,
                    x,
                    y: y + f64::from(run.line_top),
                    w: 1.,
                    h: ch,
                    rtl: false,
                });
            }
            for glyph in run.glyphs {
                let start = prefix + line[..glyph.start].encode_utf16().count();
                let end = prefix + line[..glyph.end].encode_utf16().count();
                self.layout.glyphs.push(super::macos_policy::GlyphBox {
                    start,
                    end,
                    x: x + f64::from(glyph.x),
                    y: y + f64::from(run.line_top),
                    w: f64::from(glyph.w),
                    h: ch,
                    rtl: glyph.level.is_rtl(),
                });
            }
        }
        if let Some((_, y, h)) = self.layout.caret(preedit.selected.0) {
            let shift = super::macos_policy::composition_shift(y, h, size[1]);
            self.composition_origin.1 += shift;
            for glyph in &mut self.layout.glyphs {
                glyph.y += shift;
            }
        }
        buffer
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
        let w = b
            .layout_runs()
            .next()
            .map_or(cfg.font_size as f32 * 0.6, |r| r.line_w);
        (
            w.ceil().max(1.) as u16,
            (cfg.font_size + cfg.line_padding).ceil() as u16,
        )
    }
    /// A drawable acquisition or queue submission alone is NOT a draw receipt.
    pub fn draw(
        &mut self,
        snapshot: &Snapshot,
        cfg: &ViewConfig,
        size: [u32; 2],
        scale: f64,
        cell: (u16, u16),
        preedit: &super::macos_policy::Preedit,
    ) -> Result<bool, HostError> {
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
            other => return Err(failure(format!("Metal drawable: {other:?}"))),
        };
        let cw = cell.0 as f32 * scale as f32;
        let ch = cell.1 as f32 * scale as f32;
        let padding = 12. * scale as f32;
        let width = self.config.width as f32;
        let height = self.config.height as f32;
        let mut vertices = Vec::<u8>::new();
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
        let mut buffers = Vec::new();
        let family = super::font_family::resolve_font_family(&self.fonts, &cfg.font_family);
        for (index, c) in snapshot.cells.iter().enumerate() {
            if c.width == 0 {
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
                    y + ch - 2. * scale as f32,
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
                buffers.push((b, x, y, cw * c.width as f32, color));
            }
        }
        let mut composition = None;
        if !preedit.text.is_empty() {
            let buffer = self.preedit_buffer(
                preedit,
                cfg,
                cell,
                snapshot.cursor,
                [f64::from(size[0]) / scale, f64::from(size[1]) / scale],
            );
            let factor = scale as f32;
            for g in &self.layout.glyphs {
                let selected = g.start < preedit.selected.0.saturating_add(preedit.selected.1)
                    && g.end > preedit.selected.0;
                rect(
                    &mut vertices,
                    g.x as f32 * factor,
                    g.y as f32 * factor,
                    g.w as f32 * factor,
                    ch,
                    if selected {
                        [60, 85, 120]
                    } else {
                        cfg.background
                    },
                );
                rect(
                    &mut vertices,
                    g.x as f32 * factor,
                    (g.y + g.h - 1.) as f32 * factor,
                    g.w as f32 * factor,
                    factor,
                    cfg.foreground,
                );
            }
            if let Some((x, y, h)) = self.layout.caret(preedit.selected.0) {
                rect(
                    &mut vertices,
                    x as f32 * factor,
                    y as f32 * factor,
                    factor,
                    h as f32 * factor,
                    cfg.foreground,
                );
            }
            let x = self.composition_origin.0 as f32 * factor;
            let y = self.composition_origin.1 as f32 * factor;
            composition = Some((buffer, x, y, factor));
        } else {
            self.layout = Default::default();
        }
        if snapshot.cursor_visible && preedit.text.is_empty() {
            let x = padding + snapshot.cursor.0 as f32 * cw;
            let y = padding + snapshot.cursor.1 as f32 * ch;
            let (y, w, h) = match snapshot.cursor_shape.as_str() {
                "Beam" => (y, 2. * scale as f32, ch),
                "Block" => (y, cw, ch),
                _ => (y + ch - 2. * scale as f32, cw, 2. * scale as f32),
            };
            rect(&mut vertices, x, y, w, h, cfg.foreground);
        }
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
                buffers
                    .iter()
                    .map(|(b, x, y, w, c)| TextArea {
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
                    })
                    .chain(composition.iter().map(|(buffer, x, y, scale)| TextArea {
                        buffer,
                        left: *x,
                        top: *y,
                        scale: *scale,
                        bounds: TextBounds {
                            left: 0,
                            top: 0,
                            right: size[0] as i32,
                            bottom: size[1] as i32,
                        },
                        default_color: Color::rgb(
                            cfg.foreground[0],
                            cfg.foreground[1],
                            cfg.foreground[2],
                        ),
                        custom_glyphs: &[],
                    })),
                &mut self.swash,
            )
            .map_err(failure)?;
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
        let submission = self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(2)),
            })
            .map_err(failure)?;
        self.atlas.trim();
        Ok(true)
    }
}
