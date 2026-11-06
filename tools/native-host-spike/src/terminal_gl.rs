#![allow(unsafe_op_in_unsafe_fn)]
use crate::interaction::AtlasSlots;
use crate::{Cell, Snapshot};
use glib::translate::{ToGlibPtr, from_glib_full};
use glow::HasContext;
use gtk::{cairo, pango};

pub fn cell_rect(index: usize, cols: usize, width: usize) -> (f32, f32, f32, f32) {
    (
        12.0 + (index % cols) as f32 * 11.0,
        12.0 + (index / cols) as f32 * 23.0,
        width as f32 * 11.0,
        23.0,
    )
}
pub fn effective_colors(cell: &Cell, selected: bool) -> ([u8; 3], [u8; 3]) {
    (
        cell.foreground,
        if selected {
            [60, 85, 120]
        } else {
            cell.background
        },
    )
}
#[derive(Default)]
pub struct Lifecycle {
    pub created: usize,
    pub frames: usize,
    pub deleted: usize,
    active: bool,
}
impl Lifecycle {
    pub fn realize(&mut self) {
        if !self.active {
            self.active = true;
            self.created += 1;
        }
    }
    pub fn frame(&mut self) {
        assert!(self.active);
        self.frames += 1;
    }
    pub fn unrealize(&mut self) {
        if self.active {
            self.active = false;
            self.deleted += 1;
        }
    }
}
#[link(name = "GL")]
unsafe extern "C" {
    fn glXGetProcAddress(name: *const u8) -> *const std::ffi::c_void;
}
#[link(name = "pangocairo-1.0")]
unsafe extern "C" {
    fn pango_cairo_create_layout(cr: *mut cairo::ffi::cairo_t) -> *mut pango::ffi::PangoLayout;
    fn pango_cairo_show_layout(cr: *mut cairo::ffi::cairo_t, layout: *mut pango::ffi::PangoLayout);
}

