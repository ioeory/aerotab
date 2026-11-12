use super::input::{self, Key, Modifiers};
use super::*;
use crate::terminal::engine::TerminalEngine;
use alacritty_terminal::term::TermMode;
use gtk::prelude::*;
use std::{
    cell::{Cell as StateCell, RefCell},
    rc::Rc,
    sync::Arc,
};
use tauri::Manager as _;
#[derive(Clone)]
struct CallbackScope {
    id: Uuid,
    lifetime: input::AttachmentLifetime<Uuid, usize>,
    area: glib::WeakRef<gtk::GLArea>,
    key_epoch: Rc<StateCell<u64>>,
}
impl CallbackScope {
    fn matches(&self, view: &Attachment, area: &gtk::GLArea) -> bool {
        self.lifetime
            .matches(view.generation, area.as_ptr() as usize)
            && view.area == *area
    }
    fn current(&self, area: &gtk::GLArea) -> bool {
        MANAGER.with(|m| {
            m.try_borrow()
                .ok()
                .is_some_and(|m| m.views.get(&self.id).is_some_and(|v| self.matches(v, area)))
        })
    }
    fn shortcut_failure(&self, code: &str) {
        MANAGER.with(|m| {
            let mut m = m.borrow_mut();
            if let Some(v) = m.views.get_mut(&self.id) {
                if self.area.upgrade().is_some_and(|a| self.matches(v, &a)) {
                    v.input_flow.freeze();
                    v.error = Some(HostError::new(code, "native shortcut resolver unavailable"));
                }
            }
        });
    }
    fn live(&self) -> bool {
        self.area.upgrade().is_some_and(|area| self.current(&area))
    }
    fn input(&self, area: &gtk::GLArea) -> bool {
        MANAGER.with(|m| {
            m.try_borrow().ok().is_some_and(|m| {
                m.views.get(&self.id).is_some_and(|v| {
                    self.matches(v, area) && v.visible && v.input_flow.ready && v.error.is_none()
                })
            })
        })
    }
}
struct Attachment {
    area: gtk::GLArea,
    scope: CallbackScope,
    timer: Option<glib::SourceId>,
    im: gtk::IMMulticontext,
    generation: Uuid,
    clipboard_epoch: u64,
    input_flow: input::InputFlow,
    #[cfg(debug_assertions)]
    debug_ack: u32,
    app: tauri::AppHandle,
    geometry: Geometry,
    visible: bool,
    _engine: Arc<TerminalEngine>,
    cfg: Rc<RefCell<ViewConfig>>,
    dispatch: InputDispatch,
    grid: Option<(u16, u16, u16, u16)>,
    _renderer: Rc<RefCell<Option<super::renderer::Renderer>>>,
    last_seq: Option<u32>,
    revision: input::ConfigRevision,
    handoff_output_seq: u64,
    drawn_output_seq: Option<u64>,
    error: Option<HostError>,
}
#[derive(Default)]
struct Manager {
    overlay: Option<gtk::Overlay>,
    views: HashMap<Uuid, Attachment>,
}
thread_local! {static MANAGER:RefCell<Manager>=RefCell::default();}
fn main_thread() -> Result<(), HostError> {
    if !gtk::is_initialized_main_thread() {
        return Err(HostError::new(
            "wrong_thread",
            "native host requires GTK main thread",
        ));
    }
    Ok(())
}
fn write_input(scope: &CallbackScope, text: &str) {
    write_bytes(scope, text.as_bytes());
}

fn write_bytes(scope: &CallbackScope, bytes: &[u8]) {
    let id = scope.id;
    let Some(area) = scope.area.upgrade() else {
        return;
    };
    MANAGER.with(|m| {
        let mut manager = m.borrow_mut();
        let Some(view) = manager.views.get_mut(&id) else { return; };
        if !scope.matches(view, &area) || !view.visible || !view.input_flow.ready || view.error.is_some() { return; }
        let Some(seq) = view.input_flow.reserve(bytes.len()) else {
            view.error = Some(HostError::new("input_overflow", "native input bridge capacity exceeded"));
            view.area.set_can_focus(false);
            view.area.set_sensitive(false);
            return;
        };
        use tauri::Emitter;
        if view.app.emit_to(tauri::EventTarget::webview_window("main"), "aerotab:native-input", serde_json::json!({
            "sessionId": id, "generation": view.generation, "seq": seq,
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
        })).is_err() {
            view.error = Some(HostError::new("input_bridge", "native input event delivery failed"));
            view.area.set_can_focus(false);
            view.area.set_sensitive(false);
        }
    });
}

fn modifiers(state: gtk::gdk::ModifierType) -> Modifiers {
    use gtk::gdk::ModifierType;
    Modifiers {
        shift: state.contains(ModifierType::SHIFT_MASK),
        alt: state.contains(ModifierType::MOD1_MASK),
        control: state.contains(ModifierType::CONTROL_MASK),
        super_key: state.intersects(
            ModifierType::SUPER_MASK | ModifierType::META_MASK | ModifierType::HYPER_MASK,
        ),
        level_three: state.contains(ModifierType::MOD5_MASK),
    }
}

