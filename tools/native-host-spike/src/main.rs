use gtk::prelude::*;
use javascriptcore::ValueExt;
use native_host_spike::Policy;
use serde_json::json;
use std::{cell::RefCell, ffi::CStr, rc::Rc, time::Duration};
use webkit2gtk::{UserContentManagerExt, WebViewExt};

#[link(name = "GL")]
unsafe extern "C" {
    fn glGetString(name: u32) -> *const std::ffi::c_char;
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
    fn glEnable(cap: u32);
    fn glDisable(cap: u32);
    fn glScissor(x: i32, y: i32, width: i32, height: i32);
    fn glReadPixels(x: i32, y: i32, w: i32, h: i32, format: u32, kind: u32, data: *mut u8);
}

#[derive(Default)]
struct Evidence {
    policy: Policy,
    renders: usize,
    allocations: usize,
    focus_in: usize,
    focus_out: usize,
    commits: usize,
    preedits: usize,
    web_ready: bool,
    modal_ready: bool,
    renderer: String,
    pixel_ok: bool,
}

const CONTROL: &str = r#"<!doctype html><meta charset="utf-8"><style>
body{background:#152336;color:#e4edf5;font:16px sans-serif;padding:18px}button,input{display:block;margin:15px 0;padding:12px;width:92%}small{color:#adc6da}h1{font-size:22px}</style>
<h1>GTK host composition</h1><p>WebKitGTK 4.1 control pane</p><small>NOT Tauri integration. NOT a terminal renderer.</small>
<button onclick="send('focus')">Focus native / type directly</button><button onclick="send('hide')">Hide / show native</button><button onclick="send('modal')">Open WebKit modal</button><input placeholder="WebView input (separate focus)"><p>Native input uses GtkIMMulticontext. Click colored area, type; committed text is shown in the native GTK status line.</p>
<script>function send(s){window.webkit.messageHandlers.host.postMessage(s)}</script>"#;
const MODAL: &str = r#"<!doctype html><meta charset="utf-8"><style>body{margin:0;background:#23364d;color:white;font:22px sans-serif;display:grid;place-content:center;height:100vh}button{padding:18px;font-size:18px}</style><h1>WebKit modal above native region</h1><p>Native GLArea hidden; native IME reset and blocked.</p><button onclick="window.webkit.messageHandlers.host.postMessage('close')">Close and restore native focus</button>"#;

fn web(html: &str) -> (webkit2gtk::WebView, webkit2gtk::UserContentManager) {
    let manager = webkit2gtk::UserContentManager::new();
    assert!(manager.register_script_message_handler("host"));
    let view = webkit2gtk::WebView::with_user_content_manager(&manager);
    view.load_html(html, None);
    (view, manager)
}

fn web_command(view: &webkit2gtk::WebView, command: &str) {
    view.evaluate_javascript(
        &format!(
            "window.webkit.messageHandlers.host.postMessage('{}'); true",
            command
        ),
        None,
        None,
        None::<&gio::Cancellable>,
        |result| {
            result.expect("WebKit smoke command evaluated");
        },
    );
}

fn apply(
    state: &Rc<RefCell<Evidence>>,
    area: &gtk::GLArea,
    modal: &webkit2gtk::WebView,
    im: &gtk::IMMulticontext,
) {
    let policy = state.borrow().policy;
    area.set_visible(policy.native_visible());
    area.set_sensitive(policy.native_accepts_input());
    modal.set_visible(policy.modal);
    if policy.modal {
        im.reset();
        im.focus_out();
        modal.grab_focus();
    } else if policy.native_accepts_input() {
        area.grab_focus();
        area.queue_render();
    }
}

fn main() {
    gtk::init().expect("GTK display initialization");
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    compose(window, None);
    gtk::main();
}

pub fn compose(window: gtk::Window, tauri_box: Option<gtk::Box>) {
    let tauri_host = tauri_box.is_some();
    let proof = if tauri_host {
        "tauri-window-gtk-composition"
    } else {
        "gtk-host-composition-only"
    };
    let smoke = std::env::args().any(|a| a == "--smoke");
    let display = gdk::Display::default().expect("GDK display");
    let backend = display.type_().name().to_string();
    println!(
        "{}",
        json!({"event":"environment", "proof":proof, "tauri_integrated":tauri_host,"gdk_backend":backend,"display":display.name().to_string(),"wayland_env":std::env::var("WAYLAND_DISPLAY").ok(),"foreign_reparent":false,"renderer_scope":"GL composition, not terminal glyph rendering"})
    );
    window.set_title(if tauri_host {
        "Native host spike — actual Tauri GTK window"
    } else {
        "Native host spike — GTK composition ONLY, not Tauri"
    });
    window.set_default_size(1100, 680);
    let overlay = gtk::Overlay::new();
    if let Some(vbox) = tauri_box {
        vbox.pack_start(&overlay, true, true, 0);
    } else {
        window.add(&overlay);
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    overlay.add(&row);
    let (control, manager) = web(CONTROL);
    if tauri_host {
        control.set_tooltip_text(Some(
            "WebKit control pane inside real Tauri runtime-owned GTK window",
        ));
    }
    control.set_size_request(330, -1);
    row.pack_start(&control, false, false, 0);
    let native = gtk::Box::new(gtk::Orientation::Vertical, 0);
    row.pack_start(&native, true, true, 0);
    let title = gtk::Label::new(Some(
        "IN-PROCESS NATIVE GLArea | composition proof only | click to focus",
    ));
    native.pack_start(&title, false, false, 6);
    let area = gtk::GLArea::new();
    area.set_required_version(3, 2);
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_can_focus(true);
    area.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::KEY_PRESS_MASK
            | gdk::EventMask::KEY_RELEASE_MASK
            | gdk::EventMask::FOCUS_CHANGE_MASK,
    );
    native.pack_start(&area, true, true, 0);
    let status = gtk::Label::new(Some(
        "Native IME: no commits yet | WebView keys are NOT forwarded",
    ));
    status.set_line_wrap(true);
    native.pack_start(&status, false, false, 8);
    let (modal, modal_manager) = web(MODAL);
    modal.set_hexpand(true);
    modal.set_vexpand(true);
    modal.set_no_show_all(true);
    overlay.add_overlay(&modal);
    overlay.set_overlay_pass_through(&modal, false);
    let state = Rc::new(RefCell::new(Evidence::default()));
    let im = gtk::IMMulticontext::new();
    im.set_use_preedit(true);
    {
        let im = im.clone();
        area.connect_realize(move |a| {
            im.set_client_window(a.window().as_ref());
            im.set_cursor_location(&gdk::Rectangle::new(24, 28, 12, 22));
        });
    }
    {
        let im = im.clone();
        area.connect_unrealize(move |_| {
            im.reset();
            im.set_client_window(None);
        });
    }
    {
        let state = state.clone();
        area.connect_size_allocate(move |_, _| state.borrow_mut().allocations += 1);
    }
    {
        let state = state.clone();
        let status = status.clone();
        area.connect_render(move |a, _| {
            if let Some(error) = a.error() {
                eprintln!(
                    "{}",
                    json!({"event":"gl_error", "message":error.to_string()})
                );
                return glib::Propagation::Proceed;
            }
            let mut s = state.borrow_mut();
            // GTK owns the current GL context and framebuffer. No WebView canvas or readback presentation.
            unsafe {
                let renderer = glGetString(0x1F01);
                if !renderer.is_null() {
                    s.renderer = CStr::from_ptr(renderer).to_string_lossy().into_owned();
                }
                glDisable(0x0C11);
                glClearColor(0.025, 0.06, 0.09, 1.0);
                glClear(0x4000);
                glEnable(0x0C11);
                let scale = a.scale_factor();
                for y in 0..12 {
                    for x in 0..18 {
                        glScissor(
                            (18 + x * 31) * scale,
                            (18 + y * 26) * scale,
                            23 * scale,
                            16 * scale,
                        );
                        glClearColor(0.1 + x as f32 / 24.0, 0.25 + y as f32 / 20.0, 0.65, 1.0);
                        glClear(0x4000);
                    }
                }
                glDisable(0x0C11);
                let mut pixel = [0u8; 4];
                // Readback is ONLY an assertion; never used to display the native surface.
                glReadPixels(1, 1, 1, 1, 0x1908, 0x1401, pixel.as_mut_ptr());
                s.pixel_ok = pixel[3] > 240 && pixel[2] > pixel[0];
            }
            s.renders += 1;
            status.set_text(&format!(
                "Native GL: {} | frame {} | GtkIMMulticontext attached",
                s.renderer, s.renders
            ));
            glib::Propagation::Stop
        });
    }
    {
        let im = im.clone();
        let state = state.clone();
        area.connect_focus_in_event(move |_, _| {
            state.borrow_mut().focus_in += 1;
            im.focus_in();
            glib::Propagation::Proceed
        });
    }
    {
        let im = im.clone();
        let state = state.clone();
        area.connect_focus_out_event(move |_, _| {
            state.borrow_mut().focus_out += 1;
            im.focus_out();
            glib::Propagation::Proceed
        });
    }
    area.connect_button_press_event(|a, _| {
        a.grab_focus();
        glib::Propagation::Stop
    });
    {
        let im = im.clone();
        let state = state.clone();
        area.connect_key_press_event(move |_, event| {
            if !state.borrow().policy.native_accepts_input() {
                return glib::Propagation::Stop;
            }
            if im.filter_keypress(event) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    {
        let im = im.clone();
        area.connect_key_release_event(move |_, event| {
            if im.filter_keypress(event) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    {
        let state = state.clone();
        let status = status.clone();
        im.connect_commit(move |_, text| {
            if state.borrow().policy.native_accepts_input() {
                state.borrow_mut().commits += 1;
                status.set_text(&format!("Native IME commit: {text}"));
                println!("{}", json!({"event":"native_ime_commit", "characters":text.chars().count(),"source":"GtkIMMulticontext","webview_forwarding":false}));
            }
        });
    }
    {
        let state = state.clone();
        let status = status.clone();
        im.connect_preedit_changed(move |im| {
            state.borrow_mut().preedits += 1;
            status.set_text(&format!("Native IME preedit: {}", im.preedit_string().0));
        });
    }
    for content in [&manager, &modal_manager] {
        let state = state.clone();
        let area = area.clone();
        let modal = modal.clone();
        let im = im.clone();
        content.connect_script_message_received(Some("host"), move |_, result| {
            let command = result
                .js_value()
                .map(|v| v.to_str().to_string())
                .unwrap_or_default();
            match command.as_str() {
                "focus" => area.grab_focus(),
                "hide" => {
                    let hidden = state.borrow().policy.hidden;
                    state.borrow_mut().policy.hidden = !hidden;
                }
                "modal" => state.borrow_mut().policy.modal = true,
                "close" => state.borrow_mut().policy.modal = false,
                _ => return,
            }
            apply(&state, &area, &modal, &im);
        });
    }
    {
        let state = state.clone();
        control.connect_load_changed(move |_, event| {
            if event == webkit2gtk::LoadEvent::Finished {
                state.borrow_mut().web_ready = true;
            }
        });
    }
    {
        let state = state.clone();
        modal.connect_load_changed(move |_, event| {
            if event == webkit2gtk::LoadEvent::Finished {
                state.borrow_mut().modal_ready = true;
            }
        });
    }
    window.connect_delete_event(move |_, _| {
        if !tauri_host {
            gtk::main_quit();
        }
        glib::Propagation::Proceed
    });
    println!("{}", json!({"event":"before_show"}));
    window.show_all();
    println!("{}", json!({"event":"after_show"}));
    modal.hide();
    area.grab_focus();
    if smoke {
        let mut tick = 0;
        let mut stage = 0;
        let mut initial_size = (0, 0);
        let state = state.clone();
        let area = area.clone();
        let window = window.clone();
        let modal = modal.clone();
        let control = control.clone();
        glib::timeout_add_local(Duration::from_millis(350), move || {
            tick += 1;
            if tick % 10 == 0 {
                println!(
                    "{}",
                    json!({"event":"smoke_progress","tick":tick,"stage":stage,"web_ready":state.borrow().web_ready,"modal_ready":state.borrow().modal_ready,"renders":state.borrow().renders})
                );
            }
            if tick > 60 {
                eprintln!("{}", json!({"event":"smoke_timeout", "stage":stage}));
                std::process::exit(2);
            }
            if stage == 0
                && (!state.borrow().web_ready
                    || !state.borrow().modal_ready
                    || state.borrow().renders == 0)
            {
                return glib::ControlFlow::Continue;
            }
            match stage {
                0 => {
                    assert!(area.is_mapped());
                    assert!(state.borrow().pixel_ok);
                    initial_size = (area.allocated_width(), area.allocated_height());
                    window.resize(1260, 780);
                }
                1 => {
                    assert_ne!(
                        initial_size,
                        (area.allocated_width(), area.allocated_height())
                    );
                    web_command(&control, "hide");
                }
                2 => {
                    assert!(!area.is_mapped());
                    assert!(!area.is_sensitive());
                    web_command(&control, "hide");
                }
                3 => {
                    assert!(area.is_mapped());
                    area.grab_focus();
                    web_command(&control, "modal");
                }
                4 => {
                    assert!(!area.is_mapped());
                    assert!(modal.is_mapped());
                    assert!(!area.has_focus());
                    web_command(&modal, "close");
                }
                5 => {
                    assert!(area.is_mapped());
                    assert!(!modal.is_visible());
                    assert!(area.is_focus());
                    control.grab_focus();
                }
                6 => {
                    assert!(!area.is_focus());
                    area.grab_focus();
                }
                7 => {
                    assert!(area.is_focus());
                    let s = state.borrow();
                    let software = ["llvmpipe", "softpipe", "swrast", "software"]
                        .iter()
                        .any(|name| s.renderer.to_lowercase().contains(name));
                    println!(
                        "{}",
                        json!({"event":"smoke_pass", "proof":proof, "gdk_backend":backend,"gl_renderer":s.renderer,"software_renderer":software,"hardware_acceleration_proven":!software && !s.renderer.is_empty(),"renders":s.renders,"allocations":s.allocations,"focus_in_events":s.focus_in,"focus_out_events":s.focus_out,"framebuffer_pixel_assertion":s.pixel_ok,"web_loaded":s.web_ready,"modal_web_loaded":s.modal_ready,"resize":true,"hide_show":true,"overlay_modal":true,"focus_restore":true,"ime_context":"GtkIMMulticontext","ime_commit_events":s.commits,"ime_preedit_events":s.preedits,"ime_user_composition_tested":false,"tauri_integrated":tauri_host,"foreign_reparent":false})
                    );
                    drop(s);
                    window.close();
                    if !tauri_host {
                        gtk::main_quit();
                    }
                    return glib::ControlFlow::Break;
                }
                _ => unreachable!(),
            }
            stage += 1;
            glib::ControlFlow::Continue
        });
    }
}
