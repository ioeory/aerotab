//! Main-thread Win32 child host; no terminal parser, PTY or external process here.
use super::{input, windows_policy, windows_renderer::Renderer, *};
use crate::terminal::engine::TerminalEngine;
use std::{cell::RefCell, sync::Arc};
use tauri::{Emitter, Manager};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::{Ime::*, KeyboardAndMouse::*},
        WindowsAndMessaging::*,
    },
};
thread_local! {
    static VIEWS: RefCell<HashMap<Uuid, Box<Attachment>>> = RefCell::default();
    static ACTIVATION_SEQ: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
struct Callback {
    id: Uuid,
    generation: Uuid,
}
struct Attachment {
    callback: Box<Callback>,
    session_id: Uuid,
    hwnd: HWND,
    parent: HWND,
    thread: u32,
    generation: Uuid,
    lifetime: input::AttachmentLifetime<Uuid, usize>,
    engine: Arc<TerminalEngine>,
    dispatch: InputDispatch,
    app: tauri::AppHandle,
    geometry: Geometry,
    cfg: ViewConfig,
    scale: f64,
    visible: bool,
    flow: input::InputFlow,
    revision: input::ConfigRevision,
    handoff: u64,
    drawn_output: Option<u64>,
    last_seq: Option<u32>,
    error: Option<HostError>,
    renderer: Option<Renderer>,
    text: windows_policy::TextInput,
    composing: bool,
    suppress_char: bool,
    shortcuts: std::collections::VecDeque<(u64, input::Key, input::Modifiers, Option<String>)>,
    shortcut_seq: u64,
    shortcut_started: Option<std::time::Instant>,
    paste_request: u32,
    cell: (u16, u16),
    grid: Option<(u16, u16, u16, u16)>,
    pointer: input::PointerState,
    wheel: input::WheelState,
}
impl Drop for Attachment {
    fn drop(&mut self) {
        self.lifetime.invalidate();
        if self.hwnd.is_null() {
            return;
        }
        unsafe {
            KillTimer(self.hwnd, 1);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            ShowWindow(self.hwnd, SW_HIDE);
        }
        self.renderer.take();
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
fn route_key(id: Uuid, generation: Uuid) {
    let current = VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let v = views.get_mut(&id)?;
        if v.generation != generation || !v.active() {
            v.shortcuts.clear();
            return None;
        }
        let (seq, key, m, _) = v.shortcuts.front()?.clone();
        v.shortcut_started = Some(std::time::Instant::now());
        let window = v.app.get_webview_window("main")?;
        Some((
            window,
            seq,
            super::shortcut::shortcut_script(key, m, v.geometry, id),
        ))
    });
    let Some((window, seq, script)) = current else {
        return;
    };
    let app = window.app_handle().clone();
    let result = window.eval_with_callback(script, move |result| {
        let decision = serde_json::from_str::<serde_json::Value>(&result)
            .ok()
            .and_then(|v| v.get("consumed").and_then(|x| x.as_bool()));
        let _ = app.run_on_main_thread(move || {
            VIEWS.with(|views| {
                let mut views = views.borrow_mut();
                let Some(v) = views.get_mut(&id) else {
                    return;
                };
                if v.generation != generation || v.shortcuts.front().map(|k| k.0) != Some(seq) {
                    return;
                }
                let (_, key, m, text) = v.shortcuts.pop_front().unwrap();
                v.shortcut_started = None;
                if !v.active() {
                    v.shortcuts.clear();
                    return;
                }
                match decision {
                    Some(false) => {
                        if let Some(text) = text {
                            v.bytes(text.as_bytes());
                        } else {
                            v.local_key(key, m);
                        }
                    }
                    Some(true) => {}
                    None => {
                        v.shortcuts.clear();
                    }
                }
            });
            route_key(id, generation);
        });
    });
    if result.is_err() {
        VIEWS.with(|views| {
            if let Some(v) = views.borrow_mut().get_mut(&id) {
                if v.generation == generation {
                    v.shortcuts.clear();
                    v.flow.freeze();
                    v.error = Some(HostError::new(
                        "shortcut_bridge",
                        "shortcut dispatch failed",
                    ));
                }
            }
        });
    }
}
fn missing() -> HostError {
    HostError::new("not_attached", "native view not attached")
}
impl Attachment {
    fn active(&self) -> bool {
        self.lifetime.matches(self.generation, self.hwnd as usize)
            && self.visible
            && self.flow.ready
            && self.error.is_none()
            && unsafe { IsWindowVisible(self.hwnd) != 0 && GetFocus() == self.hwnd }
    }
    fn bytes(&mut self, bytes: &[u8]) {
        if !self.active() || bytes.is_empty() {
            return;
        }
        let Some(seq) = self.flow.reserve(bytes.len()) else {
            self.error = Some(HostError::new(
                "input_overflow",
                "native input bridge capacity exceeded",
            ));
            self.flow.freeze();
            return;
        };
        if self.app.emit_to(tauri::EventTarget::webview_window("main"),"aerotab:native-input",serde_json::json!({"sessionId":self.engine_id(),"generation":self.generation,"seq":seq,"data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes)})).is_err() { self.error=Some(HostError::new("input_bridge","target event delivery failed")); self.flow.freeze(); }
    }
    fn engine_id(&self) -> Uuid {
        self.id()
    }
    fn emit_font_delta(&self, delta: i32) {
        let _ = self.app.emit_to(
            tauri::EventTarget::webview_window("main"),
            "aerotab:native-font-delta",
            serde_json::json!({
                "sessionId": self.id(),
                "generation": self.generation,
                "delta": delta,
            }),
        );
    }
    fn key(&mut self, w: usize) {
        let down = |vk: u16| unsafe { GetKeyState(vk as i32) < 0 };
        let m = input::Modifiers {
            shift: down(VK_SHIFT),
            alt: down(VK_MENU),
            control: down(VK_CONTROL),
            super_key: down(VK_LWIN) || down(VK_RWIN),
            level_three: down(VK_RMENU) && down(VK_CONTROL),
        };
        let mut key = match w as u16 {
            VK_RETURN => input::Key::Enter,
            VK_BACK => input::Key::Backspace,
            VK_TAB => input::Key::Tab,
            VK_ESCAPE => input::Key::Escape,
            VK_UP => input::Key::Up,
            VK_DOWN => input::Key::Down,
            VK_LEFT => input::Key::Left,
            VK_RIGHT => input::Key::Right,
            VK_HOME => input::Key::Home,
            VK_END => input::Key::End,
            VK_INSERT => input::Key::Insert,
            VK_DELETE => input::Key::Delete,
            VK_PRIOR => input::Key::PageUp,
            VK_NEXT => input::Key::PageDown,
            // OEM/+/- and numpad are outside ASCII VK range but drive font zoom chords.
            VK_OEM_PLUS | VK_ADD => input::Key::Character('='),
            VK_OEM_MINUS | VK_SUBTRACT => input::Key::Character('-'),
            v if (VK_F1..=VK_F24).contains(&v) => input::Key::Function((v - VK_F1 + 1) as u8),
            VK_SPACE => input::Key::Character(' '),
            _ => input::Key::Unknown,
        };
        // Translate layout text without Ctrl/Alt (which produce control codes); retain
        // Shift/CapsLock. Flag 4 prevents changing dead-key composition state.
        if !m.level_three && (key == input::Key::Unknown || matches!(key, input::Key::Character(_)))
        {
            let mut state = [0u8; 256];
            let mut text = [0u16; 8];
            unsafe {
                GetKeyboardState(state.as_mut_ptr());
                for vk in [
                    VK_CONTROL,
                    VK_LCONTROL,
                    VK_RCONTROL,
                    VK_MENU,
                    VK_LMENU,
                    VK_RMENU,
                    VK_LWIN,
                    VK_RWIN,
                ] {
                    state[vk as usize] = 0;
                }
                let n = ToUnicode(
                    w as u32,
                    MapVirtualKeyW(w as u32, MAPVK_VK_TO_VSC),
                    state.as_ptr(),
                    text.as_mut_ptr(),
                    text.len() as i32,
                    4,
                );
                if n > 0 {
                    key = String::from_utf16_lossy(&text[..n as usize])
                        .chars()
                        .next()
                        .map_or(key, input::Key::Character);
                }
            }
        }
        self.suppress_char = false;
        if !self.active() || (self.composing && !input::shortcut_before_ime(m, self.composing)) {
            return;
        }
        if !m.level_three
            && key != input::Key::Unknown
            && (m.control || m.alt || m.super_key || !matches!(key, input::Key::Character(_)))
        {
            self.suppress_char = true;
            if self.shortcuts.len() >= 256 {
                self.flow.freeze();
                self.error = Some(HostError::new(
                    "shortcut_overflow",
                    "native key queue exceeded",
                ));
                return;
            }
            self.shortcut_seq = self.shortcut_seq.wrapping_add(1);
            self.shortcuts.push_back((self.shortcut_seq, key, m, None));
            if self.shortcuts.len() == 1 {
                unsafe {
                    PostMessageW(self.hwnd, WM_APP + 1, 0, 0);
                }
            }
            return;
        }
        self.local_key(key, m);
    }
    fn local_key(&mut self, key: input::Key, m: input::Modifiers) {
        if !self.active() {
            return;
        }
        if let Some(action) = input::clipboard_action(key, m, false) {
            // Clipboard text remains behind the production confirmation/bracketed-paste bridge.
            if action == input::ClipboardAction::Copy {
                if let Some(text) = self.engine.selection_text() {
                    let _ = clipboard_copy(self.hwnd, &text);
                }
            }
            if action == input::ClipboardAction::Paste {
                if let Ok(text) = clipboard_text(self.hwnd) {
                    if !input::clipboard_callback_valid(
                        0,
                        0,
                        self.lifetime.matches(self.generation, self.hwnd as usize),
                        self.visible,
                        unsafe { IsWindowVisible(self.hwnd) != 0 },
                        unsafe { GetFocus() == self.hwnd },
                    ) {
                        return;
                    }
                    self.paste_request = self.paste_request.saturating_add(1);
                    let _=self.app.emit_to(tauri::EventTarget::webview_window("main"),"aerotab:native-paste-request",serde_json::json!({"sessionId":self.id(),"generation":self.generation,"requestId":self.paste_request,"text":text}));
                }
            }
            return;
        }
        if let Some(text) = input::encode(
            key,
            m,
            self.engine.mode_bits() & alacritty_terminal::term::TermMode::APP_CURSOR.bits() != 0,
        ) {
            self.bytes(text.as_bytes());
        }
    }
    fn mouse(&mut self, msg: u32, w: WPARAM, l: LPARAM) {
        if !self.active() {
            return;
        }
        let mode = alacritty_terminal::term::TermMode::from_bits_retain(self.engine.mode_bits());
        let mode = input::MouseMode {
            click: mode.contains(alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK),
            drag: mode.contains(alacritty_terminal::term::TermMode::MOUSE_DRAG),
            motion: mode.contains(alacritty_terminal::term::TermMode::MOUSE_MOTION),
            sgr: mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE),
            utf8: mode.contains(alacritty_terminal::term::TermMode::UTF8_MOUSE),
        };
        let m = input::Modifiers {
            shift: w & 4 != 0,
            control: w & 8 != 0,
            alt: unsafe { GetKeyState(VK_MENU as i32) < 0 },
            ..input::Modifiers::default()
        };
        let mut point = POINT {
            x: l as u16 as i16 as i32,
            y: (l >> 16) as u16 as i16 as i32,
        };
        if msg == WM_MOUSEWHEEL || msg == WM_MOUSEHWHEEL {
            unsafe {
                windows_sys::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut point);
            }
        }
        let (cols, rows) = self.engine.dims();
        let pos = input::cell_position(
            point.x as f64 / self.scale,
            point.y as f64 / self.scale,
            self.cell.0,
            self.cell.1,
            cols,
            rows,
        );
        let reporting = mode.active() && !m.shift;
        let selection = |dispatch: &InputDispatch, id: Uuid, start: bool| {
            let mut p = serde_json::json!({"id":id,"col":pos.col,"line":pos.row,"side":if start{"left"}else{"right"}});
            if start {
                p["kind"] = serde_json::json!("simple");
            }
            dispatch(
                if start {
                    "terminal.selectionStart"
                } else {
                    "terminal.selectionUpdate"
                },
                p,
            );
        };
        if msg == WM_MOUSEWHEEL || msg == WM_MOUSEHWHEEL {
            let delta = -((w >> 16) as u16 as i16) as f64 / 120.;
            let steps = self.wheel.steps(delta, reporting);
            if reporting {
                let button = if msg == WM_MOUSEHWHEEL {
                    if steps < 0 {
                        67
                    } else {
                        66
                    }
                } else if steps < 0 {
                    64
                } else {
                    65
                };
                for _ in 0..steps.unsigned_abs() {
                    if let Some(bytes) =
                        input::encode_mouse(mode, input::MouseAction::Wheel(button), pos, m)
                    {
                        self.bytes(&bytes);
                    }
                }
            } else if msg == WM_MOUSEWHEEL && steps != 0 && m.control {
                // HWND owns the wheel while native-surface is focused; bridge app font zoom.
                self.emit_font_delta(if steps < 0 { 1 } else { -1 });
            } else if msg == WM_MOUSEWHEEL && steps != 0 && !m.control {
                (self.dispatch)(
                    "terminal.scroll",
                    serde_json::json!({"id":self.id(),"cmd":{"delta":-steps*3}}),
                );
            }
            return;
        }
        let action = match msg {
            WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN => {
                let b = if msg == WM_LBUTTONDOWN {
                    0
                } else if msg == WM_MBUTTONDOWN {
                    1
                } else {
                    2
                };
                if self.pointer.press(b, reporting) {
                    selection(&self.dispatch, self.id(), true);
                }
                if reporting {
                    Some(input::MouseAction::Press(b))
                } else {
                    None
                }
            }
            WM_LBUTTONUP | WM_MBUTTONUP | WM_RBUTTONUP => {
                let b = if msg == WM_LBUTTONUP {
                    0
                } else if msg == WM_MBUTTONUP {
                    1
                } else {
                    2
                };
                let (local, reported) = self.pointer.release(b);
                if local {
                    selection(&self.dispatch, self.id(), false);
                }
                if reported {
                    Some(input::MouseAction::Release(b))
                } else {
                    None
                }
            }
            WM_MOUSEMOVE => {
                if self.pointer.selecting(w & 1 != 0) {
                    selection(&self.dispatch, self.id(), false);
                    None
                } else if reporting {
                    Some(input::MouseAction::Motion(if w & 1 != 0 {
                        Some(0)
                    } else if w & 16 != 0 {
                        Some(1)
                    } else if w & 2 != 0 {
                        Some(2)
                    } else {
                        None
                    }))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(action) = action {
            if let Some(bytes) = input::encode_mouse(mode, action, pos, m) {
                self.bytes(&bytes);
            }
        }
    }
    fn id(&self) -> Uuid {
        self.session_id
    }
    fn draw(&mut self) -> Result<(), HostError> {
        if !self.visible || unsafe { IsWindowVisible(self.hwnd) == 0 || IsIconic(self.parent) != 0 }
        {
            return Ok(());
        }
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(self.parent) };
        let scale = if dpi == 0 {
            self.scale
        } else {
            dpi as f64 / 96.
        };
        if scale != self.scale {
            self.scale = scale;
            self.place()?;
        }
        #[cfg(debug_assertions)]
        let mut perf = super::windows_renderer::PerfTrace::begin("host", 0);
        let frame = self.engine.native_snapshot_envelope();
        #[cfg(debug_assertions)]
        super::windows_renderer::PerfTrace::lap(&mut perf, "snapshot_lock_serialize");
        let snapshot = Snapshot::decode(&frame.bytes, &self.cfg)?;
        #[cfg(debug_assertions)]
        super::windows_renderer::PerfTrace::lap(&mut perf, "snapshot_decode");
        let p = windows_policy::physical(self.geometry, self.scale)?;
        let mut actual: RECT = unsafe { std::mem::zeroed() };
        if unsafe { GetClientRect(self.hwnd, &mut actual) } == 0
            || actual.right - actual.left != p[2]
            || actual.bottom - actual.top != p[3]
        {
            return Ok(());
        }
        if self.renderer.as_mut().unwrap().draw(
            &snapshot,
            &self.cfg,
            [p[2] as u32, p[3] as u32],
            self.scale,
            self.cell,
        )? && self.revision.draw_complete(self.revision.applied)
        {
            self.last_seq = Some(snapshot.seq);
            self.drawn_output = Some(frame.output_seq);
        }
        #[cfg(debug_assertions)]
        super::windows_renderer::PerfTrace::lap(&mut perf, "renderer_and_publish");
        Ok(())
    }
    fn place(&self) -> Result<(), HostError> {
        let p = windows_policy::physical(self.geometry, self.scale)?;
        let ok = unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOP,
                p[0],
                p[1],
                p[2],
                p[3],
                SWP_NOACTIVATE
                    | if self.visible {
                        SWP_SHOWWINDOW
                    } else {
                        SWP_HIDEWINDOW
                    },
            )
        };
        if ok == 0 {
            return Err(HostError::new(
                "hwnd_geometry",
                std::io::Error::last_os_error().to_string(),
            ));
        }
        Ok(())
    }
    fn resize(&mut self) {
        self.cell = self.renderer.as_mut().unwrap().metrics(&self.cfg);
        let (cols, rows) = grid_size(
            self.geometry.width.round() as i32,
            self.geometry.height.round() as i32,
            self.cell.0,
            self.cell.1,
        );
        let grid = (cols, rows, self.cell.0, self.cell.1);
        if self.grid == Some(grid) {
            return;
        }
        self.grid = Some(grid);
        self.engine.resize(cols, rows, self.cell.0, self.cell.1);
        (self.dispatch)(
            "session.resize",
            serde_json::json!({"id":self.id(),"cols":cols,"rows":rows}),
        );
    }
    fn state(&self) -> ViewState {
        ViewState {
            session_id: self.id(),
            generation: self.generation,
            geometry: self.geometry,
            visible: self.visible,
            focused: self.active(),
            last_draw_seq: self.last_seq,
            revision: self.revision.applied,
            drawn_revision: self.revision.drawn,
            handoff_output_seq: self.handoff.to_string(),
            drawn_output_seq: self.drawn_output.map(|s| s.to_string()),
            input_accepted: self.flow.seq,
            input_ack: self.flow.ack,
            input_frozen: self.flow.frozen,
            error: self.error.clone(),
            backend: "win32-dx12-glyphon",
        }
    }
}
fn clipboard_copy(hwnd: HWND, text: &str) -> Result<(), HostError> {
    use ::windows::Win32::{
        Foundation::{GlobalFree, HGLOBAL, HWND as WinHwnd},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
    };
    unsafe {
        OpenClipboard(WinHwnd(hwnd.cast()))
            .map_err(|e| HostError::new("clipboard", e.to_string()))?;
        let result = (|| {
            let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
            let h = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2)
                .map_err(|e| HostError::new("clipboard", e.to_string()))?;
            let p = GlobalLock(h) as *mut u16;
            if p.is_null() {
                let _ = GlobalFree(h);
                return Err(HostError::new("clipboard", "allocation lock failed"));
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), p, units.len());
            let _ = GlobalUnlock(h);
            if let Err(e) = EmptyClipboard() {
                let _ = GlobalFree(h);
                return Err(HostError::new("clipboard", e.to_string()));
            }
            if let Err(e) = SetClipboardData(13, ::windows::Win32::Foundation::HANDLE(h.0)) {
                let _ = GlobalFree(HGLOBAL(h.0));
                return Err(HostError::new("clipboard", e.to_string()));
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}
fn clipboard_text(hwnd: HWND) -> Result<String, HostError> {
    use ::windows::Win32::{
        Foundation::{HGLOBAL, HWND as WinHwnd},
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        },
    };
    unsafe {
        OpenClipboard(WinHwnd(hwnd.cast()))
            .map_err(|e| HostError::new("clipboard", e.to_string()))?;
        let result = (|| {
            let h = GetClipboardData(13).map_err(|e| HostError::new("clipboard", e.to_string()))?;
            let h = HGLOBAL(h.0);
            let size = GlobalSize(h);
            if !(2..=131072).contains(&size) || size % 2 != 0 {
                return Err(HostError::new(
                    "clipboard",
                    "clipboard text exceeds native bridge limit or is malformed",
                ));
            }
            let p = GlobalLock(h) as *const u16;
            if p.is_null() {
                return Err(HostError::new("clipboard", "clipboard lock failed"));
            }
            let units = std::slice::from_raw_parts(p, size / 2);
            let n = units.iter().position(|u| *u == 0).unwrap_or(units.len());
            let text = String::from_utf16_lossy(&units[..n]);
            let _ = GlobalUnlock(h);
            Ok(text)
        })();
        let _ = CloseClipboard();
        result
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let c = unsafe { &*(l as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, c.lpCreateParams as isize);
        }
    }
    let callback = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Callback;
    if callback.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    }
    if msg == WM_NCDESTROY {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        }
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    }
    if msg == WM_LBUTTONDOWN {
        unsafe {
            SetFocus(hwnd);
        }
    }
    let identity = unsafe { &*callback };
    if msg == WM_APP + 1 {
        route_key(identity.id, identity.generation);
        return 0;
    }
    VIEWS.with(|manager| {
        // Reentrant Win32 notifications must not create a second mutable reference.
        let Ok(mut manager) = manager.try_borrow_mut() else {
            return unsafe { DefWindowProcW(hwnd, msg, w, l) };
        };
        let Some(view) = manager.get_mut(&identity.id) else {
            return unsafe { DefWindowProcW(hwnd, msg, w, l) };
        };
        if !view.lifetime.matches(identity.generation, hwnd as usize) {
            return 0;
        }
        let p = &mut **view as *mut Attachment;
        match msg {
            WM_ERASEBKGND => 1,
            WM_GETDLGCODE => (DLGC_WANTALLKEYS | DLGC_WANTCHARS | DLGC_WANTARROWS) as isize,
            WM_NCDESTROY => unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, w, l)
            },
            WM_TIMER => {
                let v = unsafe { &mut *p };
                if v.shortcut_started
                    .is_some_and(|t| t.elapsed().as_secs() >= 2)
                {
                    v.shortcuts.clear();
                    v.shortcut_started = None;
                    v.flow.freeze();
                    v.error = Some(HostError::new(
                        "shortcut_timeout",
                        "shortcut resolver timed out",
                    ));
                }
                if let Err(e) = v.draw() {
                    v.error = Some(e);
                }
                0
            }
            WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_RBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONUP
            | WM_RBUTTONUP | WM_MOUSEMOVE | WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                unsafe { &mut *p }.mouse(msg, w, l);
                0
            }
            WM_SETFOCUS => {
                let v = unsafe { &mut *p };
                if v.active() {
                    let seq = ACTIVATION_SEQ.with(|seq| {
                        let next = seq.get() + 1;
                        seq.set(next);
                        next
                    });
                    let _ = v.app.emit_to(
                        tauri::EventTarget::webview_window("main"),
                        "aerotab:native-activation",
                        serde_json::json!({"sessionId":v.id(),"generation":v.generation,"seq":seq}),
                    );
                }
                0
            }
            WM_KILLFOCUS => {
                let v = unsafe { &mut *p };
                v.text.reset();
                v.composing = false;
                v.shortcuts.clear();
                v.shortcut_started = None;
                0
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                unsafe { &mut *p }.key(w);
                0
            }
            WM_CHAR | WM_SYSCHAR => {
                let v = unsafe { &mut *p };
                let active = v.active() && !v.suppress_char;
                if let Some(mut text) = v.text.character(w as u16, active, v.composing) {
                    if msg == WM_SYSCHAR {
                        text.insert(0, '\x1b');
                    }
                    // WM_CHAR owns ordinary layout/dead-key Unicode. Keep it in
                    // the same FIFO so application shortcuts cannot overtake text.
                    if v.shortcuts.len() >= 256 {
                        v.flow.freeze();
                        v.error = Some(HostError::new(
                            "shortcut_overflow",
                            "native key queue exceeded",
                        ));
                        return 0;
                    }
                    v.shortcut_seq = v.shortcut_seq.wrapping_add(1);
                    let key = text
                        .chars()
                        .next()
                        .map_or(input::Key::Unknown, input::Key::Character);
                    v.shortcuts.push_back((
                        v.shortcut_seq,
                        key,
                        input::Modifiers::default(),
                        Some(text),
                    ));
                    if v.shortcuts.len() == 1 {
                        unsafe {
                            PostMessageW(v.hwnd, WM_APP + 1, 0, 0);
                        }
                    }
                }
                0
            }
            WM_UNICHAR => {
                if unsafe { &*p }.composing {
                    return 0;
                }
                if w == 0xffff {
                    return 1;
                }
                let v = unsafe { &mut *p };
                if let Some(c) = char::from_u32(w as u32) {
                    v.bytes(c.to_string().as_bytes());
                }
                0
            }
            WM_IME_STARTCOMPOSITION => {
                unsafe { &mut *p }.composing = true;
                0
            }
            WM_IME_ENDCOMPOSITION => {
                unsafe { &mut *p }.composing = false;
                0
            }
            WM_IME_CHAR => 0,
            WM_IME_COMPOSITION => {
                let v = unsafe { &mut *p };
                if !v.active() {
                    return 0;
                }
                let context = unsafe { ImmGetContext(hwnd) };
                if !context.is_null() {
                    let snapshot = v.engine.native_snapshot_envelope();
                    if let Ok(snapshot) = Snapshot::decode(&snapshot.bytes, &v.cfg) {
                        let point = POINT {
                            x: ((12. + snapshot.cursor.0 as f64 * v.cell.0 as f64) * v.scale)
                                .round() as i32,
                            y: ((12. + (snapshot.cursor.1 + 1) as f64 * v.cell.1 as f64) * v.scale)
                                .round() as i32,
                        };
                        let candidate = CANDIDATEFORM {
                            dwIndex: 0,
                            dwStyle: CFS_CANDIDATEPOS,
                            ptCurrentPos: point,
                            rcArea: unsafe { std::mem::zeroed() },
                        };
                        let composition = COMPOSITIONFORM {
                            dwStyle: CFS_POINT,
                            ptCurrentPos: point,
                            rcArea: unsafe { std::mem::zeroed() },
                        };
                        unsafe {
                            ImmSetCandidateWindow(context, &candidate);
                            ImmSetCompositionWindow(context, &composition);
                        }
                    }
                    if l as u32 & GCS_RESULTSTR != 0 {
                        let n = unsafe {
                            ImmGetCompositionStringW(
                                context,
                                GCS_RESULTSTR,
                                std::ptr::null_mut(),
                                0,
                            )
                        };
                        if n > 0 && n <= 65536 {
                            let mut units = vec![0u16; n as usize / 2];
                            unsafe {
                                ImmGetCompositionStringW(
                                    context,
                                    GCS_RESULTSTR,
                                    units.as_mut_ptr().cast(),
                                    n as u32,
                                );
                            }
                            v.bytes(String::from_utf16_lossy(&units).as_bytes());
                        }
                    }
                    unsafe {
                        ImmReleaseContext(hwnd, context);
                    }
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
        }
    })
}
pub fn attach(
    window: &tauri::WebviewWindow,
    id: Uuid,
    geometry: Geometry,
    cfg: ViewConfig,
    engine: Arc<TerminalEngine>,
    dispatch: InputDispatch,
) -> Result<ViewState, HostError> {
    if window.label() != "main" {
        return Err(HostError::new("permission_denied", "main window required"));
    }
    if VIEWS.with(|m| m.borrow().contains_key(&id)) {
        return Err(HostError::new("already_attached", "detach old view first"));
    }
    let parent = window
        .hwnd()
        .map_err(|e| HostError::new("hwnd", e.to_string()))?
        .0 as HWND;
    let mut pid = 0;
    let thread = unsafe { GetWindowThreadProcessId(parent, &mut pid) };
    if thread != unsafe { GetCurrentThreadId() } {
        return Err(HostError::new("wrong_thread", "HWND owner thread required"));
    }
    let generation = Uuid::new_v4();
    let handoff = engine.native_snapshot_envelope().output_seq;
    let mut v = Box::new(Attachment {
        callback: Box::new(Callback { id, generation }),
        session_id: id,
        hwnd: std::ptr::null_mut(),
        parent,
        thread,
        generation,
        lifetime: input::AttachmentLifetime::new(generation, 0),
        engine,
        dispatch,
        app: window.app_handle().clone(),
        geometry,
        cfg,
        scale: window
            .scale_factor()
            .map_err(|e| HostError::new("dpi", e.to_string()))?,
        visible: true,
        flow: input::InputFlow::default(),
        revision: input::ConfigRevision::default(),
        handoff,
        drawn_output: None,
        last_seq: None,
        error: None,
        renderer: None,
        text: windows_policy::TextInput::default(),
        composing: false,
        suppress_char: false,
        shortcuts: Default::default(),
        shortcut_seq: 0,
        shortcut_started: None,
        paste_request: 0,
        cell: (1, 1),
        grid: None,
        pointer: input::PointerState::default(),
        wheel: input::WheelState::default(),
    });
    let name: Vec<u16> = "AeroTabNativeTerminal\0".encode_utf16().collect();
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let class = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: module,
        lpszClassName: name.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe {
        RegisterClassW(&class);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            name.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP,
            0,
            0,
            1,
            1,
            parent,
            std::ptr::null_mut(),
            module,
            (&mut *v.callback as *mut Callback).cast(),
        )
    };
    if hwnd.is_null() {
        return Err(HostError::new(
            "hwnd_create",
            std::io::Error::last_os_error().to_string(),
        ));
    }
    v.hwnd = hwnd;
    v.lifetime = input::AttachmentLifetime::new(generation, hwnd as usize);
    let p = windows_policy::physical(geometry, v.scale)?;
    match Renderer::new(hwnd as isize, p[2] as u32, p[3] as u32) {
        Ok(r) => v.renderer = Some(r),
        Err(e) => return Err(e),
    }
    v.resize();
    v.place()?;
    unsafe {
        EnableWindow(hwnd, 0);
        SetTimer(hwnd, 1, 33, None);
    }
    let state = v.state();
    VIEWS.with(|m| m.borrow_mut().insert(id, v));
    Ok(state)
}
/// Synthetic OS messages, not physical keyboard input. Uses the production WndProc
/// and its focus/visibility/readiness/generation gates, never the writer directly.
#[cfg(debug_assertions)]
pub fn debug_messages(id: Uuid, generation: Uuid, text: &str) -> Result<ViewState, HostError> {
    if std::env::var("AEROTAB_WINDOWS_NATIVE_UI_E2E").as_deref() != Ok("1") {
        return Err(HostError::new(
            "permission_denied",
            "Windows E2E is disabled",
        ));
    }
    let hwnd = VIEWS.with(|m| {
        let m = m.borrow();
        let v = m.get(&id).ok_or_else(missing)?;
        if v.generation != generation {
            return Err(HostError::new("stale_generation", "attachment replaced"));
        }
        Ok(v.hwnd)
    })?;
    // Release the manager borrow before synchronous WndProc re-entry.
    for unit in text.encode_utf16() {
        unsafe {
            SendMessageW(
                hwnd,
                WM_KEYDOWN,
                if unit == 13 { VK_RETURN as usize } else { 0 },
                0,
            );
            SendMessageW(hwnd, WM_CHAR, unit as usize, 0);
        }
    }
    state(id)
}
#[cfg(debug_assertions)]
pub fn debug_state(id: Uuid) -> Result<serde_json::Value, HostError> {
    VIEWS.with(|m| {let m=m.borrow();let v=m.get(&id).ok_or_else(missing)?;
        Ok(serde_json::json!({"host":v.state(),"osVisible":unsafe{IsWindowVisible(v.hwnd)!=0},"hwnd":v.hwnd as usize}))
    })
}
pub fn state(id: Uuid) -> Result<ViewState, HostError> {
    VIEWS.with(|m| m.borrow().get(&id).map(|v| v.state()).ok_or_else(missing))
}
pub fn focus(id: Uuid) -> Result<(), HostError> {
    let hwnd = VIEWS.with(|m| {
        let m = m.borrow();
        let v = m.get(&id).ok_or_else(missing)?;
        if !v.visible || !v.flow.ready || v.error.is_some() {
            return Err(HostError::new(
                "input_not_ready",
                "inactive or frozen attachment",
            ));
        }
        Ok(v.hwnd)
    })?;
    unsafe {
        SetFocus(hwnd);
    }
    Ok(())
}
pub fn update(
    id: Uuid,
    geometry: Geometry,
    visible: bool,
    cfg: Option<ViewConfig>,
) -> Result<ViewState, HostError> {
    VIEWS.with(|m| {
        let mut m = m.borrow_mut();
        let v = m.get_mut(&id).ok_or_else(missing)?;
        if v.thread != unsafe { GetCurrentThreadId() } {
            return Err(HostError::new("wrong_thread", "HWND owner thread required"));
        }
        let cfg =
            cfg.ok_or_else(|| HostError::new("config_revision", "epoch and revision required"))?;
        if cfg.input_generation == Some(v.generation) && cfg.input_ack > v.flow.seq {
            return Err(HostError::new("input_ack", "invalid input acknowledgement"));
        }
        v.revision
            .apply(
                v.generation.as_u128(),
                cfg.expected_generation.map(|g| g.as_u128()),
                cfg.revision,
            )
            .map_err(|e| HostError::new("config_revision", e))?;
        if cfg.input_generation == Some(v.generation) {
            if cfg.input_frozen {
                v.flow.freeze();
            }
            v.flow.acknowledge(cfg.input_ready, cfg.input_ack);
        }
        v.cfg = cfg;
        v.geometry = geometry;
        v.visible = visible;
        if !visible {
            v.text.reset();
            v.composing = false;
        }
        v.resize();
        v.place()?;
        unsafe {
            EnableWindow(
                v.hwnd,
                i32::from(visible && v.flow.ready && v.error.is_none()),
            );
        }
        Ok(v.state())
    })
}
pub fn detach(id: Uuid) -> Result<(), HostError> {
    let mut v = VIEWS
        .with(|m| m.borrow_mut().remove(&id))
        .ok_or_else(missing)?;
    v.lifetime.invalidate();
    v.flow.freeze();
    drop(v);
    Ok(())
}
