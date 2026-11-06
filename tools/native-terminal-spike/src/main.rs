//! Standalone native-surface + PTY subproof, NOT an embedded terminal.
use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use native_terminal_spike::{Snapshot, Terminal};
use native_terminal_spike::{
    input::{encode_commit, encode_key},
    pty::{Session, grid_size, interactive_command, smoke_child, smoke_command},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use wgpu::util::DeviceExt;
use winit::raw_window_handle::HasWindowHandle;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

const CELL_W: f32 = 11.0;
const CELL_H: f32 = 23.0;
const PAD: f32 = 12.0;
const SMOKE_FRAMES: u32 = 8;
const DEMO: &str = "Native GPU surface / glyphon + wgpu + alacritty_terminal\r\n\
STATIC VT FIXTURE — no PTY, SSH or embedded view in this spike.\r\n\
\x1b[31mANSI red\x1b[32m green\x1b[34m blue\x1b[0m  fixed grid: 0123456789\r\n\
\x1b[38;2;255;170;40mTruecolor foreground\x1b[0m \x1b[48;5;24mindexed background\x1b[0m\r\n\
Unicode: 中文 日本語 한국어  e\u{301}  λ Ω →  wide cells\r\n\
\x1b[7mInverse cells\x1b[0m  Resize the native window; Escape closes.\r\n\r\n\
Cursor: ";

type Failure = Box<dyn std::error::Error>;

fn main() {
    let mut mode = Mode::Static;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--smoke" => mode = Mode::Smoke,
            "--interactive" => mode = Mode::Interactive,
            "--pty-smoke" => mode = Mode::PtySmoke,
            "--pty-child" => {
                if let Err(error) = smoke_child() {
                    eprintln!("{error}");
                    std::process::exit(1);
                }
                return;
            }
            "--help" | "-h" => {
                println!(
                    "native-terminal-spike [--smoke | --interactive | --pty-smoke]\nStandalone native surface, NOT embedded. Default/--smoke: static VT fixture. --interactive: real PTY shell. --pty-smoke: bounded controlled PTY roundtrip, protocol reply, resize, EOF and cleanup evidence. Software adapters are identified."
                );
                return;
            }
            _ => {
                eprintln!("Unknown argument: {argument}");
                std::process::exit(2);
            }
        }
    }
    if let Err(error) = run(mode) {
        eprintln!(
            "{}",
            serde_json::json!({"ok": false, "error": error.to_string(), "embedded": false, "interactive": false})
        );
        std::process::exit(1);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Static,
    Smoke,
    Interactive,
    PtySmoke,
}
impl Mode {
    fn bounded(self) -> bool {
        matches!(self, Self::Smoke | Self::PtySmoke)
    }
    fn pty(self) -> bool {
        matches!(self, Self::Interactive | Self::PtySmoke)
    }
}

fn run(mode: Mode) -> Result<(), Failure> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        state: None,
        mode,
        started: Instant::now(),
        failure: None,
        finished: false,
    };
    event_loop.run_app(&mut app)?;
    if let Some(state) = &mut app.state
        && let Some(session) = &mut state.session
    {
        session.shutdown()?;
    }
    if let Some(error) = app.failure {
        return Err(error.into());
    }
    if mode.bounded() && !app.finished {
        return Err("window closed before smoke completed".into());
    }
    Ok(())
}

struct App {
    state: Option<State>,
    mode: Mode,
    started: Instant,
    failure: Option<String>,
    finished: bool,
}