fn route_key(
    scope: CallbackScope,
    window: tauri::WebviewWindow,
    queue: Rc<RefCell<std::collections::VecDeque<gtk::gdk::EventKey>>>,
    replay: Rc<StateCell<bool>>,
) {
    let Some(area) = scope.area.upgrade() else {
        return;
    };
    if !scope.input(&area) || !area.has_focus() || !area.is_mapped() {
        queue.borrow_mut().clear();
        return;
    }
    let Some(event) = queue.borrow().front().cloned() else {
        return;
    };
    let (key, m) = key_input(&event);
    let geometry =
        MANAGER.with(|manager| manager.borrow().views.get(&scope.id).map(|v| v.geometry));
    let Some(geometry) = geometry else {
        return;
    };
    let script = super::shortcut::shortcut_script(key, m, geometry, scope.id);
    let app = window.app_handle().clone();
    let next_window = window.clone();
    // GTK state is main-thread-only. eval callbacks may run on another thread;
    // deliver decisions through a local GLib channel-equivalent idle poll.
    let (tx, rx) = std::sync::mpsc::channel();
    if window
        .eval_with_callback(script, move |result| {
            let _ = tx.send(result);
        })
        .is_err()
    {
        queue.borrow_mut().clear();
        scope.shortcut_failure("shortcut_bridge");
        return;
    }
    let epoch = scope.key_epoch.get();
    let started = std::time::Instant::now();
    glib::timeout_add_local(std::time::Duration::from_millis(2), move || {
        let _ = &app;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) if started.elapsed().as_secs() < 2 => {
                return glib::ControlFlow::Continue
            }
            _ => {
                queue.borrow_mut().clear();
                scope.shortcut_failure("shortcut_timeout");
                return glib::ControlFlow::Break;
            }
        };
        queue.borrow_mut().pop_front();
        let valid = scope.key_epoch.get() == epoch
            && scope.input(&area)
            && area.has_focus()
            && area.is_mapped();
        if valid
            && serde_json::from_str::<serde_json::Value>(&result)
                .ok()
                .and_then(|v| v.get("consumed").and_then(|v| v.as_bool()))
                == Some(false)
        {
            replay.set(true);
            let _: bool = area.emit_by_name("key-press-event", &[&*event]);
            replay.set(false);
        }
        if !valid {
            queue.borrow_mut().clear();
        }
        route_key(
            scope.clone(),
            next_window.clone(),
            queue.clone(),
            replay.clone(),
        );
        glib::ControlFlow::Break
    });
}
fn key_input(event: &gtk::gdk::EventKey) -> (Key, Modifiers) {
    use gtk::gdk::keys::constants as keys;
    let modifiers = modifiers(event.state());
    let key = match event.keyval() {
        keys::Return | keys::KP_Enter | keys::ISO_Enter => Key::Enter,
        keys::BackSpace => Key::Backspace,
        keys::Tab | keys::ISO_Left_Tab | keys::KP_Tab => Key::Tab,
        keys::Escape => Key::Escape,
        keys::Up | keys::KP_Up => Key::Up,
        keys::Down | keys::KP_Down => Key::Down,
        keys::Right | keys::KP_Right => Key::Right,
        keys::Left | keys::KP_Left => Key::Left,
        keys::Home | keys::KP_Home => Key::Home,
        keys::End | keys::KP_End => Key::End,
        keys::Insert | keys::KP_Insert => Key::Insert,
        keys::Delete | keys::KP_Delete => Key::Delete,
        keys::Page_Up | keys::KP_Page_Up => Key::PageUp,
        keys::Page_Down | keys::KP_Page_Down => Key::PageDown,
        keys::F1 => Key::Function(1),
        keys::F2 => Key::Function(2),
        keys::F3 => Key::Function(3),
        keys::F4 => Key::Function(4),
        keys::F5 => Key::Function(5),
        keys::F6 => Key::Function(6),
        keys::F7 => Key::Function(7),
        keys::F8 => Key::Function(8),
        keys::F9 => Key::Function(9),
        keys::F10 => Key::Function(10),
        keys::F11 => Key::Function(11),
        keys::F12 => Key::Function(12),
        value if (keys::F13..=keys::F24).contains(&value) => {
            Key::Function((*value - *keys::F1 + 1) as u8)
        }
        value => value.to_unicode().map_or(Key::Unknown, Key::Character),
    };
    (key, modifiers)
}

fn mouse_mode(engine: &TerminalEngine) -> input::MouseMode {
    let mode = TermMode::from_bits_retain(engine.mode_bits());
    input::MouseMode {
        click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
        drag: mode.contains(TermMode::MOUSE_DRAG),
        motion: mode.contains(TermMode::MOUSE_MOTION),
        sgr: mode.contains(TermMode::SGR_MOUSE),
        utf8: mode.contains(TermMode::UTF8_MOUSE),
    }
}

fn pointer_cell(engine: &TerminalEngine, cfg: &ViewConfig, x: f64, y: f64) -> input::CellPosition {
    let (cols, rows) = engine.dims();
    let (cw, ch) = super::renderer::metrics(cfg);
    input::cell_position(x, y, cw, ch, cols, rows)
}

fn selection(dispatch: &InputDispatch, id: Uuid, pos: input::CellPosition, start: bool) {
    let method = if start {
        "terminal.selectionStart"
    } else {
        "terminal.selectionUpdate"
    };
    let mut params = serde_json::json!({
        "id": id, "col": pos.col, "line": pos.row,
        "side": if start { "left" } else { "right" },
    });
    if start {
        params["kind"] = serde_json::json!("simple");
    }
    dispatch(method, params);
}

fn held_button(state: gtk::gdk::ModifierType) -> Option<u8> {
    use gtk::gdk::ModifierType;
    [
        ModifierType::BUTTON1_MASK,
        ModifierType::BUTTON2_MASK,
        ModifierType::BUTTON3_MASK,
    ]
    .iter()
    .position(|mask| state.contains(*mask))
    .map(|b| b as u8)
}

