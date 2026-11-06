#[cfg(not(windows))]
fn main() {
    eprintln!("windows-host requires Windows; Linux composition is a separate experiment");
}

#[cfg(windows)]
mod native_child;

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../main.rs"]
mod renderer;

#[cfg(windows)]
fn main() {
    if std::env::args().any(|arg| arg == "--pty-child") {
        if let Err(error) = native_terminal_spike::pty::smoke_child() {
            eprintln!("{error}");
            std::process::exit(1)
        }
        return;
    }
    if let Err(error) = host::run() {
        eprintln!(
            "{}",
            serde_json::json!({"ok":false,"error":error.to_string(),"proof":"windows_child_host"})
        );
        std::process::exit(1)
    }
}

#[cfg(windows)]
mod host {
    use super::renderer::{Mode, State};
    use native_terminal_spike::host_policy::{Rect, place};
    use std::{
        num::NonZeroIsize,
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::*,
        System::Threading::GetCurrentProcessId,
        UI::{
            Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled, SetFocus},
            WindowsAndMessaging::*,
        },
    };
    use winit::{
        application::ApplicationHandler,
        dpi::PhysicalSize,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        raw_window_handle::{
            HasWindowHandle, RawDisplayHandle, RawWindowHandle, Win32WindowHandle,
            WindowsDisplayHandle,
        },
        window::{Window, WindowId},
    };
    type Error = Box<dyn std::error::Error>;
    struct Native {
        hwnd: HWND,
    }
    impl Drop for Native {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.hwnd);
            }
        }
    }
    struct Host {
        terminal: Option<State>,
        web: Option<wry::WebView>,
        child: Option<Native>,
        window: Option<Arc<Window>>,
        messages: Arc<Mutex<Vec<String>>>,
        started: Instant,
        phase: usize,
        ack: bool,
        checks: Vec<serde_json::Value>,
        error: Option<String>,
        last_step: Instant,
        finished: bool,
        input: Box<super::native_child::InputState>,
        native_sent: bool,
    }
    impl Host {
        fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Error> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("WS_CHILD GPU + WebView2 same-process spike / NOT production")
                        .with_inner_size(PhysicalSize::new(1280, 800)),
                )?,
            );
            let parent = match window.window_handle()?.as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get() as HWND,
                _ => return Err("not Win32".into()),
            };
            let inbox = self.messages.clone();
            let profile =
                std::env::temp_dir().join(format!("aerotab-spike-webview-{}", std::process::id()));
            let mut context = wry::WebContext::new(Some(profile));
            let web = wry::WebViewBuilder::new_with_web_context(&mut context)
                .with_html(include_str!("../../host-fixture.html"))
                .with_ipc_handler(move |request| inbox.lock().unwrap().push(request.body().clone()))
                .build_as_child(&*window)?;
            use wry::WebViewExtWindows;
            let mut web_parent = Default::default();
            unsafe {
                web.controller().ParentWindow(&mut web_parent)?;
            }
            let web_parent = web_parent.0 as HWND;
            if unsafe { GetAncestor(web_parent, GA_ROOT) } != parent {
                return Err("WebView2 is not under the same top-level HWND".into());
            }
            // Native child is created in this process, directly under the same top-level HWND.
            let hwnd = super::native_child::create(parent, &mut self.input)?;
            if hwnd.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            self.child = Some(Native { hwnd });
            let raw = Win32WindowHandle::new(NonZeroIsize::new(hwnd as isize).unwrap());
            let mut terminal = pollster::block_on(State::new_on_surface(
                window.clone(),
                event_loop,
                Mode::PtySmoke,
                Some((
                    RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                    RawWindowHandle::Win32(raw),
                )),
            ))?;
            terminal.hosted = true;
            terminal.native_smoke = true;
            self.terminal = Some(terminal);
            self.web = Some(web);
            self.window = Some(window);
            Ok(())
        }
        fn tick(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Error> {
            if self.finished {
                return Ok(());
            }
            if self.started.elapsed() > Duration::from_secs(35) {
                return Err(format!(
                    "host deadline phase={} checks={:?} terminal={:?}",
                    self.phase,
                    self.checks,
                    self.terminal.as_ref().map(|t| t.evidence())
                )
                .into());
            }
            let messages = std::mem::take(&mut *self.messages.lock().unwrap());
            for message in messages {
                let value: serde_json::Value = serde_json::from_str(&message)?;
                let seq = value["seq"].as_u64().ok_or("missing seq")? as usize;
                if seq != self.phase {
                    continue;
                }
                let rect = Rect {
                    x: value["x"].as_f64().ok_or("x")?,
                    y: value["y"].as_f64().ok_or("y")?,
                    width: value["width"].as_f64().ok_or("width")?,
                    height: value["height"].as_f64().ok_or("height")?,
                };
                let p = place(
                    rect,
                    value["scale"].as_f64().ok_or("scale")?,
                    value["active"].as_bool().unwrap_or(false),
                    value["modal"].as_bool().unwrap_or(true),
                );
                let child = self.child.as_ref().unwrap().hwnd;
                unsafe {
                    EnableWindow(child, i32::from(p.input));
                    SetWindowPos(
                        child,
                        HWND_TOP,
                        p.x,
                        p.y,
                        p.width,
                        p.height,
                        SWP_NOACTIVATE
                            | if p.visible {
                                SWP_SHOWWINDOW
                            } else {
                                SWP_HIDEWINDOW
                            },
                    );
                    if !p.visible {
                        self.web.as_ref().unwrap().focus()?;
                    }
                }
                let parent = match self.window.as_ref().unwrap().window_handle()?.as_raw() {
                    RawWindowHandle::Win32(h) => h.hwnd.get() as HWND,
                    _ => unreachable!(),
                };
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(child, &mut pid) };
                let same_parent = unsafe {
                    GetParent(child) == parent
                        && IsChild(parent, child) != 0
                        && pid == GetCurrentProcessId()
                };
                let mut actual: RECT = unsafe { std::mem::zeroed() };
                unsafe {
                    GetWindowRect(child, &mut actual);
                }
                let mut origin = POINT {
                    x: actual.left,
                    y: actual.top,
                };
                unsafe {
                    windows_sys::Win32::Graphics::Gdi::ScreenToClient(parent, &mut origin);
                }
                if [
                    origin.x,
                    origin.y,
                    actual.right - actual.left,
                    actual.bottom - actual.top,
                ] != [p.x, p.y, p.width, p.height]
                {
                    return Err("native child geometry differs from DOM physical rect".into());
                }
                let visible = unsafe { IsWindowVisible(child) != 0 };
                let enabled = unsafe { IsWindowEnabled(child) != 0 };
                if !same_parent || visible != p.visible || enabled != p.input {
                    return Err("child parent/visibility/input assertion failed".into());
                }
                // API focus proof only, not native keyboard/IME behavioral proof.
                let focus = if p.visible {
                    unsafe {
                        SetFocus(child);
                        GetFocus() == child
                    }
                } else {
                    unsafe { GetFocus() != child }
                };
                if self.phase == 4 && value["count"].as_u64() != Some(1) {
                    return Err("business DOM callback failed".into());
                }
                if !p.visible {
                    self.input.queue.clear();
                    unsafe {
                        SendMessageW(child, WM_CHAR, b'X' as usize, 0);
                        SendMessageW(child, WM_KEYDOWN, 0x26, 0);
                    }
                    if !self.input.queue.is_empty() {
                        return Err("hidden/modal child accepted native input".into());
                    }
                }
                self.checks.push(serde_json::json!({"hidden_native_input_blocked":!p.visible,"phase":seq,"same_parent_and_process":same_parent,"visible":visible,"enabled":enabled,"focus_api":focus,"physical_rect":[p.x,p.y,p.width,p.height],"geometry_readback_verified":true,"dom_scale":value["scale"],"business_count":value["count"]}));
                // Keep the controlled PTY at 80x20 until it acknowledges resize and exits.
                if self.terminal.as_ref().unwrap().pty_complete {
                    self.terminal.as_mut().unwrap().resize(PhysicalSize::new(
                        p.width.max(1) as u32,
                        p.height.max(1) as u32,
                    ))?;
                }
                self.ack = true;
                self.last_step = Instant::now();
            }
            if let Some(terminal) = &mut self.terminal {
                self.input.app_cursor = terminal.app_cursor();
                self.input.cursor = terminal.cursor_position();
                if terminal.native_ready() && !self.native_sent {
                    let child = self.child.as_ref().unwrap().hwnd;
                    unsafe {
                        SetFocus(
                            self.window
                                .as_ref()
                                .unwrap()
                                .window_handle()
                                .ok()
                                .and_then(|h| match h.as_raw() {
                                    RawWindowHandle::Win32(h) => Some(h.hwnd.get() as HWND),
                                    _ => None,
                                })
                                .unwrap(),
                        );
                        SendMessageW(child, WM_CHAR, b'X' as usize, 0);
                        if !self.input.queue.is_empty() {
                            return Err("unfocused child accepted input".into());
                        }
                        SetFocus(child);
                        SendMessageW(child, WM_IME_STARTCOMPOSITION, 0, 0);
                        SendMessageW(child, WM_CHAR, b'X' as usize, 0);
                        SendMessageW(child, WM_IME_ENDCOMPOSITION, 0, 0);
                        if !self.input.queue.is_empty() {
                            return Err("IME preedit leaked into PTY".into());
                        }
                        for unit in "Native".encode_utf16() {
                            SendMessageW(child, WM_CHAR, unit as usize, 0);
                        }
                        SendMessageW(child, WM_UNICHAR, 0x4e2d, 0);
                        SendMessageW(child, WM_CHAR, 0xd83d, 0);
                        SendMessageW(child, WM_CHAR, 0xde42, 0);
                        SendMessageW(child, WM_CHAR, 10, 0);
                    }
                    self.native_sent = true;
                }
                while let Some(bytes) = self.input.queue.pop_front() {
                    terminal.input(&bytes)?;
                }
                if self.phase == 1 || self.phase == 3 {
                    terminal.pump()?;
                } else {
                    terminal.render()?;
                }
            }
            if self.ack
                && self.last_step.elapsed() > Duration::from_millis(180)
                && self.terminal.as_ref().unwrap().pty_complete
            {
                if self.phase == 4 {
                    let terminal = self.terminal.as_mut().unwrap();
                    terminal.shutdown()?;
                    if !terminal.native_ack
                        || terminal
                            .pixels
                            .as_ref()
                            .and_then(|p| p["glyph_background_difference"].as_bool())
                            != Some(true)
                    {
                        return Err("native input or actual glyph pixel readback failed".into());
                    }
                    println!(
                        "{}",
                        serde_json::json!({"ok":true,"proof":"in_process_WS_CHILD_GPU_and_WebView2_same_top_level","tauri":false,"wry":true,"webview_controller_same_top_level_asserted":true,"embedded_native_child":true,"checks":self.checks,"terminal":terminal.evidence(),"gpu_pixel_readback":terminal.pixels,"modal_and_tab_hide_restore":true,"business_webview_not_input_transparent":true,"native_input_synthetic_sendmessage":true,"native_input_ack":terminal.native_ack,"wndproc_input_messages":self.input.messages,"ime_result_events":self.input.ime_results,"native_keyboard_ime_manually_tested":false,"elapsed_ms":self.started.elapsed().as_millis()})
                    );
                    self.finished = true;
                    event_loop.exit();
                    return Ok(());
                }
                self.phase += 1;
                self.ack = false;
                if self.phase == 4 {
                    let _ = self
                        .window
                        .as_ref()
                        .unwrap()
                        .request_inner_size(PhysicalSize::new(1200, 760));
                }
                self.web
                    .as_ref()
                    .unwrap()
                    .evaluate_script(&format!("window.step({})", self.phase))?;
            }
            Ok(())
        }
    }
    impl ApplicationHandler for Host {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_none()
                && let Err(error) = self.start(event_loop)
            {
                self.error = Some(error.to_string());
                event_loop.exit()
            }
        }
        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(16),
            ));
            if self.window.is_some()
                && let Err(error) = self.tick(event_loop)
            {
                self.error = Some(error.to_string());
                event_loop.exit()
            }
        }
        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => {
                    self.error = Some("host closed before smoke completed".into());
                    event_loop.exit()
                }
                WindowEvent::Resized(size) => {
                    if let Some(web) = &self.web {
                        let _ = web.set_bounds(wry::Rect {
                            position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                            size: wry::dpi::PhysicalSize::new(size.width, size.height).into(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    pub fn run() -> Result<(), Error> {
        let event_loop = EventLoop::new()?;
        let mut host = Host {
            terminal: None,
            web: None,
            child: None,
            window: None,
            messages: Default::default(),
            started: Instant::now(),
            phase: 0,
            ack: false,
            checks: Vec::new(),
            error: None,
            last_step: Instant::now(),
            finished: false,
            input: Box::default(),
            native_sent: false,
        };
        event_loop.run_app(&mut host)?;
        if let Some(t) = &mut host.terminal {
            t.shutdown()?
        }
        host.terminal.take();
        host.web.take();
        host.child.take();
        let profile =
            std::env::temp_dir().join(format!("aerotab-spike-webview-{}", std::process::id()));
        // Remove only this process's isolated temporary WebView2 profile, never user data.
        if let Err(error) = std::fs::remove_dir_all(&profile) {
            eprintln!("temporary WebView2 profile cleanup incomplete: {error}");
        }
        if let Some(error) = host.error {
            return Err(error.into());
        }
        Ok(())
    }
}