pub struct Renderer {
    gl: glow::Context,
    program: glow::Program,
    texture: glow::Texture,
    vao: glow::VertexArray,
    buffer: glow::Buffer,
    glyphs: AtlasSlots,
    pub glyph_uploads: usize,
    pub renderer: String,
    pub pixel_ink: usize,
    pub pixel_red: usize,
    pub glyph_regions: usize,
    pub lifecycle: Lifecycle,
}
impl Renderer {
    /// # Safety
    /// GTK must have made the GLArea context current, with its FBO bound.
    pub unsafe fn new() -> Self {
        let gl = glow::Context::from_loader_function(|name| {
            let c = std::ffi::CString::new(name).unwrap();
            glXGetProcAddress(c.as_ptr().cast())
        });
        let program = gl.create_program().unwrap();
        for (kind, source) in [
            (
                glow::VERTEX_SHADER,
                "#version 150\nin vec2 position; in vec2 uv; in vec3 color; in float glyph; uniform vec2 viewport; out vec2 tex; out vec3 rgb; out float mode; void main(){gl_Position=vec4(position.x/viewport.x*2.-1.,1.-position.y/viewport.y*2.,0.,1.);tex=uv;rgb=color;mode=glyph;}",
            ),
            (
                glow::FRAGMENT_SHADER,
                "#version 150\nuniform sampler2D atlas; in vec2 tex; in vec3 rgb; in float mode; out vec4 pixel; void main(){float a=mode>0.5?texture(atlas,tex).r:1.;pixel=vec4(rgb,a);}",
            ),
        ] {
            let shader = gl.create_shader(kind).unwrap();
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            assert!(
                gl.get_shader_compile_status(shader),
                "{}",
                gl.get_shader_info_log(shader)
            );
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        assert!(
            gl.get_program_link_status(program),
            "{}",
            gl.get_program_info_log(program)
        );
        let vao = gl.create_vertex_array().unwrap();
        let buffer = gl.create_buffer().unwrap();
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
        for (name, count, offset) in [
            ("position", 2, 0),
            ("uv", 2, 8),
            ("color", 3, 16),
            ("glyph", 1, 28),
        ] {
            let location = gl.get_attrib_location(program, name).unwrap();
            gl.enable_vertex_attrib_array(location);
            gl.vertex_attrib_pointer_f32(location, count, glow::FLOAT, false, 32, offset);
        }
        let texture = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::R8 as i32,
            1024,
            1024,
            0,
            glow::RED,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&vec![0; 1024 * 1024])),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::NEAREST as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::NEAREST as i32,
        );
        let renderer = gl.get_parameter_string(glow::RENDERER);
        let mut lifecycle = Lifecycle::default();
        lifecycle.realize();
        Self {
            gl,
            program,
            texture,
            vao,
            buffer,
            glyphs: AtlasSlots::new(atlas_capacity()),
            glyph_uploads: 0,
            renderer,
            pixel_ink: 0,
            pixel_red: 0,
            glyph_regions: 0,
            lifecycle,
        }
    }
    unsafe fn glyph(&mut self, text: &str, width: usize, flags: u16) -> usize {
        let key = format!("{width}:{flags}:{text}");
        if let Some(slot) = self.glyphs.get(&key) {
            return slot;
        }
        let slot = self.glyphs.insert(&key);
        // Only ONE cell's glyph/combining sequence rasterized here. Never an entire screen.
        let mut surface = cairo::ImageSurface::create(cairo::Format::A8, 32, 32).unwrap();
        {
            let cr = cairo::Context::new(&surface).unwrap();
            let layout: pango::Layout = from_glib_full(pango_cairo_create_layout(cr.to_raw_none()));
            let mut font = pango::FontDescription::from_string("monospace");
            if flags & 2 != 0 {
                font.set_weight(pango::Weight::Bold);
            }
            if flags & 4 != 0 {
                font.set_style(pango::Style::Italic);
            }
            font.set_absolute_size(17.0 * f64::from(pango::SCALE));
            layout.set_font_description(Some(&font));
            layout.set_text(text);
            cr.rectangle(0.0, 0.0, (width * 11) as f64, 23.0);
            cr.clip();
            cr.set_source_rgba(1., 1., 1., 1.);
            pango_cairo_show_layout(cr.to_raw_none(), layout.to_glib_none().0);
        }
        surface.flush();
        let stride = surface.stride() as usize;
        let mut alpha = vec![0u8; 1024];
        {
            let data = surface.data().unwrap();
            for y in 0..32 {
                alpha[y * 32..y * 32 + 32].copy_from_slice(&data[y * stride..y * stride + 32]);
            }
        }
        self.gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        self.gl.tex_sub_image_2d(
            glow::TEXTURE_2D,
            0,
            (slot % 32 * 32) as i32,
            (slot / 32 * 32) as i32,
            32,
            32,
            glow::RED,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&alpha)),
        );
        self.glyph_uploads += 1;
        slot
    }
    /// # Safety
    /// The renderer's GLArea context must be current and its live framebuffer bound.
    pub unsafe fn draw(
        &mut self,
        snapshot: &Snapshot,
        width: i32,
        height: i32,
        scale: i32,
        selection: Option<(usize, usize)>,
    ) {
        self.lifecycle.frame();
        let gl = &self.gl;
        gl.viewport(0, 0, width * scale, height * scale);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::DEPTH_TEST);
        gl.clear_color(24. / 255., 28. / 255., 36. / 255., 1.);
        gl.clear(glow::COLOR_BUFFER_BIT);
        let mut vertices = Vec::<f32>::new();
        for (index, cell) in snapshot.cells.iter().enumerate() {
            if cell.width == 0 {
                continue;
            }
            let rect = cell_rect(index, snapshot.cols, cell.width);
            let selected = selection.is_some_and(|(a, b)| index >= a.min(b) && index <= a.max(b));
            let (mut fg, bg) = effective_colors(cell, selected);
            if cell.flags & 128 != 0 {
                fg = fg.map(|v| (v as f32 * 0.6) as u8);
            }
            if cell.text != " "
                && self
                    .glyphs
                    .needs_flush(&format!("{}:{}:{}", cell.width, cell.flags, cell.text))
            {
                self.submit(&vertices, width, height);
                vertices.clear();
                self.glyphs.clear_page();
            }
            quad(&mut vertices, rect, [0., 0., 0., 0.], bg, 0.);
            if cell.text != " " {
                let slot = self.glyph(&cell.text, cell.width, cell.flags);
                let u = (slot % 32 * 32) as f32 / 1024.;
                let v = (slot / 32 * 32) as f32 / 1024.;
                quad(
                    &mut vertices,
                    (rect.0, rect.1, rect.2, 23.),
                    [u, v, u + rect.2 / 1024., v + 23. / 1024.],
                    fg,
                    1.,
                );
            }
        }
        for (index, cell) in snapshot.cells.iter().enumerate() {
            let r = cell_rect(index, snapshot.cols, cell.width);
            if cell.flags & 8 != 0 {
                quad(
                    &mut vertices,
                    (r.0, r.1 + 20., r.2, 1.),
                    [0.; 4],
                    cell.foreground,
                    0.,
                );
            }
            if cell.flags & 512 != 0 {
                quad(
                    &mut vertices,
                    (r.0, r.1 + 11., r.2, 1.),
                    [0.; 4],
                    cell.foreground,
                    0.,
                );
            }
        }
        if snapshot.cursor_visible {
            let index = snapshot.cursor.1 * snapshot.cols + snapshot.cursor.0;
            let r = cell_rect(index, snapshot.cols, 1);
            quad(
                &mut vertices,
                match snapshot.cursor_shape.as_str() {
                    "Beam" => (r.0, r.1, 2., 23.),
                    "Block" => (r.0, r.1, 11., 23.),
                    _ => (r.0, r.1 + 21., 11., 2.),
                },
                [0.; 4],
                [230, 230, 230],
                0.,
            );
        }
        self.submit(&vertices, width, height);
        let gl = &self.gl;
        // Assertion-only readback of the first cell row. This is NEVER used for presentation.
        let w = (width * scale).min(900);
        let h = 23 * scale;
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
        gl.read_pixels(
            0,
            (height - 35) * scale,
            w,
            h,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        self.pixel_ink = pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 70 || p[1] > 70 || p[2] > 80)
            .count();
        self.pixel_red = pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 120 && p[0] > p[1].saturating_mul(2))
            .count();
        self.glyph_regions = 0;
        for x in 0..snapshot.cols.min((width as usize).saturating_sub(24) / 11) {
            let mut ink = false;
            for y in 0..h as usize {
                for dx in 0..11 * scale as usize {
                    let px = (12 + x * 11) * scale as usize + dx;
                    if px < w as usize {
                        let o = (y * w as usize + px) * 4;
                        ink |= pixels[o] > 70 || pixels[o + 1] > 70 || pixels[o + 2] > 80;
                    }
                }
            }
            self.glyph_regions += usize::from(ink);
        }
    }
    pub fn atlas_pages(&self) -> usize {
        self.glyphs.pages
    }
    pub fn atlas_slots(&self) -> usize {
        self.glyphs.len()
    }
    unsafe fn submit(&self, vertices: &[f32], width: i32, height: i32) {
        let gl = &self.gl;
        gl.use_program(Some(self.program));
        gl.uniform_2_f32(
            gl.get_uniform_location(self.program, "viewport").as_ref(),
            width as f32,
            height as f32,
        );
        gl.uniform_1_i32(gl.get_uniform_location(self.program, "atlas").as_ref(), 0);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
        gl.bind_vertex_array(Some(self.vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
        let bytes = std::slice::from_raw_parts(vertices.as_ptr().cast::<u8>(), vertices.len() * 4);
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.draw_arrays(glow::TRIANGLES, 0, (vertices.len() / 8) as i32);
        gl.disable(glow::BLEND);
    }
    /// # Safety
    /// The original GLArea context must still be current; call before unrealize completes.
    pub unsafe fn destroy(mut self) {
        self.lifecycle.unrealize();
        println!(
            "{}",
            serde_json::json!({"event":"gl_renderer_destroyed","created":self.lifecycle.created,"frames":self.lifecycle.frames,"deleted":self.lifecycle.deleted})
        );
        self.gl.delete_buffer(self.buffer);
        self.gl.delete_vertex_array(self.vao);
        self.gl.delete_texture(self.texture);
        self.gl.delete_program(self.program);
    }
}
pub fn atlas_capacity() -> usize {
    let args: Vec<_> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--atlas-slots") {
        let n = args
            .get(i + 1)
            .expect("atlas slots value")
            .parse::<usize>()
            .expect("numeric atlas slots");
        assert!((1..=1024).contains(&n));
        n
    } else {
        1024
    }
}
fn quad(v: &mut Vec<f32>, r: (f32, f32, f32, f32), uv: [f32; 4], color: [u8; 3], mode: f32) {
    for (x, y, u, t) in [
        (r.0, r.1, uv[0], uv[1]),
        (r.0 + r.2, r.1, uv[2], uv[1]),
        (r.0, r.1 + r.3, uv[0], uv[3]),
        (r.0, r.1 + r.3, uv[0], uv[3]),
        (r.0 + r.2, r.1, uv[2], uv[1]),
        (r.0 + r.2, r.1 + r.3, uv[2], uv[3]),
    ] {
        v.extend([
            x,
            y,
            u,
            t,
            color[0] as f32 / 255.,
            color[1] as f32 / 255.,
            color[2] as f32 / 255.,
            mode,
        ]);
    }
}