fn apply(area: &gtk::GLArea, g: Geometry) {
    area.set_margin_start(g.x.round() as i32);
    area.set_margin_top(g.y.round() as i32);
    area.set_size_request(g.width.round() as i32, g.height.round() as i32);
}
pub fn attach(
    window: &tauri::WebviewWindow,
    id: Uuid,
    geometry: Geometry,
    cfg: ViewConfig,
    engine: Arc<TerminalEngine>,
    dispatch: InputDispatch,
) -> Result<ViewState, HostError> {
    main_thread()?;
    if window.label() != "main" {
        return Err(HostError::new(
            "permission_denied",
            "native views attach only to main",
        ));
    }
    if MANAGER.with(|m| m.borrow().views.contains_key(&id)) {
        return Err(HostError::new("already_attached", "detach old view first"));
    }
    let result = Arc::new(std::sync::Mutex::new(None));
    let output = result.clone();
    window
        .with_webview(move |platform| {
            let webview = platform.inner();
            let r = MANAGER.with(|m| {
                let mut m = m.borrow_mut();
                if m.overlay.is_some() {
                    return Ok(());
                }
                let parent = webview
                    .parent()
                    .and_then(|p| p.downcast::<gtk::Box>().ok())
                    .ok_or_else(|| {
                        HostError::new(
                            "unsupported_container",
                            "Wry WebView parent is not GtkBox; refusing unsafe reparent",
                        )
                    })?;
                let overlay = gtk::Overlay::new();
                overlay.set_hexpand(true);
                overlay.set_vexpand(true);
                parent.remove(&webview);
                parent.pack_start(&overlay, true, true, 0);
                overlay.add(&webview);
                overlay.show_all();
                m.overlay = Some(overlay);
                Ok(())
            });
            *output.lock().unwrap() = Some(r);
        })
        .map_err(|e| HostError::new("webview_access", e.to_string()))?;
    result.lock().unwrap().take().ok_or_else(|| {
        HostError::new(
            "main_thread",
            "with_webview did not execute synchronously on main",
        )
    })??;
    // Finite pre-attach canonical capture: later continuous output must not
    // turn readiness into an endlessly moving target.
    let handoff_output_seq = engine.native_snapshot_envelope().output_seq;
    let generation = Uuid::new_v4();
    let clipboard_request = Rc::new(StateCell::new(0u32));
    let area = gtk::GLArea::new();
    let scope = CallbackScope {
        id,
        lifetime: input::AttachmentLifetime::new(generation, area.as_ptr() as usize),
        area: area.downgrade(),
        key_epoch: Rc::new(StateCell::new(0)),
    };
    area.set_required_version(3, 2);
    area.set_has_depth_buffer(false);
    area.set_auto_render(false);
    area.set_halign(gtk::Align::Start);
    area.set_valign(gtk::Align::Start);
    area.set_can_focus(false); // Enabled only by generation-scoped listener handshake.
    area.set_sensitive(false);
    apply(&area, geometry);
    area.add_events(
        gtk::gdk::EventMask::BUTTON_PRESS_MASK
            | gtk::gdk::EventMask::BUTTON_RELEASE_MASK
            | gtk::gdk::EventMask::POINTER_MOTION_MASK
            | gtk::gdk::EventMask::SCROLL_MASK
            | gtk::gdk::EventMask::SMOOTH_SCROLL_MASK
            | gtk::gdk::EventMask::KEY_PRESS_MASK
            | gtk::gdk::EventMask::KEY_RELEASE_MASK,
    );
    let cfg = Rc::new(RefCell::new(cfg));
    let renderer = Rc::new(RefCell::new(None::<super::renderer::Renderer>));
    {
        let renderer = renderer.clone();
        let engine = engine.clone();
        let cfg = cfg.clone();
        let scope = scope.clone();
        area.connect_render(move |a, _| {
            if !scope.current(a) {
                return glib::Propagation::Stop;
            }
            if let Some(e) = a.error() {
                MANAGER.with(|m| {
                    if let Some(v) = m.borrow_mut().views.get_mut(&id) {
                        v.error = Some(HostError::new("gl_context", e.to_string()));
                    }
                });
                return glib::Propagation::Stop;
            }
            // Reading the native view must not consume the WebView's damage,
            // transfer events or raw output. seq is not a content revision.
            // GTK callbacks and updates are main-thread serialized: capture one
            // immutable config revision + config + canonical snapshot, with no await.
            let ticket = MANAGER.with(|m| {
                m.borrow()
                    .views
                    .get(&id)
                    .map(|v| (v.revision.applied, v.geometry))
            });
            let Some((ticket, geometry)) = ticket else {
                return glib::Propagation::Stop;
            };
            // A queued render may precede GTK allocation for the newest bounds.
            // Do not ACK that revision using the previous drawable dimensions.
            if a.allocated_width() != geometry.width.round() as i32
                || a.allocated_height() != geometry.height.round() as i32
            {
                return glib::Propagation::Stop;
            }
            let cfg = cfg.borrow().clone();
            let frame = engine.native_snapshot_envelope();
            match Snapshot::decode(&frame.bytes, &cfg) {
                Ok(snapshot) => {
                    let mut r = renderer.borrow_mut();
                    if r.is_none() {
                        *r = Some(unsafe { super::renderer::Renderer::new(cfg.clone()) });
                    }
                    unsafe {
                        r.as_mut().unwrap().draw(
                            &snapshot,
                            a.allocated_width(),
                            a.allocated_height(),
                            a.scale_factor(),
                            None,
                        );
                    }
                    MANAGER.with(|m| {
                        if let Some(v) = m.borrow_mut().views.get_mut(&id) {
                            if !scope.matches(v, a)
                                || a.error().is_some()
                                || !v.revision.draw_complete(ticket)
                            {
                                return;
                            }
                            v.last_seq = Some(snapshot.seq);
                            v.drawn_output_seq = Some(frame.output_seq);
                            if v.error
                                .as_ref()
                                .map_or(true, |error| !error.code.starts_with("input_"))
                            {
                                v.error = None;
                            }
                        }
                    });
                }
                Err(e) => {
                    MANAGER.with(|m| {
                        if let Some(v) = m.borrow_mut().views.get_mut(&id) {
                            v.error = Some(e);
                        }
                    });
                }
            }
            glib::Propagation::Stop
        });
    }
    {
        let renderer = renderer.clone();
        let scope = scope.clone();
        area.connect_unrealize(move |a| {
            if !scope.current(a) {
                return;
            }
            a.make_current();
            if let Some(r) = renderer.borrow_mut().take() {
                if a.error().is_none() {
                    unsafe {
                        r.destroy();
                    }
                }
            }
        });
    }
    let im = gtk::IMMulticontext::new();
    let composing = Rc::new(StateCell::new(false));
    {
        let composing = composing.clone();
        let scope = scope.clone();
        im.connect_preedit_start(move |_| {
            if scope.live() {
                composing.set(true);
            }
        });
    }
    {
        let composing = composing.clone();
        let scope = scope.clone();
        im.connect_preedit_end(move |_| {
            if scope.live() {
                composing.set(false);
            }
        });
    }
    {
        let im = im.clone();
        let scope = scope.clone();
        area.connect_realize(move |a| {
            if scope.current(a) {
                im.set_client_window(a.window().as_ref());
            }
        });
    }
    {
        let im = im.clone();
        let scope = scope.clone();
        area.connect_focus_in_event(move |a, _| {
            if !scope.input(a) {
                return glib::Propagation::Stop;
            }
            im.focus_in();
            #[cfg(debug_assertions)]
            if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1") {
                debug_record(scope.id, "actual-focus-in-event");
            }
            glib::Propagation::Proceed
        });
    }
    {
        let im = im.clone();
        let scope = scope.clone();
        area.connect_focus_out_event(move |a, _| {
            if !scope.current(a) {
                return glib::Propagation::Stop;
            }
            scope.key_epoch.set(scope.key_epoch.get().wrapping_add(1));
            im.focus_out();
            glib::Propagation::Proceed
        });
    }
    {
        let scope = scope.clone();
        im.connect_commit(move |_, text| write_input(&scope, text));
    }
    {
        let im = im.clone();
        let engine = engine.clone();
        let composing = composing.clone();
        let clipboard_request = clipboard_request.clone();
        let window = window.clone();
        let scope = scope.clone();
        let queue = Rc::new(RefCell::new(std::collections::VecDeque::new()));
        let replay = Rc::new(StateCell::new(false));
        area.connect_key_press_event(move |area, event| {
            if !scope.input(area) || !area.has_focus() || !area.is_mapped() {
                return glib::Propagation::Stop;
            }
            let (key, modifiers) = key_input(event);
            if !replay.get() && !composing.get() && !modifiers.level_three && key != Key::Unknown {
                if queue.borrow().len() >= 256 {
                    scope.shortcut_failure("shortcut_overflow");
                    return glib::Propagation::Stop;
                }
                queue.borrow_mut().push_back(event.clone());
                if queue.borrow().len() == 1 {
                    route_key(scope.clone(), window.clone(), queue.clone(), replay.clone());
                }
                return glib::Propagation::Stop;
            }
            if let Some(action) = input::clipboard_action(key, modifiers, composing.get()) {
                let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
                match action {
                    input::ClipboardAction::Copy => {
                        if let Some(text) = engine.selection_text() {
                            clipboard.set_text(&text);
                        }
                    }
                    input::ClipboardAction::Paste => {
                        let request_id = clipboard_request.get().saturating_add(1);
                        clipboard_request.set(request_id);
                        let weak_area = area.downgrade();
                        let app = window.app_handle().clone();
                        let epoch =
                            MANAGER.with(|m| m.borrow().views.get(&id).map(|v| v.clipboard_epoch));
                        let scope = scope.clone();
                        clipboard.request_text(move |_, text| {
                            let Some(area) = weak_area.upgrade() else {
                                return;
                            };
                            if !scope.input(&area) {
                                return;
                            }
                            let valid = MANAGER.with(|m| {
                                m.borrow().views.get(&id).is_some_and(|v| {
                                    epoch.is_some_and(|epoch| {
                                        input::clipboard_callback_valid(
                                            epoch,
                                            v.clipboard_epoch,
                                            v.generation == generation && v.area == area,
                                            v.visible,
                                            area.is_mapped(),
                                            area.has_focus(),
                                        )
                                    })
                                })
                            });
                            if !valid || !area.is_mapped() || !area.has_focus() {
                                return;
                            }
                            let Some(text) = text.filter(|text| !text.is_empty()) else {
                                return;
                            };
                            use tauri::Emitter;
                            // Target only main. No clipboard text in diagnostics/logs.
                            let _ = app.emit_to(
                                tauri::EventTarget::webview_window("main"),
                                "aerotab:native-paste-request",
                                serde_json::json!({
                                    "sessionId": id, "generation": generation,
                                    "requestId": request_id, "text": text,
                                }),
                            );
                        });
                    }
                }
                return glib::Propagation::Stop;
            }
            let ime_filtered = composing.get();
            if ime_filtered && im.filter_keypress(event) {
                return glib::Propagation::Stop;
            }
            let app_cursor = engine.mode_bits() & TermMode::APP_CURSOR.bits() != 0;
            let encoded = input::encode(key, modifiers, app_cursor);
            // Explicit Ctrl/Alt terminal chords bypass IM's simple text commit;
            // otherwise Alt+x could lose its ESC and Ctrl+C could become "c".
            if input::shortcut_before_ime(modifiers, composing.get()) {
                if let Some(text) = encoded.as_deref() {
                    write_input(&scope, text);
                    return glib::Propagation::Stop;
                }
            }
            // IM must see ordinary keys first, including arrows/Return used to
            // select a preedit candidate, and dead/unknown keys we cannot encode.
            if !ime_filtered && im.filter_keypress(event) {
                return glib::Propagation::Stop;
            }
            if let Some(text) = encoded.as_deref() {
                write_input(&scope, text);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    {
        let im = im.clone();
        let scope = scope.clone();
        area.connect_key_release_event(move |area, event| {
            if !scope.input(area) {
                return glib::Propagation::Stop;
            }
            // Some IM modules finish composition on release. Never emit PTY
            // bytes from releases (including autorepeat release/press pairs).
            if im.filter_keypress(event) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    let pointer = Rc::new(RefCell::new(input::PointerState::default()));
    {
        let dispatch = dispatch.clone();
        let cfg = cfg.clone();
        let engine = engine.clone();
        let vertical = RefCell::new(input::WheelState::default());
        let horizontal = RefCell::new(input::WheelState::default());
        let scope = scope.clone();
        area.connect_scroll_event(move |area, event| {
            if !scope.input(area) {
                return glib::Propagation::Stop;
            }
            use gtk::gdk::ScrollDirection;
            let m = modifiers(event.state());
            let mode = mouse_mode(&engine);
            let reporting = mode.active() && !m.shift;
            // Outside tracking, zoom/system chords need the app bridge.
            if !reporting && (m.control || m.super_key) {
                return glib::Propagation::Proceed;
            }
            let (dx, dy) = match event.direction() {
                ScrollDirection::Up => (0., -1.),
                ScrollDirection::Down => (0., 1.),
                ScrollDirection::Left => (-1., 0.),
                ScrollDirection::Right => (1., 0.),
                ScrollDirection::Smooth => event.delta(),
                _ => return glib::Propagation::Proceed,
            };
            let vy = vertical.borrow_mut().steps(dy, reporting);
            let hx = horizontal.borrow_mut().steps(dx, reporting);
            if reporting {
                let (x, y) = event.position();
                let pos = pointer_cell(&engine, &cfg.borrow(), x, y);
                for (steps, up, down) in [(vy, 64, 65), (hx, 66, 67)] {
                    let button = if steps < 0 { up } else { down };
                    for _ in 0..steps.unsigned_abs() {
                        if let Some(bytes) =
                            input::encode_mouse(mode, input::MouseAction::Wheel(button), pos, m)
                        {
                            write_bytes(&scope, &bytes);
                        }
                    }
                }
                // Even unencodable legacy coordinates must not scroll locally.
                return glib::Propagation::Stop;
            }
            if vy != 0 {
                dispatch(
                    "terminal.scroll",
                    serde_json::json!({"id": id, "cmd": {"delta": -vy * 3}}),
                );
            }
            glib::Propagation::Stop
        });
    }
    {
        let dispatch = dispatch.clone();
        let cfg = cfg.clone();
        let engine = engine.clone();
        let pointer = pointer.clone();
        let scope = scope.clone();
        area.connect_button_press_event(move |area, event| {
            if !scope.input(area) {
                return glib::Propagation::Stop;
            }
            // GTK emits an additional 2/3-button-press event on double clicks.
            if event.event_type() != gtk::gdk::EventType::ButtonPress {
                return glib::Propagation::Stop;
            }
            let Some(button) = event.button().checked_sub(1).filter(|b| *b < 3) else {
                return glib::Propagation::Proceed;
            };
            area.grab_focus();
            let m = modifiers(event.state());
            let mode = mouse_mode(&engine);
            let reporting = mode.active() && !m.shift;
            let (x, y) = event.position();
            let pos = pointer_cell(&engine, &cfg.borrow(), x, y);
            if pointer.borrow_mut().press(button as u8, reporting) {
                selection(&dispatch, id, pos, true);
            }
            if reporting {
                if let Some(bytes) =
                    input::encode_mouse(mode, input::MouseAction::Press(button as u8), pos, m)
                {
                    write_bytes(&scope, &bytes);
                }
            }
            if reporting || button == 0 {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    {
        let dispatch = dispatch.clone();
        let cfg = cfg.clone();
        let engine = engine.clone();
        let pointer = pointer.clone();
        let scope = scope.clone();
        area.connect_motion_notify_event(move |area, event| {
            if !scope.input(area) {
                return glib::Propagation::Stop;
            }
            let state = event.state();
            let (x, y) = event.position();
            let pos = pointer_cell(&engine, &cfg.borrow(), x, y);
            if pointer
                .borrow()
                .selecting(state.contains(gtk::gdk::ModifierType::BUTTON1_MASK))
            {
                selection(&dispatch, id, pos, false);
                return glib::Propagation::Stop;
            }
            let m = modifiers(state);
            let mode = mouse_mode(&engine);
            if mode.active() && !m.shift {
                if let Some(bytes) = input::encode_mouse(
                    mode,
                    input::MouseAction::Motion(held_button(state)),
                    pos,
                    m,
                ) {
                    write_bytes(&scope, &bytes);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    {
        let dispatch = dispatch.clone();
        let cfg = cfg.clone();
        let engine = engine.clone();
        let pointer = pointer.clone();
        let scope = scope.clone();
        area.connect_button_release_event(move |area, event| {
            if !scope.input(area) {
                return glib::Propagation::Stop;
            }
            let Some(button) = event.button().checked_sub(1).filter(|b| *b < 3) else {
                return glib::Propagation::Proceed;
            };
            let (local, reported) = pointer.borrow_mut().release(button as u8);
            let (x, y) = event.position();
            let pos = pointer_cell(&engine, &cfg.borrow(), x, y);
            if local {
                selection(&dispatch, id, pos, false);
            } else if reported {
                let mut m = modifiers(event.state());
                // Finish a remote press even if Shift was depressed mid-drag;
                // a Shift-started local selection never has a remote release.
                m.shift = false;
                if let Some(bytes) = input::encode_mouse(
                    mouse_mode(&engine),
                    input::MouseAction::Release(button as u8),
                    pos,
                    m,
                ) {
                    write_bytes(&scope, &bytes);
                }
            }
            if local || reported {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    MANAGER.with(|m| {
        let mut m = m.borrow_mut();
        m.overlay.as_ref().unwrap().add_overlay(&area);
        m.views.insert(
            id,
            Attachment {
                area: area.clone(),
                scope: scope.clone(),
                timer: None,
                im: im.clone(),
                generation,
                clipboard_epoch: 0,
                input_flow: input::InputFlow::default(),
                #[cfg(debug_assertions)]
                debug_ack: 0,
                app: window.app_handle().clone(),
                geometry,
                visible: true,
                _engine: engine,
                cfg,
                dispatch,
                grid: None,
                _renderer: renderer,
                last_seq: None,
                revision: input::ConfigRevision::default(),
                handoff_output_seq,
                drawn_output_seq: None,
                error: None,
            },
        );
    });
    {
        let scope = scope.clone();
        area.connect_size_allocate(move |area, r| {
            if scope.current(area) {
                resize(&scope, r.width(), r.height());
            }
        });
    }
    resize(
        &scope,
        geometry.width.round() as i32,
        geometry.height.round() as i32,
    );
    area.show();
    area.queue_render();
    let weak = area.downgrade();
    let timer_scope = scope.clone();
    let timer = glib::timeout_add_local(std::time::Duration::from_millis(33), move || {
        let Some(area) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !timer_scope.current(&area) {
            return glib::ControlFlow::Break;
        }
        if area.is_mapped() {
            area.queue_render();
        }
        glib::ControlFlow::Continue
    });
    MANAGER.with(|m| {
        if let Some(v) = m.borrow_mut().views.get_mut(&id) {
            v.timer = Some(timer);
        }
    });
    state(id)
}
fn resize(scope: &CallbackScope, width: i32, height: i32) {
    let id = scope.id;
    let Some(area) = scope.area.upgrade() else {
        return;
    };
    let pending = MANAGER.with(|m| {
        let mut m = m.borrow_mut();
        let v = m.views.get_mut(&id)?;
        if !scope.matches(v, &area) {
            return None;
        }
        let (cw, ch) = super::renderer::metrics(&v.cfg.borrow());
        let (cols, rows) = super::grid_size(width, height, cw, ch);
        let grid = (cols, rows, cw, ch);
        if v.grid == Some(grid) {
            return None;
        }
        v.grid = Some(grid);
        v._engine.resize(cols, rows, cw, ch);
        Some((
            v.dispatch.clone(),
            serde_json::json!({"id":id,"cols":cols,"rows":rows}),
        ))
    });
    if let Some((dispatch, params)) = pending {
        dispatch("session.resize", params);
    }
}
pub fn focus(id: Uuid) -> Result<(), HostError> {
    main_thread()?;
    let (area, scope) = MANAGER
        .with(|m| {
            m.borrow()
                .views
                .get(&id)
                .map(|v| (v.area.clone(), v.scope.clone()))
        })
        .ok_or_else(|| HostError::new("not_attached", "native view not attached"))?;
    if !scope.input(&area) {
        return Err(HostError::new(
            "input_not_ready",
            "cannot focus inactive or frozen attachment",
        ));
    }
    if !area.is_visible() {
        return Err(HostError::new("not_visible", "cannot focus hidden view"));
    }
    area.grab_focus();
    Ok(())
}
pub fn update(
    id: Uuid,
    geometry: Geometry,
    visible: bool,
    cfg: Option<ViewConfig>,
) -> Result<ViewState, HostError> {
    main_thread()?;
    let area = MANAGER.with(|m| {
        let mut m = m.borrow_mut();
        let v = m
            .views
            .get_mut(&id)
            .ok_or_else(|| HostError::new("not_attached", "native view not attached"))?;
        let cfg = cfg.ok_or_else(|| {
            HostError::new(
                "config_revision",
                "epoch and revision required even for geometry-only updates",
            )
        })?;
        // Validate the whole update before mutating geometry, style or input ACK.
        if cfg.input_generation == Some(v.generation) && cfg.input_ack > v.input_flow.seq {
            return Err(HostError::new(
                "input_ack",
                "invalid native input acknowledgement",
            ));
        }
        v.revision
            .apply(
                v.generation.as_u128(),
                cfg.expected_generation.map(|g| g.as_u128()),
                cfg.revision,
            )
            .map_err(|e| HostError::new("config_revision", e))?;
        {
            if cfg.input_generation == Some(v.generation) {
                if cfg.input_frozen {
                    v.input_flow.freeze();
                }
                if !v.input_flow.acknowledge(cfg.input_ready, cfg.input_ack) {
                    return Err(HostError::new(
                        "input_ack",
                        "invalid native input acknowledgement",
                    ));
                }
                #[cfg(debug_assertions)]
                if debug_session(id) {
                    // Mirror only an ACK accepted by the real host flow. No setter for seq.
                    v.debug_ack = v.debug_ack.max(cfg.input_ack);
                }
                v.area
                    .set_can_focus(v.input_flow.ready && v.error.is_none());
                v.area
                    .set_sensitive(v.input_flow.ready && v.error.is_none());
            }
            let style_changed = {
                let old = v.cfg.borrow();
                old.font_family != cfg.font_family
                    || old.font_size != cfg.font_size
                    || old.font_weight != cfg.font_weight
                    || old.line_padding != cfg.line_padding
                    || old.foreground != cfg.foreground
                    || old.background != cfg.background
                    || old.palette != cfg.palette
            };
            *v.cfg.borrow_mut() = cfg;
            if style_changed {
                v.area.make_current();
                if let Some(r) = v._renderer.borrow_mut().take() {
                    if v.area.error().is_none() {
                        unsafe {
                            r.destroy();
                        }
                    }
                }
                v.grid = None;
            }
        }
        v.geometry = geometry;
        if !visible {
            v.clipboard_epoch = v.clipboard_epoch.wrapping_add(1);
        }
        v.visible = visible;
        Ok((v.area.clone(), v.scope.clone()))
    })?;
    let (area, scope) = area;
    apply(&area, geometry);
    resize(
        &scope,
        geometry.width.round() as i32,
        geometry.height.round() as i32,
    );
    area.set_visible(visible);
    area.queue_render();
    #[cfg(debug_assertions)]
    if debug_session(id) {
        debug_record(id, "update");
    }
    state(id)
}
pub fn detach(id: Uuid) -> Result<(), HostError> {
    main_thread()?;
    #[cfg(debug_assertions)]
    debug_record(id, "detach");
    let mut view = MANAGER
        .with(|m| m.borrow_mut().views.remove(&id))
        .ok_or_else(|| HostError::new("not_attached", "native view not attached"))?;
    // Revoke before GTK removal can synchronously emit focus/unrealize signals.
    // Frozen input ACK draining has already finished; do not invalidate at freeze.
    view.scope.lifetime.invalidate();
    view.im.focus_out();
    view.im.reset();
    view.im.set_client_window(None::<&gtk::gdk::Window>);
    if let Some(timer) = view.timer.take() {
        timer.remove();
    }
    view.area.set_can_focus(false);
    view.area.set_sensitive(false);
    view.area.make_current();
    if let Some(renderer) = view._renderer.borrow_mut().take() {
        if view.area.error().is_none() {
            unsafe {
                renderer.destroy();
            }
        }
    }
    if let Some(parent) = view
        .area
        .parent()
        .and_then(|p| p.downcast::<gtk::Container>().ok())
    {
        parent.remove(&view.area);
    }
    Ok(())
}
pub fn state(id: Uuid) -> Result<ViewState, HostError> {
    main_thread()?;
    MANAGER.with(|m| {
        let m = m.borrow();
        let v = m
            .views
            .get(&id)
            .ok_or_else(|| HostError::new("not_attached", "native view not attached"))?;
        Ok(ViewState {
            session_id: id,
            generation: v.generation,
            geometry: v.geometry,
            visible: v.visible,
            focused: v.area.has_focus(),
            last_draw_seq: v.last_seq,
            revision: v.revision.applied,
            drawn_revision: v.revision.drawn,
            handoff_output_seq: v.handoff_output_seq.to_string(),
            drawn_output_seq: v.drawn_output_seq.map(|seq| seq.to_string()),
            input_accepted: v.input_flow.seq,
            input_ack: v.input_flow.ack,
            input_frozen: v.input_flow.frozen,
            error: v.error.clone(),
            backend: "gtk-glarea",
        })
    })
}

// Local harness only: never compiled into a release and never registered as RPC.
#[cfg(debug_assertions)]
thread_local! {
    static DEBUG_SESSION: StateCell<Option<Uuid>> = const { StateCell::new(None) };
    static DEBUG_TRACE: RefCell<Vec<serde_json::Value>> = const { RefCell::new(Vec::new()) };
    static DEBUG_OLD_AREA: RefCell<Option<gtk::GLArea>> = const { RefCell::new(None) };
}
#[cfg(debug_assertions)]
fn debug_session(id: Uuid) -> bool {
    std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() == Ok("1")
        && DEBUG_SESSION.with(|s| s.get() == Some(id))
}
#[cfg(debug_assertions)]
impl NativeHost {
    pub fn debug_e2e_arm(id: Uuid) -> Result<(), HostError> {
        main_thread()?;
        if std::env::var("AEROTAB_NATIVE_UI_E2E").as_deref() != Ok("1") {
            return Err(HostError::new("harness_disabled", "local harness disabled"));
        }
        DEBUG_SESSION.with(|s| s.set(Some(id)));
        Ok(())
    }
    pub fn debug_e2e_flow(id: Uuid) -> Result<serde_json::Value, HostError> {
        main_thread()?;
        if !debug_session(id) {
            return Err(HostError::new("harness_disabled", "session not armed"));
        }
        MANAGER.with(|m| {
            let m = m.borrow();
            let v = m.views.get(&id).ok_or_else(|| HostError::new("not_attached", "not attached"))?;
            Ok(serde_json::json!({"sessionId": id, "generation": v.generation, "reserved": v.input_flow.seq,
                "ack": v.debug_ack, "pending": v.input_flow.seq - v.debug_ack, "ready": v.input_flow.ready, "frozen":v.input_flow.frozen,
                "mapped": v.area.is_mapped(), "focused": v.area.has_focus()}))
        })
    }
    pub fn debug_e2e_trace(id: Uuid) -> Result<serde_json::Value, HostError> {
        main_thread()?;
        if !debug_session(id) {
            return Err(HostError::new("harness_disabled", "session not armed"));
        }
        Ok(DEBUG_TRACE.with(|t| serde_json::json!(*t.borrow())))
    }
    pub fn debug_e2e_inject(id: Uuid, burst: bool) -> Result<(), HostError> {
        main_thread()?;
        if !debug_session(id) {
            return Err(HostError::new("harness_disabled", "session not armed"));
        }
        let area = MANAGER
            .with(|m| m.borrow().views.get(&id).map(|v| v.area.clone()))
            .ok_or_else(|| HostError::new("not_attached", "not attached"))?;
        if !area.is_mapped() || !area.has_focus() || !area.is_sensitive() {
            return Err(HostError::new(
                "not_focused",
                "mapped focused GLArea required",
            ));
        }
        use glib::translate::*;
        // Emit actual GTK key-press signals into the production callback. This
        // is synthetic GDK, NOT hardware delivery, OS focus or IME acceptance.
        let command = if burst {
            "printf 'E2E_RACE_%s_END\\n' aB3dE5fG7hJ9\r"
        } else {
            "printf 'GDK_%s_OK\\n' BRIDGE\r"
        };
        if burst {
            DEBUG_OLD_AREA.with(|a| *a.borrow_mut() = Some(area.clone()));
        }
        for ch in command.chars() {
            let event = gtk::gdk::Event::new(gtk::gdk::EventType::KeyPress);
            unsafe {
                let ptr: *mut gtk::gdk::ffi::GdkEvent = event.to_glib_none().0;
                (*ptr).key.window = area.window().into_glib_ptr();
                (*ptr).key.keyval = if ch == '\r' { 0xff0d } else { ch as u32 };
                (*ptr).key.send_event = 1;
            }
            area.emit_by_name::<bool>("key-press-event", &[&event]);
        }
        if burst {
            debug_record(id, "burst");
        }
        Ok(())
    }
    pub fn debug_e2e_late(id: Uuid) -> Result<(), HostError> {
        main_thread()?;
        if !debug_session(id) {
            return Err(HostError::new("harness_disabled", "session not armed"));
        }
        if let Some(area) = DEBUG_OLD_AREA.with(|a| a.borrow().clone()) {
            let event = gtk::gdk::Event::new(gtk::gdk::EventType::KeyPress);
            use glib::translate::*;
            unsafe {
                let ptr: *mut gtk::gdk::ffi::GdkEvent = event.to_glib_none().0;
                (*ptr).key.keyval = 0xff0d; // harmless blank Return; detect reservation, not payload
                (*ptr).key.send_event = 1;
            }
            area.emit_by_name::<bool>("key-press-event", &[&event]);
        }
        debug_record(id, "late-old-widget");
        Ok(())
    }
}
#[cfg(debug_assertions)]
fn debug_record(id: Uuid, phase: &str) {
    if !debug_session(id) {
        return;
    }
    if let Ok(mut flow) = NativeHost::debug_e2e_flow(id) {
        flow["phase"] = serde_json::json!(phase);
        DEBUG_TRACE.with(|t| t.borrow_mut().push(flow));
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn mouse_modes_follow_the_existing_canonical_engine() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let engine = TerminalEngine::new(crate::terminal::engine::EngineConfig::default(), tx);
        assert!(!mouse_mode(&engine).active());
        engine.feed(b"\x1b[?1000h\x1b[?1006h");
        let mode = mouse_mode(&engine);
        assert!(mode.click && mode.sgr);
        assert!(!mode.motion && !mode.drag);
        engine.feed(b"\x1b[?1002h");
        assert!(mouse_mode(&engine).drag);
        engine.feed(b"\x1b[?1003h\x1b[?1005h");
        let mode = mouse_mode(&engine);
        assert!(mode.motion && mode.utf8 && !mode.sgr);
        // Encoding modes are mutually exclusive in the canonical engine.
        engine.feed(b"\x1b[?1006h");
        let mode = mouse_mode(&engine);
        assert!(mode.sgr && !mode.utf8);
        let bytes = input::encode_mouse(
            mode,
            input::MouseAction::Motion(None),
            input::CellPosition { col: 0, row: 0 },
            Modifiers::default(),
        );
        assert_eq!(bytes, Some(b"\x1b[<35;1;1M".to_vec()));
        engine.feed(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l");
        assert!(!mouse_mode(&engine).active());
    }

    #[test]
    fn gdk_button_masks_use_actual_button_state() {
        use gtk::gdk::ModifierType;
        assert_eq!(held_button(ModifierType::empty()), None);
        assert_eq!(held_button(ModifierType::BUTTON1_MASK), Some(0));
        assert_eq!(held_button(ModifierType::BUTTON2_MASK), Some(1));
        assert_eq!(held_button(ModifierType::BUTTON3_MASK), Some(2));
        let m = modifiers(
            ModifierType::SHIFT_MASK | ModifierType::CONTROL_MASK | ModifierType::MOD1_MASK,
        );
        assert!(m.shift && m.control && m.alt);
    }
}