impl App {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl ToString) {
        self.failure = Some(error.to_string());
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title(if self.mode.pty() {
                    "Native PTY terminal spike / NOT embedded"
                } else {
                    "Native terminal spike — STATIC fixture / NOT embedded"
                })
                .with_inner_size(PhysicalSize::new(1060, 560)),
        ) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                self.fail(event_loop, error);
                return;
            }
        };
        match pollster::block_on(State::new(window, event_loop, self.mode)) {
            Ok(state) => {
                state.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.mode.bounded() || self.mode.pty() {
            if self.mode.bounded() && self.started.elapsed() > Duration::from_secs(30) {
                self.fail(
                    event_loop,
                    format!(
                        "smoke deadline exceeded: {}",
                        self.state
                            .as_ref()
                            .map(|state| format!(
                                "phase={} frames={} eof={} child_exit={:?} received={:?}",
                                state.phase,
                                state.frames,
                                state.eof,
                                state.session.as_ref().and_then(|s| s.exit_code),
                                String::from_utf8_lossy(&state.received)
                            ))
                            .unwrap_or_else(|| "no surface".into())
                    ),
                );
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(16),
            ));
            if let Some(state) = &self.state {
                state.window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(modifiers) => state.modifiers = modifiers.state(),
            WindowEvent::Ime(winit::event::Ime::Preedit(text, _)) => {
                state.ime_composing = !text.is_empty()
            }
            WindowEvent::Ime(winit::event::Ime::Commit(text)) => {
                state.ime_composing = false;
                if let Err(error) = state.input(&encode_commit(&text)) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Ime(winit::event::Ime::Disabled) => state.ime_composing = false,
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                if !self.mode.pty()
                    && event.logical_key
                        == winit::keyboard::Key::Named(winit::keyboard::NamedKey::Escape)
                {
                    event_loop.exit();
                } else {
                    let bytes = encode_key(
                        &event.logical_key,
                        event.text.as_deref(),
                        state.modifiers,
                        state.terminal.application_cursor(),
                        state.ime_composing,
                    );
                    if let Err(error) = state.input(&bytes) {
                        self.fail(event_loop, error);
                    }
                }
            }
            WindowEvent::Resized(size) => {
                if let Err(error) = state.resize(size) {
                    self.fail(event_loop, error);
                    return;
                }
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => match state.render() {
                Ok(true)
                    if (self.mode == Mode::Smoke && state.frames >= SMOKE_FRAMES)
                        || (self.mode == Mode::PtySmoke
                            && state.pty_complete
                            && state.frames >= SMOKE_FRAMES) =>
                {
                    if let Err(error) = state.device.poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(Duration::from_secs(10)),
                    }) {
                        self.fail(event_loop, error);
                        return;
                    }
                    if let Some(session) = &mut state.session
                        && let Err(error) = session.shutdown()
                    {
                        self.fail(event_loop, error);
                        return;
                    }
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": true, "proof": "standalone_native_surface_only", "embedded": false,
                            "interactive": self.mode == Mode::Interactive, "fixture": if self.mode.pty() { "controlled_real_pty" } else { "static_vt" }, "adapter": state.info.name,
                            "pty": state.session.as_ref().map(|session| serde_json::json!({
                                "child_pid": session.pid, "bytes_to_child": session.bytes_written,
                                "bytes_from_child": state.bytes_read, "protocol_reply_bytes": state.reply_bytes,
                                "protocol_reply_owner": if cfg!(windows) { "ConPTY (DSR consumed before renderer)" } else { "alacritty events" },
                                "shell_profile_loaded": false,
                                "child_kind": "controlled executable, not interactive shell",
                                "conpty_close_thread_joined": cfg!(windows),
                                "resize_count": session.resize_count, "same_pid_resize_verified": state.pty_complete,
                                "child_exit_code": session.exit_code, "reader_joined": true,
                                "eof_seen": state.eof, "roundtrip_and_ansi_unicode_verified": state.pty_complete,
                                "ime_manually_tested": false, "native_keyboard_manually_tested": false,
                            })),
                            "backend": format!("{:?}", state.info.backend),
                            "device_type": format!("{:?}", state.info.device_type),
                            "software_adapter": state.info.device_type == wgpu::DeviceType::Cpu,
                            "driver": state.info.driver, "driver_info": state.info.driver_info,
                            "surface_format": format!("{:?}", state.config.format),
                            "surface_size": [state.config.width, state.config.height],
                            "frames_presented": state.frames, "gpu_queue_completed": true,
                            "requested_backend": std::env::var("WGPU_BACKEND").ok(),
                            "grid": [state.snapshot.cols, state.snapshot.rows],
                            "text_areas": state.text.len(), "glyphs_shaped": state.glyphs_shaped,
                            "elapsed_ms": self.started.elapsed().as_millis(),
                            "display": std::env::var("DISPLAY").ok(),
                            "wayland_display": std::env::var("WAYLAND_DISPLAY").ok(),
                            "window_handle": format!("{:?}", state.window.window_handle().map(|handle| handle.as_raw())),
                        })
                    );
                    self.finished = true;
                    event_loop.exit();
                }
                Ok(_) => {}
                Err(error) => self.fail(event_loop, error),
            },
            _ => {}
        }
    }
}

struct CellText {
    buffer: Buffer,
    left: f32,
    top: f32,
    width: f32,
    color: [u8; 3],
}

pub(crate) struct State {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    info: wgpu::AdapterInfo,
    font_system: FontSystem,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    backgrounds: wgpu::RenderPipeline,
    terminal: Terminal,
    snapshot: Snapshot,
    text: Vec<CellText>,
    frames: u32,
    pub(crate) pixels: Option<serde_json::Value>,
    glyphs_shaped: usize,
    suspended: bool,
    mode: Mode,
    session: Option<Session>,
    modifiers: winit::keyboard::ModifiersState,
    ime_composing: bool,
    received: Vec<u8>,
    bytes_read: usize,
    reply_bytes: usize,
    eof: bool,
    phase: u8,
    pub(crate) pty_complete: bool,
    pub(crate) hosted: bool,
    pub(crate) native_smoke: bool,
    pub(crate) native_ack: bool,
    window: Arc<Window>,
}

impl State {
    #[allow(dead_code)] // Shared with the optional Windows composition host.
    pub(crate) fn native_ready(&self) -> bool {
        self.phase == 2 && String::from_utf8_lossy(&self.received).contains("RESIZED:")
    }
    #[allow(dead_code)] // Used by the optional embedded child host.
    pub(crate) fn cursor_position(&self) -> (i32, i32) {
        (
            (12 + self.snapshot.cursor.0 * 11) as i32,
            (12 + (self.snapshot.cursor.1 + 1) * 23) as i32,
        )
    }
    #[allow(dead_code)] // Used by the optional embedded child host.
    pub(crate) fn app_cursor(&self) -> bool {
        self.terminal.application_cursor()
    }
    #[allow(dead_code)] // Used by the optional embedded child host.
    pub(crate) fn evidence(&self) -> serde_json::Value {
        serde_json::json!({"adapter":self.info.name,"backend":format!("{:?}",self.info.backend),"device_type":format!("{:?}",self.info.device_type),"software_adapter":self.info.device_type == wgpu::DeviceType::Cpu,"frames":self.frames,"pty_complete":self.pty_complete,"pid":self.session.as_ref().map(|s|s.pid),"bytes_from_child":self.bytes_read,"bytes_to_child":self.session.as_ref().map(|s|s.bytes_written),"eof":self.eof})
    }
    #[allow(dead_code)]
    pub(crate) fn shutdown(&mut self) -> Result<(), Failure> {
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        })?;
        if let Some(session) = &mut self.session {
            session.shutdown()?;
        }
        Ok(())
    }
    pub(crate) async fn new(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        mode: Mode,
    ) -> Result<Self, Failure> {
        Self::new_on_surface(window, event_loop, mode, None).await
    }

    pub(crate) async fn new_on_surface(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        mode: Mode,
        child: Option<(wgpu::rwh::RawDisplayHandle, wgpu::rwh::RawWindowHandle)>,
    ) -> Result<Self, Failure> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = if let Some((raw_display_handle, raw_window_handle)) = child {
            // Host owns this same-process WS_CHILD until after State and its surface drop.
            unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_display_handle),
                    raw_window_handle,
                })?
            }
        } else {
            instance.create_surface(window.clone())?
        };
        // Compatible surface is mandatory. No offscreen/WebView fallback is permitted.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("adapter has no usable native surface configuration")?;
        if surface
            .get_capabilities(&adapter)
            .usages
            .contains(wgpu::TextureUsages::COPY_SRC)
        {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&device, &config);
        let font_system = FontSystem::new();
        if font_system.db().faces().next().is_none() {
            return Err("no system fonts available".into());
        }
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, config.format);
        let renderer =
            TextRenderer::new(&mut atlas, &device, wgpu::MultisampleState::default(), None);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cell backgrounds and cursor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("cells.wgsl").into()),
        });
        let backgrounds = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("native terminal cell rectangles"),
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
        let mut terminal = Terminal::new(92, 22);
        if !mode.pty() {
            terminal.feed(DEMO.as_bytes());
        }
        let (cols, rows) = grid_size(size.width, size.height);
        window.set_ime_allowed(mode == Mode::Interactive);
        let session = if mode.pty() {
            Some(Session::spawn(
                if mode == Mode::PtySmoke {
                    smoke_command()?
                } else {
                    interactive_command()
                },
                cols,
                rows,
            )?)
        } else {
            None
        };
        let snapshot = terminal.snapshot();
        let mut state = Self {
            device,
            queue,
            surface,
            config,
            info,
            font_system,
            swash: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            backgrounds,
            terminal,
            snapshot,
            text: Vec::new(),
            frames: 0,
            pixels: None,
            glyphs_shaped: 0,
            suspended: false,
            mode,
            session,
            modifiers: Default::default(),
            ime_composing: false,
            received: Vec::new(),
            bytes_read: 0,
            reply_bytes: 0,
            eof: false,
            phase: 0,
            pty_complete: false,
            hosted: child.is_some(),
            native_smoke: false,
            native_ack: false,
            window,
        };
        state.resize(size)?;
        Ok(state)
    }

    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) -> Result<(), Failure> {
        self.suspended = size.width == 0 || size.height == 0;
        if self.suspended {
            return Ok(());
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        let (cols, rows) = grid_size(size.width, size.height);
        self.terminal.resize(cols as usize, rows as usize);
        if let Some(session) = &mut self.session
            && session.exit_code.is_none()
        {
            session.resize(cols, rows)?;
        }
        self.rebuild_text();
        Ok(())
    }

    fn rebuild_text(&mut self) {
        self.snapshot = self.terminal.snapshot();
        self.text.clear();
        self.glyphs_shaped = 0;
        for placement in native_terminal_spike::layout::placements(&self.snapshot) {
            let cell = &self.snapshot.cells[placement.index];
            let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(18.0, CELL_H));
            buffer.set_size(Some(placement.width), Some(CELL_H));
            buffer.set_wrap(glyphon::Wrap::None);
            buffer.set_text(
                &cell.text,
                &Attrs::new().family(Family::Monospace),
                Shaping::Advanced,
                None,
            );
            buffer.shape_until_scroll(&mut self.font_system, false);
            self.glyphs_shaped += buffer
                .layout_runs()
                .map(|run| run.glyphs.len())
                .sum::<usize>();
            self.text.push(CellText {
                buffer,
                left: placement.left,
                top: placement.top,
                width: placement.width,
                color: cell.foreground,
            });
        }
    }

    pub(crate) fn input(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        if bytes.is_empty() || self.eof {
            return Ok(());
        }
        if let Some(session) = &mut self.session {
            session.write(bytes)?;
        }
        Ok(())
    }

    pub(crate) fn pump(&mut self) -> Result<(), Failure> {
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        session.poll_exit()?;
        let (chunks, eof, error) = session.output.drain();
        if let Some(error) = error {
            return Err(error.into());
        }
        let changed = !chunks.is_empty();
        for chunk in chunks {
            self.bytes_read += chunk.len();
            if self.mode == Mode::PtySmoke {
                if self.received.len() + chunk.len() > 64 * 1024 {
                    return Err("smoke output exceeded bound".into());
                }
                self.received.extend_from_slice(&chunk);
            }
            self.terminal.feed(&chunk);
            // Only alacritty events generate protocol replies; no second VT parser.
            for reply in self.terminal.take_replies() {
                self.reply_bytes += reply.len();
                session.write(&reply)?;
            }
        }
        if eof && !self.eof {
            self.eof = true;
            self.window
                .set_title("Native PTY spike — [session ended] / NOT embedded");
            self.terminal.feed(b"\r\n[session ended]\r\n");
        }
        if changed || eof {
            self.rebuild_text();
        }
        if self.mode != Mode::PtySmoke {
            return Ok(());
        }
        let transcript = String::from_utf8_lossy(&self.received).into_owned();
        let pid = self.session.as_ref().unwrap().pid;
        match self.phase {
            0 if transcript.contains(&format!("READY:{pid}\r\n")) => {
                self.input(b"ROUNDTRIP_NATIVE_42\n")?;
                self.phase = 1;
            }
            1 if transcript.contains("PROTOCOL_REPLY_OK") => {
                let grid = self.terminal.snapshot();
                if !grid
                    .cells
                    .iter()
                    .any(|cell| cell.text == "中" && cell.foreground == [12, 34, 56])
                {
                    return Err("PTY ANSI/Unicode grid validation failed".into());
                }
                if !self.hosted {
                    let _ = self.window.request_inner_size(PhysicalSize::new(904, 484));
                }
                self.resize(PhysicalSize::new(904, 484))?;
                self.input(b"RESIZE\n")?;
                self.phase = 2;
            }
            2 if transcript.contains(&format!("RESIZED:{pid}:80:20")) && !self.native_smoke => {
                self.input(b"EXIT\n")?;
                self.phase = 3;
            }
            2 if self.native_smoke && transcript.contains("NATIVE_ACK:Native中🙂") => {
                self.native_ack = true;
                self.input(b"EXIT\n")?;
                self.phase = 3;
            }
            3 if self.eof
                && self.session.as_ref().unwrap().exit_code == Some(0)
                && transcript.contains("CHILD_DONE") =>
            {
                self.pty_complete = true;
            }
            _ => {}
        }
        if self.eof && !self.pty_complete && self.session.as_ref().unwrap().exit_code.is_some() {
            return Err(format!(
                "PTY smoke ended early at phase {}: {transcript}",
                self.phase
            )
            .into());
        }
        Ok(())
    }

    pub(crate) fn render(&mut self) -> Result<bool, Failure> {
        self.pump()?;
        if self.suspended {
            return Ok(false);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            other => return Err(format!("unusable native surface: {other:?}").into()),
        };
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        self.renderer.prepare(
            &self.device,
            &self.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            self.text.iter().map(|text| {
                let left = text.left;
                let top = text.top;
                TextArea {
                    buffer: &text.buffer,
                    left,
                    top,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: left as i32,
                        top: top as i32,
                        right: (left + text.width) as i32,
                        bottom: (top + CELL_H) as i32,
                    },
                    default_color: Color::rgb(text.color[0], text.color[1], text.color[2]),
                    custom_glyphs: &[],
                }
            }),
            &mut self.swash,
        )?;
        let mut vertices = Vec::with_capacity(self.snapshot.cells.len() * 120);
        for (index, cell) in self.snapshot.cells.iter().enumerate() {
            self.rectangle(
                &mut vertices,
                index % self.snapshot.cols,
                index / self.snapshot.cols,
                CELL_H,
                cell.background,
            );
        }
        // Underline cursor is deliberately non-blinking in this deterministic fixture.
        if self.snapshot.cursor_visible {
            self.rectangle(
                &mut vertices,
                self.snapshot.cursor.0,
                self.snapshot.cursor.1,
                2.0,
                [236, 239, 244],
            );
        }
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cell and cursor vertices"),
                contents: &vertices,
                usage: wgpu::BufferUsages::VERTEX,
            });
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native terminal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.009,
                            g: 0.012,
                            b: 0.018,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.backgrounds);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.draw(0..(vertices.len() / 20) as u32, 0..1);
            self.renderer
                .render(&self.atlas, &self.viewport, &mut pass)?;
        }
        let readback = if self.hosted
            && self.pixels.is_none()
            && self.frames > 8
            && self.config.usage.contains(wgpu::TextureUsages::COPY_SRC)
        {
            let pitch = (self.config.width * 4).div_ceil(256) * 256;
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("actual surface pixel readback"),
                size: (pitch * self.config.height) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                frame.texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(pitch),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: self.config.width,
                    height: self.config.height,
                    depth_or_array_layers: 1,
                },
            );
            Some((buffer, pitch))
        } else {
            None
        };
        self.queue.submit(Some(encoder.finish()));
        if let Some((buffer, pitch)) = readback {
            let (tx, rx) = std::sync::mpsc::channel();
            buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = tx.send(result);
            });
            self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(3)),
            })?;
            rx.recv_timeout(Duration::from_secs(3))??;
            let bytes = buffer.get_mapped_range(..)?;
            let mut colors = std::collections::HashSet::new();
            let mut glyph_cells = 0;
            for (index, cell) in self.snapshot.cells.iter().enumerate() {
                if cell.text.trim().is_empty() {
                    continue;
                }
                let mut local = std::collections::HashSet::new();
                for y in (12 + (index / self.snapshot.cols) * 23)
                    ..(12 + ((index / self.snapshot.cols) + 1) * 23)
                        .min(self.config.height as usize)
                {
                    for x in (12 + (index % self.snapshot.cols) * 11)
                        ..(12 + ((index % self.snapshot.cols) + 1) * 11)
                            .min(self.config.width as usize)
                    {
                        let offset = y * pitch as usize + x * 4;
                        let pixel = [
                            bytes[offset],
                            bytes[offset + 1],
                            bytes[offset + 2],
                            bytes[offset + 3],
                        ];
                        colors.insert(pixel);
                        local.insert(pixel);
                    }
                }
                if local.len() > 2 {
                    glyph_cells += 1;
                }
            }
            self.pixels = Some(
                serde_json::json!({"source":"presented_swapchain_texture","unique_rgba_values":colors.len(),"nonblank_cells_with_pixel_variation":glyph_cells,"glyph_background_difference":glyph_cells>0&&colors.len()>3}),
            );
            drop(bytes);
            buffer.unmap();
        }
        self.window.pre_present_notify();
        self.queue.present(frame);
        self.atlas.trim();
        self.frames += 1;
        Ok(true)
    }

    fn rectangle(&self, bytes: &mut Vec<u8>, col: usize, row: usize, height: f32, rgb: [u8; 3]) {
        let x = PAD + col as f32 * CELL_W;
        let y = PAD + row as f32 * CELL_H + CELL_H - height;
        for (dx, dy) in [
            (0.0, 0.0),
            (CELL_W, 0.0),
            (0.0, height),
            (0.0, height),
            (CELL_W, 0.0),
            (CELL_W, height),
        ] {
            let linear = |v: u8| {
                let c = v as f32 / 255.0;
                if self.config.format.is_srgb() {
                    if c <= 0.04045 {
                        c / 12.92
                    } else {
                        ((c + 0.055) / 1.055).powf(2.4)
                    }
                } else {
                    c
                }
            };
            for value in [
                (x + dx) / self.config.width as f32 * 2.0 - 1.0,
                1.0 - (y + dy) / self.config.height as f32 * 2.0,
                linear(rgb[0]),
                linear(rgb[1]),
                linear(rgb[2]),
            ] {
                bytes.extend_from_slice(&value.to_ne_bytes());
            }
        }
    }
}
