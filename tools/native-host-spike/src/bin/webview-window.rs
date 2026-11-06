use gtk::prelude::*;
use javascriptcore::ValueExt;
use native_host_spike::{CssRect, map_css_rect};
use serde_json::{Value, json};
use std::{cell::RefCell, ffi::CStr, rc::Rc, time::Duration};
use webkit2gtk::{UserContentManagerExt, WebViewExt};

#[link(name = "GL")]
unsafe extern "C" {
    fn glGetString(name: u32) -> *const std::ffi::c_char;
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
}
#[derive(Default)]
struct State {
    seq: u64,
    ack: u64,
    stage: String,
    rects: Vec<(i32, i32, i32, i32)>,
    hidden: bool,
    modal: bool,
    focus: i64,
    renders: [usize; 2],
    renderer: String,
    dpr: f64,
    commits: usize,
    focus_generation: u64,
}
impl State {
    fn set_focus(&mut self, focus: i64) {
        if self.focus != focus {
            self.focus_generation += 1;
            self.focus = focus;
        }
    }
}
thread_local! {static INPUTS:RefCell<[native_host_spike::handoff::InputFifo;2]>=RefCell::new(std::array::from_fn(|_|native_host_spike::handoff::InputFifo::new(4096)));static TIMEOUT:RefCell<native_host_spike::handoff::TimeoutEvidence>=RefCell::default();static RELOAD:RefCell<Option<Value>>=const{RefCell::new(None)};static STALE_RELOAD:RefCell<usize>=const{RefCell::new(0)};static DRAW_SNAPSHOTS:RefCell<[Option<native_host_spike::Snapshot>;2]>=const{RefCell::new([None,None])};}
fn input_context(
    index: usize,
    state: &State,
    switches: &[native_host_spike::handoff::Switch; 2],
) -> native_host_spike::handoff::InputContext {
    native_host_spike::handoff::InputContext {
        generation: PAGE.with(|p| p.borrow().generation),
        focus_generation: state.focus_generation,
        epoch: switches[index].epoch(),
        view: switches[index].active,
        focused: state.focus == index as i64 && !state.hidden && !state.modal,
    }
}
fn accept_input(
    index: usize,
    received: native_host_spike::handoff::InputContext,
    current: native_host_spike::handoff::InputContext,
    bytes: &[u8],
    session: &mut native_host_spike::terminal_session::NativeSession,
    pending: bool,
    view: &webkit2gtk::WebView,
) -> bool {
    let p = session.pane_mut(index);
    let result =
        INPUTS.with(|q| q.borrow_mut()[index].accept(received, current, bytes, p.initial_pid));
    match result {
        Ok(id) => {
            println!(
                "{}",
                json!({"event":"input_accepted","pane":index,"id":id,"bytes":bytes.len(),"queued":pending})
            );
            if !pending {
                replay_input(index, session, view);
            }
            true
        }
        Err(reason) => {
            let error = json!({"event":"input_backpressure","pane":index,"reason":reason,"accepted":false,"bytes":bytes.len(),"retryable":true});
            eprintln!("{error}");
            eval(
                view,
                &format!("window.inputError?.({}); true", json!(error.to_string())),
            );
            false
        }
    }
}
fn replay_input(
    index: usize,
    session: &mut native_host_spike::terminal_session::NativeSession,
    view: &webkit2gtk::WebView,
) {
    let p = session.pane_mut(index);
    let result = INPUTS.with(|q| {
        q.borrow_mut()[index].replay(p.initial_pid, |bytes| {
            p.pty
                .write(bytes)
                .map_err(|e| std::io::Error::other(e.to_string()))
        })
    });
    match result {
        Ok(n) if n > 0 => println!(
            "{}",
            json!({"event":"input_replayed","pane":index,"count":n,"pid":p.initial_pid})
        ),
        Err(e) => {
            eprintln!(
                "{}",
                json!({"event":"input_write_error","pane":index,"error":e.to_string(),"retained":true})
            );
            eval(
                view,
                "window.inputError?.('PTY write failed; accepted input retained'); true",
            );
        }
        _ => {}
    }
}

thread_local! {static PAGE: RefCell<native_host_spike::handoff::PageReady> = RefCell::default(); static DEFERRED: RefCell<[Option<String>;2]> = const { RefCell::new([None,None]) };}
fn eval(view: &webkit2gtk::WebView, script: &str) {
    let recoverable = script.starts_with("window.gridFrame");
    if recoverable && !PAGE.with(|p| p.borrow().ready) {
        if let Some(f) = script
            .strip_prefix("window.gridFrame(")
            .and_then(|s| s.strip_suffix("); true"))
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            && let Some(pane) = f["pane"].as_u64().filter(|i| *i < 2)
        {
            DEFERRED.with(|q| q.borrow_mut()[pane as usize] = Some(script.to_owned()));
        }
        return;
    }
    let guarded;
    if recoverable {
        let mut f: Value = serde_json::from_str(
            script
                .strip_prefix("window.gridFrame(")
                .unwrap()
                .strip_suffix("); true")
                .unwrap(),
        )
        .unwrap();
        PAGE.with(|p| {
            let p = p.borrow();
            f["document"] = json!(p.document);
            f["startup_generation"] = json!(p.generation);
        });
        guarded = format!("window.gridFrame({f}); true");
    } else {
        guarded = script.to_owned();
    }
    view.evaluate_javascript(&guarded, None, None, None::<&gio::Cancellable>, move |r| {
        if let Err(error) = r {
            eprintln!(
                "{}",
                json!({"event":"javascript_error","message":error.to_string()})
            );
            if !recoverable {
                std::process::exit(3);
            }
        }
    });
}
fn install(view: webkit2gtk::WebView, handle: tauri::WebviewWindow) {
    view.connect_load_changed(|_, event| {
        if event == webkit2gtk::LoadEvent::Started {
            PAGE.with(|p| p.borrow_mut().invalidate());
            DEFERRED.with(|q| *q.borrow_mut() = [None, None]);
        }
    });
    let parent = view.parent().expect("Wry WebView parent");
    let parent_type = parent.type_().name().to_string();
    let parent_box = parent.clone().downcast::<gtk::Box>().unwrap_or_else(|_| {
        eprintln!("{}", json!({"event":"unsupported_wry_parent","parent":parent_type,"reason":"Wry caches is_in_fixed_parent; refusing unverified Fixed reparenting"}));
        std::process::exit(4)
    });
    let overlay = gtk::Overlay::new();
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    // Wry built into GtkBox, so is_in_fixed_parent=false. Preserve automatic allocation,
    // no new GtkFixed parent and no Wry set_bounds calls. WebView remains in same GTK window.
    parent_box.remove(&view);
    parent_box.pack_start(&overlay, true, true, 0);
    overlay.add(&view);
    // Individual overlay children restrict input interception to their allocated pane rects.
    // A full-window Fixed overlay with pass-through would also bypass native pane clicks.
    let state = Rc::new(RefCell::new(State {
        focus: -1,
        ..State::default()
    }));
    let handoff_mode = std::env::args().any(|a| a == "--handoff-smoke");
    let switches = Rc::new(RefCell::new([
        native_host_spike::handoff::Switch::default(),
        native_host_spike::handoff::Switch::default(),
    ]));
    let expected = Rc::new(RefCell::new([None::<Value>, None::<Value>]));
    let web_acks = Rc::new(RefCell::new([0usize; 2]));
    {
        let state = state.clone();
        let switches = switches.clone();
        let expected = expected.clone();
        view.connect_load_changed(move |_, event| {
            if event == webkit2gtk::LoadEvent::Started {
                let mut st = state.borrow_mut();
                st.seq = 0;
                st.ack = 0;
                st.set_focus(-1);
                for s in switches.borrow_mut().iter_mut() {
                    s.fail();
                }
                *expected.borrow_mut() = [None, None];
            }
        });
    }
    let load_mode = std::env::args().any(|a| a == "--load-smoke");
    let terminal_mode = handoff_mode
        || load_mode
        || std::env::args().any(|a| a == "--terminal-smoke")
        || !std::env::args().any(|a| a == "--smoke");
    let session = terminal_mode.then(|| {
        Rc::new(RefCell::new(
            native_host_spike::terminal_session::NativeSession::spawn(),
        ))
    });
    let selections = Rc::new(RefCell::new([
        native_host_spike::interaction::Selection::default(),
        native_host_spike::interaction::Selection::default(),
    ]));
    let mut gpu_renderers = Vec::new();
    let mut panes = Vec::new();
    let mut contexts = Vec::new();
    for index in 0..2 {
        let gpu = Rc::new(RefCell::new(
            None::<native_host_spike::terminal_gl::Renderer>,
        ));
        gpu_renderers.push(gpu.clone());
        let area = gtk::GLArea::new();
        area.set_required_version(3, 2);
        area.set_can_focus(true);
        area.set_halign(gtk::Align::Start);
        area.set_valign(gtk::Align::Start);
        area.add_events(
            gdk::EventMask::KEY_PRESS_MASK
                | gdk::EventMask::KEY_RELEASE_MASK
                | gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::SCROLL_MASK
                | gdk::EventMask::FOCUS_CHANGE_MASK,
        );
        overlay.add_overlay(&area);
        overlay.set_overlay_pass_through(&area, false);
        let im = gtk::IMMulticontext::new();
        {
            let im = im.clone();
            area.connect_realize(move |a| {
                im.set_client_window(a.window().as_ref());
                im.set_cursor_location(&gdk::Rectangle::new(12, 22, 10, 20));
            });
        }
        {
            let im = im.clone();
            let gpu = gpu.clone();
            area.connect_unrealize(move |a| {
                a.make_current();
                if let Some(renderer) = gpu.borrow_mut().take() {
                    unsafe {
                        renderer.destroy();
                    }
                }
                im.reset();
                im.set_client_window(None);
            });
        }
        {
            let im = im.clone();
            let state = state.clone();
            area.connect_focus_in_event(move |_, _| {
                state.borrow_mut().set_focus(index as i64);
                im.focus_in();
                glib::Propagation::Proceed
            });
        }
        {
            let im = im.clone();
            area.connect_focus_out_event(move |_, _| {
                im.focus_out();
                glib::Propagation::Proceed
            });
        }
        {
            let im = im.clone();
            let state = state.clone();
            area.connect_key_press_event(move |_, e| {
                if state.borrow().hidden || state.borrow().modal {
                    return glib::Propagation::Stop;
                }
                if im.filter_keypress(e) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        {
            let im = im.clone();
            area.connect_key_release_event(move |_, e| {
                im.filter_keypress(e);
                glib::Propagation::Stop
            });
        }
        {
            let state = state.clone();
            let session = session.clone();
            let switches = switches.clone();
            let view = view.clone();
            im.connect_commit(move |_, text| {
                let s = state.borrow();
                let sw = switches.borrow();
                let current = input_context(index, &s, &sw);
                let received = native_host_spike::handoff::InputContext {
                    view: native_host_spike::handoff::View::Native,
                    ..current
                };
                let pending = sw[index].pending().is_some();
                drop(sw);
                drop(s);
                if let Some(session) = &session {
                    let accepted = accept_input(
                        index,
                        received,
                        current,
                        text.as_bytes(),
                        &mut session.borrow_mut(),
                        pending,
                        &view,
                    );
                    if accepted {
                        state.borrow_mut().commits += 1;
                    } else {
                        session.borrow_mut().pane_mut(index).blocked += 1;
                    }
                }
            });
        }
        {
            let selections = selections.clone();
            let session = session.clone();
            let state = state.clone();
            area.connect_button_press_event(move |a, e| {
                if e.button() != 1 {
                    return glib::Propagation::Proceed;
                }
                if state.borrow().modal || state.borrow().hidden {
                    return glib::Propagation::Stop;
                }
                a.grab_focus();
                state.borrow_mut().set_focus(index as i64);
                if let Some(session) = &session {
                    let s = session.borrow().pane(index).terminal.snapshot();
                    let (x, y) = e.position();
                    selections.borrow_mut()[index].begin(&s, x, y, 1.0);
                }
                glib::Propagation::Stop
            });
        }
        {
            let selections = selections.clone();
            let session = session.clone();
            let state = state.clone();
            area.connect_motion_notify_event(move |a, e| {
                if !state.borrow().modal
                    && !state.borrow().hidden
                    && let Some(session) = &session
                {
                    let s = session.borrow().pane(index).terminal.snapshot();
                    let (x, y) = e.position();
                    selections.borrow_mut()[index].extend(&s, x, y, 1.0);
                    a.queue_render();
                }
                glib::Propagation::Stop
            });
        }
        {
            let selections = selections.clone();
            let session = session.clone();
            let state = state.clone();
            area.connect_button_release_event(move |_, e| {
                if e.button() != 1 {
                    return glib::Propagation::Proceed;
                }
                selections.borrow_mut()[index].dragging = false;
                if !state.borrow().modal
                    && !state.borrow().hidden
                    && let Some(session) = &session
                {
                    let s = session.borrow().pane(index).terminal.snapshot();
                    let _ = s;
                    let text = selections.borrow()[index]
                        .range()
                        .map(|(a, b)| {
                            session
                                .borrow_mut()
                                .pane_mut(index)
                                .terminal
                                .select_cells(a, b)
                        })
                        .unwrap_or_default();
                    gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(&text);
                }
                glib::Propagation::Stop
            });
        }
        {
            let state = state.clone();
            let session = session.clone();
            let gpu = gpu.clone();
            let selections = selections.clone();
            let switches = switches.clone();
            let expected = expected.clone();
            let view = view.clone();
            area.connect_render(move |a, _| {
                if let Some(error) = a.error() {
                    eprintln!(
                        "{}",
                        json!({"event":"gl_error","message":error.to_string()})
                    );
                    return glib::Propagation::Proceed;
                }
                if let Some(session) = &session {
                    let frame_start = std::time::Instant::now();
                    let pending = switches.borrow()[index].pending();
                    let snapshot = DRAW_SNAPSHOTS
                        .with(|q| q.borrow()[index].clone())
                        .filter(|_| {
                            pending
                                .is_some_and(|(v, _)| v == native_host_spike::handoff::View::Native)
                        })
                        .unwrap_or_else(|| session.borrow().pane(index).terminal.snapshot());
                    let mut renderer = gpu.borrow_mut();
                    if renderer.is_none() {
                        *renderer =
                            Some(unsafe { native_host_spike::terminal_gl::Renderer::new() });
                    }
                    unsafe {
                        renderer.as_mut().unwrap().draw(
                            &snapshot,
                            a.allocated_width(),
                            a.allocated_height(),
                            a.scale_factor(),
                            selections.borrow()[index].range(),
                        );
                    }
                    session
                        .borrow_mut()
                        .pane_mut(index)
                        .metrics
                        .frame(frame_start);
                    if let Some((native_host_spike::handoff::View::Native, stamp)) = pending
                        && expected.borrow()[index].as_ref().is_some_and(|f| {
                            f["epoch"] == stamp.epoch
                                && f["revision"] == stamp.revision
                                && f["output_seq"] == stamp.output_seq
                        })
                        && switches.borrow()[index].epoch() == stamp.epoch
                    {
                        if snapshot
                            .cells
                            .iter()
                            .take(snapshot.cols)
                            .any(|c| c.width > 0 && c.text != " ")
                        {
                            assert!(renderer.as_ref().unwrap().pixel_ink > 0);
                        }
                        if switches.borrow_mut()[index].ack(stamp) {
                            expected.borrow_mut()[index] = None;
                            eval(&view, &format!("window.gridCommit({index},'native'); true"));
                        }
                    }
                    state.borrow_mut().renderer = renderer.as_ref().unwrap().renderer.clone();
                    state.borrow_mut().renders[index] += 1;
                    return glib::Propagation::Stop;
                }
                unsafe {
                    let ptr = glGetString(0x1F01);
                    if !ptr.is_null() {
                        state.borrow_mut().renderer =
                            CStr::from_ptr(ptr).to_string_lossy().into_owned();
                    }
                    glClearColor(
                        if index == 0 { 0.10 } else { 0.48 },
                        if index == 0 { 0.42 } else { 0.16 },
                        0.32,
                        1.0,
                    );
                    glClear(0x4000);
                }
                state.borrow_mut().renders[index] += 1;
                glib::Propagation::Stop
            });
        }
        {
            let session = session.clone();
            let state = state.clone();
            area.connect_scroll_event(move |a, e| {
                if !state.borrow().hidden
                    && !state.borrow().modal
                    && let Some(session) = &session
                {
                    let mut s = session.borrow_mut();
                    let p = s.pane_mut(index);
                    p.terminal
                        .scroll_lines(if e.direction() == gdk::ScrollDirection::Up {
                            3
                        } else {
                            -3
                        });
                    p.revision += 1;
                    a.queue_render();
                }
                glib::Propagation::Stop
            });
        }
        panes.push(area);
        contexts.push(im);
    }
    let manager = view
        .user_content_manager()
        .expect("Wry user content manager");
    assert!(manager.register_script_message_handler("layout"));
    {
        let state = state.clone();
        let panes = panes.clone();
        let contexts = contexts.clone();
        let view = view.clone();
        let session = session.clone();
        let selections = selections.clone();
        let switches = switches.clone();
        let expected = expected.clone();
        let anchors = Rc::new(RefCell::new([0usize; 2]));
        let web_acks = web_acks.clone();
        manager.connect_script_message_received(Some("layout"), move |_,result| {
            let text = result.js_value().expect("JS layout value").to_str().to_string();
            let message: Value = serde_json::from_str(&text).expect("layout JSON");
            if message["kind"]=="negativeControl"{assert_eq!(message["legacy_early_frames"],1);assert_eq!(message["legacy_timeouts"],1);println!("{}",json!({"event":"startup_negative_control_pass","legacy_early_frames":1,"legacy_timeouts":1,"live_frameearly":0,"live_retries":0}));return;}
            if message["kind"]=="fixtureHello"{if let Some(doc)=message["document"].as_str(){let generation=PAGE.with(|p|p.borrow_mut().attach(doc));let delay=if std::env::args().any(|a|a=="--delayed-startup"){2400}else{0};if delay>0{eval(&view,"window.legacyStartupNegativeControl(2400,1800); true");}eval(&view,&format!("setTimeout(()=>window.fixtureChallenge({},{}),{}); true",json!(doc),generation,delay));}return;}
            if message["kind"]=="fixtureReady"{let valid=PAGE.with(|p|p.borrow_mut().confirm(message["document"].as_str().unwrap_or(""),message["generation"].as_u64().unwrap_or(0)));println!("{}",json!({"event":"startup_ready","generation":message["generation"],"accepted":valid,"frameearly":0}));if valid{for script in DEFERRED.with(|q|std::mem::take(&mut *q.borrow_mut())).into_iter().flatten(){eval(&view,&script);}}return;}
            if message["kind"]=="input" {let index=message["pane"].as_u64().unwrap_or(9) as usize;if index>=2{return;}let epoch=message["epoch"].as_u64().unwrap_or(u64::MAX);let action=message["action"].as_str().unwrap_or("");if action=="focus"{let valid=PAGE.with(|p|{let p=p.borrow();p.ready&&message["document"]==p.document&&message["generation"]==p.generation});if valid{state.borrow_mut().set_focus(index as i64);let st=state.borrow();let sw=switches.borrow();eval(&view,&format!("window.inputContext({index},{},{},{}); true",sw[index].epoch(),st.focus_generation,PAGE.with(|p|p.borrow().generation)));}return;}
                let st=state.borrow();let sw=switches.borrow();let current=input_context(index,&st,&sw);let received=native_host_spike::handoff::InputContext{generation:message["generation"].as_u64().unwrap_or(0),focus_generation:message["focus_generation"].as_u64().unwrap_or(0),epoch,view:native_host_spike::handoff::View::Web,focused:current.focused};let pending=sw[index].pending().is_some();drop(sw);drop(st);if received!=current||!current.focused||!PAGE.with(|p|{let p=p.borrow();p.ready&&message["document"]==p.document}){let error=json!({"event":"input_rejected","reason":"stale_or_inactive","pane":index,"accepted":false});eprintln!("{error}");eval(&view,&format!("window.inputError?.({}); true",json!(error.to_string())));return;}
                if let Some(session)=&session{let mut s=session.borrow_mut();let mut user_bytes=None;let p=s.pane_mut(index);use native_host_spike::input::{Key,Modifiers,encode_key,encode_commit};match action{
                    "scroll"=>p.terminal.scroll_lines(message["lines"].as_i64().unwrap_or(0).clamp(-1000,1000) as i32),
                    "selectStart"|"selectEnd"=>{let snap=p.terminal.snapshot();let col=message["col"].as_u64().unwrap_or(u64::MAX) as usize;let row=message["row"].as_u64().unwrap_or(u64::MAX) as usize;if col<snap.cols&&row<snap.rows{let cell=row*snap.cols+col;if action=="selectStart"{anchors.borrow_mut()[index]=cell;}else{let text=p.terminal.select_cells(anchors.borrow()[index],cell);gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD).set_text(&text);}}},
                    "commit"|"paste"=>{let text=message["text"].as_str().unwrap_or("");let mut bytes=encode_commit(text);if action=="paste"&&p.terminal.snapshot().mode&16!=0{bytes=[b"\x1b[200~".as_slice(),&bytes,b"\x1b[201~"].concat();}user_bytes=Some(bytes);},
                    "key"=>{let text=message["key"].as_str().unwrap_or("");let key=match text{"Enter"=>Key::Enter,"Backspace"=>Key::Backspace,"Tab"=>Key::Tab,"Escape"=>Key::Escape,"ArrowUp"=>Key::Up,"ArrowDown"=>Key::Down,"ArrowLeft"=>Key::Left,"ArrowRight"=>Key::Right,"Home"=>Key::Home,"End"=>Key::End,"Delete"=>Key::Delete,"Insert"=>Key::Insert,"PageUp"=>Key::PageUp,"PageDown"=>Key::PageDown,t if t.chars().count()==1=>Key::Text(t),_=>return};let m=Modifiers{shift:message["shift"]==true,alt:message["alt"]==true,control:message["control"]==true,super_key:message["super_key"]==true};let bytes=encode_key(key,None,m,p.terminal.application_cursor(),false);user_bytes=Some(bytes);},_=>{}}p.revision+=1;if let Some(bytes)=user_bytes{accept_input(index,received,current,&bytes,&mut s,pending,&view);}}return;
            }
            if message["kind"]=="switch" {let index=message["pane"].as_u64().unwrap() as usize;if index>=2{return;}
                if let Some(session)=&session{let s=session.borrow();let p=s.pane(index);let target=if message["target"]=="web"{native_host_spike::handoff::View::Web}else{native_host_spike::handoff::View::Native};let stamp=switches.borrow_mut()[index].begin(target,p.revision,p.output_seq);let f=native_host_spike::handoff::frame(&p.terminal.snapshot(),stamp,index);expected.borrow_mut()[index]=Some(f.clone());DRAW_SNAPSHOTS.with(|q|q.borrow_mut()[index]=Some(p.terminal.snapshot()));if target==native_host_spike::handoff::View::Web{eval(&view,&format!("window.gridFrame({f}); true"));}else{panes[index].show();panes[index].queue_render();}}return;}
            if message["kind"]=="gridAck" {
                let index=message["pane"].as_u64().unwrap() as usize;if index>=2{return;}
                let page_valid=PAGE.with(|p|{let p=p.borrow();p.ready&&message["document"]==p.document&&message["generation"]==p.generation});
                if message["regression_old_generation"]==true{assert!(!page_valid);assert!(expected.borrow()[index].as_ref().is_some_and(|f|native_host_spike::handoff::draw_ack_matches(f,&message["ack"])));assert!(switches.borrow()[index].pending().is_some());STALE_RELOAD.with(|n|*n.borrow_mut()+=1);println!("{}",json!({"event":"reload_old_generation_rejected","generation":message["generation"],"current_generation":PAGE.with(|p|p.borrow().generation),"pending_preserved":true}));return;}
                let valid=page_valid&&expected.borrow()[index].as_ref().is_some_and(|f|native_host_spike::handoff::draw_ack_matches(f,&message["ack"]));
                if !valid {eprintln!("{}",json!({"event":"rejected_draw_ack","pane":index,"epoch":message["ack"]["epoch"],"page_valid":page_valid}));}
                if valid {let f=expected.borrow()[index].as_ref().unwrap().clone();let stamp=native_host_spike::handoff::Stamp{epoch:f["epoch"].as_u64().unwrap(),revision:f["revision"].as_u64().unwrap(),output_seq:f["output_seq"].as_u64().unwrap()};web_acks.borrow_mut()[index]+=1;if switches.borrow_mut()[index].ack(stamp){panes[index].hide();eval(&view,&format!("window.gridCommit({index},'web'); true"));}expected.borrow_mut()[index]=None;}return;
            }
            let seq = message["seq"].as_u64().expect("sequence");
            if message["kind"] == "ack" { state.borrow_mut().ack = seq; return; }
            if seq <= state.borrow().seq { eprintln!("{}",json!({"event":"stale_sequence","seq":seq})); return; }
            let dpr = message["dpr"].as_f64().expect("DPR");
            let hidden = message["hidden"].as_bool().unwrap();
            let modal = message["modal"].as_bool().unwrap();
            let focus = message["focus"].as_i64().unwrap();
            let mut rects = Vec::new();
            for (index, pane) in panes.iter().enumerate() {
                let r = &message["panes"][index];
                let rect = map_css_rect(CssRect { x:r["x"].as_f64().unwrap(), y:r["y"].as_f64().unwrap(), width:r["width"].as_f64().unwrap(), height:r["height"].as_f64().unwrap() }, dpr, view.scale_factor()).expect("valid pane geometry");
                pane.set_margin_start(rect.0);
                pane.set_margin_top(rect.1);
                pane.set_size_request(rect.2, rect.3);
                if let Some(session) = &session { session.borrow_mut().resize(index,rect.2,rect.3); }
                pane.set_visible(!hidden && !modal && (switches.borrow()[index].active==native_host_spike::handoff::View::Native || switches.borrow()[index].pending().is_some_and(|(v,_)|v==native_host_spike::handoff::View::Native)));
                pane.set_sensitive(!hidden && !modal);
                if hidden || modal { selections.borrow_mut()[index].dragging=false;contexts[index].reset(); contexts[index].focus_out(); }
                rects.push(rect);
            }
            if hidden || modal { view.grab_focus(); } else if (0..2).contains(&focus) { panes[focus as usize].grab_focus(); }
            let mut s = state.borrow_mut();
            s.seq=seq; s.stage=message["stage"].as_str().unwrap().to_owned(); s.rects=rects; s.hidden=hidden; s.modal=modal; s.set_focus(focus); s.dpr=dpr;
            println!("{}",json!({"event":"layout_applied","seq":seq,"stage":s.stage,"gtk_rects":s.rects,"dpr":dpr,"gtk_scale":view.scale_factor(),"native_visible":!hidden&&!modal}));
            drop(s);
            eval(&view,&format!("window.hostAck({seq}); true"));
        });
    }
    overlay.show_all();
    for pane in &panes {
        pane.hide();
    }
    println!(
        "{}",
        json!({"event":"actual_webview_installed","gdk_backend":gdk::Display::default().unwrap().type_().name(),"wry_parent":parent_type,"new_webview_parent":view.parent().unwrap().type_().name(),"wry_fixed_assumption":false,"same_window":true,"tauri_webview_window":true,"foreign_reparent":false,"toplevel_native_windows":0,"gtk_scale":view.scale_factor()})
    );
    // Late handler installation after page load is supported by WebKit; poll fixture readiness.
    eval(
        &view,
        "window.fixtureStep && window.fixtureStep('initial'); true",
    );
    if let Some(session) = &session {
        let session = session.clone();
        let panes = panes.clone();
        let switches = switches.clone();
        let expected = expected.clone();
        let view = view.clone();
        let state = state.clone();
        let mut pending_ticks = [0usize; 2];
        glib::timeout_add_local(Duration::from_millis(30), move || {
            for (index, pending_tick) in pending_ticks.iter_mut().enumerate() {
                if switches.borrow()[index].pending().is_some() && PAGE.with(|p| p.borrow().ready) {
                    *pending_tick += 1;
                    if *pending_tick > 60 {
                        switches.borrow_mut()[index].fail();
                        expected.borrow_mut()[index] = None;
                        *pending_tick = 0;
                        let deliberate = std::env::args().any(|a| a == "--deliberate-timeout");
                        if deliberate {
                            TIMEOUT.with(|e| e.borrow_mut().event = true);
                        }
                        println!(
                            "{}",
                            json!({"event":if deliberate{"expected_timeout"}else{"unexpected_timeout"},"pane":index,"preserved_active":format!("{:?}",switches.borrow()[index].active)})
                        );
                        eval(
                            &view,
                            "window.inputError?.('Renderer timeout; retained old view, replaying accepted input'); true",
                        );
                    }
                } else {
                    *pending_tick = 0;
                }
            }
            for index in 0..2 {
                session.borrow_mut().pane_mut(index).poll_local();
                if index == 1
                    && switches.borrow()[0].pending().is_some()
                    && session.borrow().pane(1).contains("PULSE:1:999")
                {
                    TIMEOUT.with(|e| e.borrow_mut().other_marker = true);
                }
                if switches.borrow()[index].pending().is_none() {
                    replay_input(index, &mut session.borrow_mut(), &view);
                }
            }
            for (index, _) in panes.iter().enumerate() {
                if switches.borrow()[index].active == native_host_spike::handoff::View::Web
                    && switches.borrow()[index].pending().is_none()
                    && expected.borrow()[index].is_none()
                {
                    let s = session.borrow();
                    let p = s.pane(index);
                    let stamp = native_host_spike::handoff::Stamp {
                        epoch: switches.borrow()[index].epoch(),
                        revision: p.revision,
                        output_seq: p.output_seq,
                    };
                    let f = native_host_spike::handoff::frame(&p.terminal.snapshot(), stamp, index);
                    expected.borrow_mut()[index] = Some(f.clone());
                    eval(&view, &format!("window.gridFrame({f}); true"));
                }
                if switches.borrow()[index].pending().is_none()
                    && !state.borrow().modal
                    && !state.borrow().hidden
                {
                    panes[index].set_visible(
                        switches.borrow()[index].active == native_host_spike::handoff::View::Native,
                    );
                }
            }
            for pane in &panes {
                if pane.is_mapped() {
                    pane.queue_render();
                }
            }
            glib::ControlFlow::Continue
        });
    }
    if handoff_mode {
        let session = session.clone().unwrap();
        let state = state.clone();
        let panes = panes.clone();
        let view = view.clone();
        let start = std::time::Instant::now();
        let mut phase = 0usize;
        let mut waiting = false;
        let mut revisions = [0; 2];
        let mut baseline = [String::new(), String::new()];
        let mut identities = [0usize; 2];
        let deliberate_timeout = std::env::args().any(|a| a == "--deliberate-timeout");
        let mut isolation_started = false;
        let mut isolation_passed = false;
        let reload_mode = std::env::args().any(|a| a == "--reload-regression");
        let mut reload_phase = 0;
        let mut reload_old_generation = 0;
        let mut reload_old_document = String::new();
        let contexts = contexts.clone();
        let input_regression = std::env::args().any(|a| a == "--input-regression");
        let mut input_phase = 0;
        let mut input_wait = 0;
        let mut composition_phase = 0;
        glib::timeout_add_local(Duration::from_millis(180), move || {
            if start.elapsed() > Duration::from_secs(90) {
                for index in 0..2 {
                    session.borrow_mut().pane_mut(index).pty.shutdown().unwrap();
                }
                eprintln!("handoff timeout phase {phase}");
                std::process::exit(2);
            }
            if !PAGE.with(|p| p.borrow().ready)
                || state.borrow().seq == 0
                || state.borrow().ack != state.borrow().seq
                || state.borrow().renders.contains(&0)
            {
                return glib::ControlFlow::Continue;
            }
            if !(0..2).all(|i| session.borrow().pane(i).contains("DSR_EXACTLY_ONCE")) {
                return glib::ControlFlow::Continue;
            }
            if input_regression && input_phase < 2 {
                if input_phase == 0 {
                    state.borrow_mut().set_focus(0);
                    let s = session.borrow();
                    let p = s.pane(0);
                    let stamp = switches.borrow_mut()[0].begin(
                        native_host_spike::handoff::View::Web,
                        p.revision,
                        p.output_seq,
                    );
                    let f = native_host_spike::handoff::frame(&p.terminal.snapshot(), stamp, 0);
                    expected.borrow_mut()[0] = Some(f.clone());
                    drop(s);
                    contexts[0].emit_by_name::<()>("commit", &[&"PULSE:0:980\n"]);
                    contexts[0].emit_by_name::<()>("commit", &[&"PULSE:0:981\n"]);
                    assert_eq!(INPUTS.with(|q| q.borrow()[0].bytes()), 24);
                    let c = input_context(0, &state.borrow(), &switches.borrow());
                    assert!(!accept_input(
                        0,
                        c,
                        c,
                        &vec![b'x'; 4096],
                        &mut session.borrow_mut(),
                        true,
                        &view
                    ));
                    eval(&view, &format!("window.gridFrame({f}); true"));
                    input_phase = 1;
                    return glib::ControlFlow::Continue;
                }
                if !session.borrow().pane(0).contains("PULSE:0:981") {
                    return glib::ControlFlow::Continue;
                }
                assert_eq!(
                    switches.borrow()[0].active,
                    native_host_spike::handoff::View::Web
                );
                if input_wait == 0 {
                    let s = session.borrow();
                    let p = s.pane(0);
                    let stamp = switches.borrow_mut()[0].begin(
                        native_host_spike::handoff::View::Native,
                        p.revision,
                        p.output_seq,
                    );
                    let f = native_host_spike::handoff::frame(&p.terminal.snapshot(), stamp, 0);
                    expected.borrow_mut()[0] = Some(f);
                    DRAW_SNAPSHOTS.with(|q| q.borrow_mut()[0] = Some(p.terminal.snapshot()));
                    drop(s);
                    eval(&view, "window.fixtureInput(0,'PULSE:0:982\\n'); true");
                    input_wait = 1;
                    return glib::ControlFlow::Continue;
                }
                if INPUTS.with(|q| q.borrow()[0].bytes()) == 0 {
                    return glib::ControlFlow::Continue;
                }
                switches.borrow_mut()[0].fail();
                replay_input(0, &mut session.borrow_mut(), &view);
                switches.borrow_mut()[0].active = native_host_spike::handoff::View::Native;
                expected.borrow_mut()[0] = None;
                input_phase = 2;
                println!(
                    "{}",
                    json!({"event":"input_fifo_regression_pass","native_commit_queued":2,"web_ipc_cancel_replayed":true,"overflow_rejected":true})
                );
            }
            if reload_mode && reload_phase < 3 {
                if reload_phase == 0 {
                    PAGE.with(|p| {
                        let p = p.borrow();
                        reload_old_generation = p.generation;
                        reload_old_document = p.document.clone();
                    });
                    reload_phase = 1;
                    view.reload();
                    return glib::ControlFlow::Continue;
                }
                if reload_phase == 1 {
                    if !PAGE.with(|p| {
                        let p = p.borrow();
                        p.ready && p.generation > reload_old_generation
                    }) {
                        return glib::ControlFlow::Continue;
                    }
                    let s = session.borrow();
                    let p = s.pane(0);
                    let stamp = switches.borrow_mut()[0].begin(
                        native_host_spike::handoff::View::Web,
                        p.revision,
                        p.output_seq,
                    );
                    let f = native_host_spike::handoff::frame(&p.terminal.snapshot(), stamp, 0);
                    expected.borrow_mut()[0] = Some(f.clone());
                    let mut ack = f["transport"].clone();
                    ack["phase"] = json!("drawn");
                    ack["evidence"] = json!("canvas-raf");
                    ack["physical_present"] = json!(false);
                    let injected = json!({"kind":"gridAck","pane":0,"document":reload_old_document,"generation":reload_old_generation,"regression_old_generation":true,"ack":ack});
                    eval(
                        &view,
                        &format!(
                            "window.webkit.messageHandlers.layout.postMessage(JSON.stringify({injected})); true"
                        ),
                    );
                    RELOAD.with(|q| *q.borrow_mut() = Some(f));
                    reload_phase = 2;
                    return glib::ControlFlow::Continue;
                }
                if STALE_RELOAD.with(|n| *n.borrow()) == 0 {
                    return glib::ControlFlow::Continue;
                }
                assert!(switches.borrow()[0].pending().is_some());
                switches.borrow_mut()[0].fail();
                expected.borrow_mut()[0] = None;
                println!(
                    "{}",
                    json!({"event":"reload_regression_pass","real_reload":true,"old_generation_rejected":STALE_RELOAD.with(|n|*n.borrow()),"new_pending_not_committed":true})
                );
                reload_phase = 3;
            }
            if deliberate_timeout && !isolation_passed {
                if !isolation_started {
                    state.borrow_mut().set_focus(0);
                    let stamp =
                        switches.borrow_mut()[0].begin(native_host_spike::handoff::View::Web, 0, 0);
                    expected.borrow_mut()[0] = Some(json!({"epoch":stamp.epoch}));
                    let c = input_context(0, &state.borrow(), &switches.borrow());
                    for text in ["PULSE:0:990\n", "PULSE:0:991\n"] {
                        assert!(accept_input(
                            0,
                            c,
                            c,
                            text.as_bytes(),
                            &mut session.borrow_mut(),
                            true,
                            &view
                        ));
                    }
                    session
                        .borrow_mut()
                        .pane_mut(1)
                        .input("PULSE:1:999\n", false);
                    isolation_started = true;
                    return glib::ControlFlow::Continue;
                }
                if switches.borrow()[0].pending().is_some() {
                    return glib::ControlFlow::Continue;
                }
                assert_eq!(
                    switches.borrow()[0].active,
                    native_host_spike::handoff::View::Native
                );
                if !session.borrow().pane(0).contains("PULSE:0:991") {
                    return glib::ControlFlow::Continue;
                }
                assert!(session.borrow().pane(1).contains("PULSE:1:999"));
                TIMEOUT.with(|e| {
                    let mut e = e.borrow_mut();
                    assert!(e.event, "missing expected_timeout event");
                    e.old_view = true;
                    assert!(e.other_marker, "other pane must parse before timeout");
                    e.queue_replayed = INPUTS.with(|q| q.borrow()[0].bytes() == 0);
                });
                println!(
                    "{}",
                    json!({"event":"deliberate_timeout_preserved_old","other_pane_marker":true})
                );
                isolation_passed = true;
            }
            if phase == 0 {
                for index in 0..2 {
                    let mut s = session.borrow_mut();
                    let p = s.pane_mut(index);
                    identities[index] = &p.terminal as *const _ as usize;
                    baseline[index] = p.terminal.select_cells(0, 2);
                    assert_eq!(baseline[index], "GPU");
                    p.input("PARTIAL\n", false);
                }
                phase = 1;
                return glib::ControlFlow::Continue;
            }
            if phase <= 12 {
                if waiting {
                    if switches.borrow().iter().any(|s| s.pending().is_some()) {
                        return glib::ControlFlow::Continue;
                    }
                    let wanted = if phase % 2 == 1 {
                        native_host_spike::handoff::View::Web
                    } else {
                        native_host_spike::handoff::View::Native
                    };
                    if phase == 5 {
                        if composition_phase == 0 {
                            eval(&view, "window.fixtureInput(0,'NATIVE_COMMIT\\n'); true");
                            composition_phase = 1;
                            return glib::ControlFlow::Continue;
                        }
                        if !session.borrow().pane(0).contains("COMMIT_OK") {
                            return glib::ControlFlow::Continue;
                        }
                        if composition_phase == 1 {
                            eval(&view, "window.fixtureInput(1,'NATIVE_COMMIT\\n'); true");
                            composition_phase = 2;
                            return glib::ControlFlow::Continue;
                        }
                        if !session.borrow().pane(1).contains("COMMIT_OK") {
                            return glib::ControlFlow::Continue;
                        }
                    }
                    if switches.borrow().iter().any(|s| s.active != wanted) {
                        waiting = false;
                        return glib::ControlFlow::Continue;
                    }
                    for index in 0..2 {
                        let mut s = session.borrow_mut();
                        let p = s.pane_mut(index);
                        assert_eq!(identities[index], &p.terminal as *const _ as usize);
                        assert_eq!(p.pty.pid, p.initial_pid);
                        assert!(p.revision >= revisions[index]);
                        assert_eq!(p.replies, 1);
                        if phase == 1 {
                            p.input("FINISH\n", false);
                        }
                        if phase == 2 {
                            p.input("STYLE\n", false);
                        }
                        if phase == 3 {
                            p.input("ALT\n", false);
                        }
                        if phase == 4 {
                            let text = p
                                .terminal
                                .snapshot()
                                .cells
                                .iter()
                                .map(|c| c.text.as_str())
                                .collect::<String>();
                            assert!(text.contains("ALT SCREEN 中"));
                            p.input("MAIN\n", false);
                        }
                        if phase == 5 {
                            assert_eq!(p.terminal.select_cells(0, 2), baseline[index]);
                        }
                        if phase == 7 {
                            p.terminal.scroll_lines(30);
                            p.revision += 1;
                            assert!(p.terminal.snapshot().display_offset > 0);
                        }
                        if phase == 8 {
                            assert!(p.terminal.snapshot().display_offset > 0);
                            p.terminal.scroll_bottom();
                            p.revision += 1;
                        }
                        if phase == 3 {
                            assert!(
                                p.terminal
                                    .snapshot()
                                    .cells
                                    .iter()
                                    .any(|c| c.flags & 14 == 14)
                            );
                        }
                        if phase >= 6 {
                            p.input(&format!("LOAD:{index}:{}\n", phase - 6), false);
                        }
                    }
                    println!(
                        "{}",
                        json!({"event":"handoff_cycle_verified","phase":phase,"views":format!("{:?}/{:?}",switches.borrow()[0].active,switches.borrow()[1].active),"web_ack_counts":*web_acks.borrow()})
                    );
                    if phase.is_multiple_of(3) {
                        handle
                            .set_size(tauri::LogicalSize::new(
                                1100. + (phase % 2) as f64 * 140.,
                                740.,
                            ))
                            .unwrap();
                    }
                    if phase == 8 {
                        eval(&view, "window.fixtureStep('modal'); true");
                    }
                    if phase == 9 {
                        eval(&view, "window.fixtureStep('close'); true");
                    }
                    phase += 1;
                    waiting = false;
                    return glib::ControlFlow::Continue;
                }
                if phase == 1 && !(0..2).all(|i| session.borrow().pane(i).raw.ends_with(b"\xe4")) {
                    return glib::ControlFlow::Continue;
                }
                if phase == 2
                    && !(0..2).all(|i| {
                        session
                            .borrow()
                            .pane(i)
                            .raw
                            .ends_with(b"\xe4\xb8\xad\x1b[38;2;")
                    })
                {
                    return glib::ControlFlow::Continue;
                }
                if phase == 3 && !(0..2).all(|i| session.borrow().pane(i).contains("12;34;56mX")) {
                    return glib::ControlFlow::Continue;
                }
                if phase == 4 && !(0..2).all(|i| session.borrow().pane(i).contains("ALT SCREEN")) {
                    return glib::ControlFlow::Continue;
                }
                if phase == 5 && !(0..2).all(|i| session.borrow().pane(i).contains("\x1b[?1049l")) {
                    return glib::ControlFlow::Continue;
                }
                if phase >= 7
                    && !(0..2).all(|i| {
                        session
                            .borrow()
                            .pane(i)
                            .contains(&format!("PRODUCED:{i}:{}:", phase - 7))
                    })
                {
                    return glib::ControlFlow::Continue;
                }
                if state.borrow().modal || state.borrow().hidden {
                    eval(&view, "window.fixtureStep('close'); true");
                    return glib::ControlFlow::Continue;
                }
                for index in 0..2 {
                    let s = session.borrow();
                    let p = s.pane(index);
                    revisions[index] = p.revision;
                    let target = if switches.borrow()[index].active
                        == native_host_spike::handoff::View::Native
                    {
                        native_host_spike::handoff::View::Web
                    } else {
                        native_host_spike::handoff::View::Native
                    };
                    let stamp =
                        switches.borrow_mut()[index].begin(target, p.revision, p.output_seq);
                    let f = native_host_spike::handoff::frame(&p.terminal.snapshot(), stamp, index);
                    expected.borrow_mut()[index] = Some(f.clone());
                    DRAW_SNAPSHOTS.with(|q| q.borrow_mut()[index] = Some(p.terminal.snapshot()));
                    if target == native_host_spike::handoff::View::Web {
                        eval(&view, &format!("window.gridFrame({f}); true"));
                    } else {
                        panes[index].show();
                        panes[index].queue_render();
                    }
                }
                waiting = true;
                return glib::ControlFlow::Continue;
            }
            if phase == 13 {
                if !(0..2).all(|i| {
                    session
                        .borrow()
                        .pane(i)
                        .contains(&format!("PRODUCED:{i}:6:"))
                }) {
                    return glib::ControlFlow::Continue;
                }
                for index in 0..2 {
                    for round in 0..=6 {
                        assert!(native_host_spike::load::verify(
                            &session.borrow().pane(index).raw,
                            index,
                            round
                        ));
                    }
                    session.borrow_mut().pane_mut(index).input("EXIT\n", false);
                }
                phase = 14;
                return glib::ControlFlow::Continue;
            }
            if !(0..2).all(|i| {
                let s = session.borrow();
                s.pane(i).eof && s.pane(i).pty.exit_code.is_some()
            }) {
                return glib::ControlFlow::Continue;
            }
            for index in 0..2 {
                let mut s = session.borrow_mut();
                let p = s.pane_mut(index);
                assert_eq!(p.pty.exit_code, Some(0));
                assert_eq!(p.replies, 1);
                assert!(p.contains("COMMIT_OK"));
                if index == 0 && (deliberate_timeout || input_regression) {
                    let text = String::from_utf8_lossy(&p.raw);
                    let markers = if deliberate_timeout {
                        ["PULSE:0:990", "PULSE:0:991"]
                    } else {
                        ["PULSE:0:980", "PULSE:0:981"]
                    };
                    for marker in markers {
                        assert_eq!(text.matches(marker).count(), 1);
                    }
                    assert!(text.find(markers[0]) < text.find(markers[1]));
                    if input_regression {
                        assert_eq!(text.matches("PULSE:0:982").count(), 1);
                    }
                }
                assert_eq!(p.metrics.parsed, p.raw.len());
                p.pty.shutdown().unwrap();
                assert_eq!(unsafe { libc::kill(p.initial_pid as i32, 0) }, -1);
                assert!(web_acks.borrow()[index] >= 6);
            }
            if deliberate_timeout {
                TIMEOUT.with(|e| {
                    let mut e = e.borrow_mut();
                    e.cleanup = true;
                    assert!(e.complete(), "incomplete timeout evidence");
                });
                println!(
                    "{}",
                    json!({"event":"expected_timeout_complete","old_view_active":true,"other_marker":true,"queue_replayed":true,"cleanup":true})
                );
            }
            println!(
                "{}",
                json!({"event":"single_core_two_views_pass","backend":gdk::Display::default().unwrap().type_().name(),"switches_per_pane":12,"web_ack_counts":*web_acks.borrow(),"partial_utf8_csi":true,"alternate_screen":true,"parser_instances":2,"pty_restarts":0,"same_core_identity":true,"cells_styles_width_combining_cursor":true,"web_canvas_readback":true,"physical_present_measured":false,"protocol":"FrameTransport v1 read-only module + host snapshot adapter","web_composition_roundtrip":true,"history_viewport_crossview":true,"parser_bytecount_exact":true,"scrollback_serialized":false,"manual":false})
            );
            handle.close().unwrap();
            glib::ControlFlow::Break
        });
        return;
    }
    if load_mode {
        let session = session.clone().unwrap();
        let panes = panes.clone();
        let view = view.clone();
        let state = state.clone();
        let start = std::time::Instant::now();
        let mut round = 0;
        let mut sent = false;
        let mut exiting = false;
        let mut produced = [0usize; 2];
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if start.elapsed() > Duration::from_secs(120) {
                eprintln!(
                    "{}",
                    json!({"event":"load_timeout_failure","round":round,"no_silent_drop":true})
                );
                for index in 0..2 {
                    session.borrow_mut().pane_mut(index).pty.shutdown().unwrap();
                }
                handle.close().unwrap();
                std::process::exit(2);
            }
            if state.borrow().seq == 0
                || state.borrow().renders.contains(&0)
                || state.borrow().ack != state.borrow().seq
            {
                return glib::ControlFlow::Continue;
            }
            {
                let s = session.borrow();
                assert_ne!(s.initial_pid, s.pane(1).initial_pid);
                for index in 0..2 {
                    assert_eq!(s.pane(index).pty.pid, s.pane(index).initial_pid);
                }
            }
            if round < native_host_spike::load::ROUNDS {
                if !sent {
                    if !session.borrow().contains("DSR_EXACTLY_ONCE")
                        || !session.borrow().pane(1).contains("DSR_EXACTLY_ONCE")
                    {
                        return glib::ControlFlow::Continue;
                    }
                    let stage = [
                        "split", "hide", "show", "modal", "close", "hide", "show", "modal",
                        "close", "split", "modal", "close",
                    ][round];
                    eval(&view, &format!("window.fixtureStep('{stage}'); true"));
                    handle
                        .set_size(tauri::LogicalSize::new(
                            1120.0 + (round % 2) as f64 * 120.0,
                            720.0,
                        ))
                        .unwrap();
                    for index in 0..2 {
                        session
                            .borrow_mut()
                            .pane_mut(index)
                            .input(&format!("LOAD:{index}:{round}\n"), false);
                    }
                    sent = true;
                }
                let complete = (0..2).all(|index| {
                    session
                        .borrow()
                        .pane(index)
                        .contains(&format!("PRODUCED:{index}:{round}:"))
                });
                if !complete {
                    return glib::ControlFlow::Continue;
                }
                {
                    let s = state.borrow();
                    for (index, pane) in panes.iter().enumerate() {
                        if s.hidden || s.modal {
                            assert!(!pane.is_mapped());
                            let before = session.borrow().pane(index).pty.bytes_written;
                            contexts[index].emit_by_name::<()>("commit", &[&"BLOCKED_INPUT\n"]);
                            assert_eq!(session.borrow().pane(index).pty.bytes_written, before);
                        } else {
                            assert!(pane.is_mapped());
                        }
                    }
                }
                for (index, total) in produced.iter_mut().enumerate() {
                    let n = native_host_spike::load::payload(index, round).len();
                    let s = session.borrow();
                    assert!(
                        native_host_spike::load::verify(&s.pane(index).raw, index, round),
                        "trace mismatch/drop pane {index} round {round}"
                    );
                    assert!(
                        s.pane(index)
                            .contains(&format!("PRODUCED:{index}:{round}:{n}\r\n"))
                    );
                    *total += n;
                }
                println!(
                    "{}",
                    json!({"event":"load_round_verified","round":round,"payload_bytes":produced,"layout_stage":state.borrow().stage})
                );
                round += 1;
                sent = false;
                return glib::ControlFlow::Continue;
            }
            if !exiting {
                for index in 0..2 {
                    session.borrow_mut().pane_mut(index).input("EXIT\n", false);
                }
                exiting = true;
                return glib::ControlFlow::Continue;
            }
            if !(0..2).all(|index| {
                let s = session.borrow();
                s.pane(index).eof && s.pane(index).pty.exit_code.is_some()
            }) {
                return glib::ControlFlow::Continue;
            }
            let mut reports = Vec::new();
            for index in 0..2 {
                let mut s = session.borrow_mut();
                let p = s.pane_mut(index);
                assert_eq!(p.pty.exit_code, Some(0));
                assert_eq!(p.replies, 1);
                assert!(p.contains("CHILD_DONE"));
                assert_eq!(p.metrics.parsed, p.raw.len());
                p.pty.shutdown().unwrap();
                assert_eq!(unsafe { libc::kill(p.initial_pid as i32, 0) }, -1);
                let gpu = gpu_renderers[index].borrow();
                let gpu = gpu.as_ref().unwrap();
                assert!(gpu.atlas_slots() <= native_host_spike::terminal_gl::atlas_capacity());
                reports.push(json!({"pane":index,"pid":p.initial_pid,"payload_bytes_produced_and_exact_trace_verified":produced[index],"metrics":p.metrics.summary(),"resizes":p.pty.resize_count,"eof":p.eof,"exit":p.pty.exit_code,"reader_joined":true,"atlas_slots":gpu.atlas_slots(),"atlas_pages":gpu.atlas_pages(),"atlas_capacity":native_host_spike::terminal_gl::atlas_capacity(),"texture_allocation_bytes":1048576,"renderer":gpu.renderer,"software":gpu.renderer.contains("llvmpipe")}));
            }
            println!(
                "{}",
                json!({"event":"dual_pty_load_pass","backend":gdk::Display::default().unwrap().type_().name(),"wall_elapsed_s":start.elapsed().as_secs_f64(),"usage":native_host_spike::load::usage(),"panes":reports,"same_window_svelte_native_gl":true,"manual_actions":false,"no_xterm_baseline":true,"raw_trace_retained_for_assertions":true,"latency_scope":"draw includes assertion GL readback; update includes hidden/modal wait; not input-to-photon"})
            );
            handle.close().unwrap();
            glib::ControlFlow::Break
        });
        return;
    }
    if !terminal_mode && !std::env::args().any(|a| a == "--smoke") {
        return;
    }
    let steps = [
        "initial", "split", "hide", "show", "modal", "close", "focus0", "focus1", "resized",
        "zoomed",
    ];
    let mut step = 0;
    let mut ticks = 0;
    let mut initial = Vec::new();
    let mut before_resize = Vec::new();
    let mut terminal_phase = 0;
    let state = state.clone();
    let panes = panes.clone();
    let view = view.clone();
    glib::timeout_add_local(Duration::from_millis(400), move || {
        ticks += 1;
        if ticks > 65 {
            eprintln!(
                "{}",
                json!({"event":"smoke_timeout","step":step,"stage":state.borrow().stage})
            );
            std::process::exit(2);
        }
        if state.borrow().stage != steps[step]
            || state.borrow().renders.contains(&0)
            || state.borrow().ack != state.borrow().seq
        {
            if step == 0 {
                eval(
                    &view,
                    "window.fixtureStep && window.fixtureStep('initial'); true",
                );
            }
            return glib::ControlFlow::Continue;
        }
        if let Some(session) = &session {
            let session = session.borrow();
            assert_ne!(session.initial_pid, session.pane(1).initial_pid);
            for index in 0..2 {
                let pane = session.pane(index);
                assert_eq!(pane.pty.pid, pane.initial_pid);
                if step < 9 {
                    assert!(pane.pty.exit_code.is_none());
                    assert_eq!(unsafe { libc::kill(pane.initial_pid as i32, 0) }, 0);
                }
                if !pane.contains("DSR_EXACTLY_ONCE") {
                    return glib::ControlFlow::Continue;
                }
                assert_eq!(pane.replies, 1);
            }
        }
        if step < 9
            && let Some(session) = &session
        {
            for index in 0..2 {
                session
                    .borrow_mut()
                    .pane_mut(index)
                    .input(&format!("PULSE:{index}:{step}\n"), false);
            }
        }
        let s = state.borrow();
        assert_eq!(panes[0].toplevel(), view.toplevel());
        assert_eq!(panes[1].toplevel(), view.toplevel());
        if s.hidden || s.modal {
            for pane in &panes {
                assert!(!pane.is_mapped());
                assert!(!pane.is_sensitive());
                assert!(!pane.is_focus());
            }
            if let Some(session) = &session {
                for (index, context) in contexts.iter().enumerate() {
                    let bytes = session.borrow().pane(index).pty.bytes_written;
                    context.emit_by_name::<()>("commit", &[&"BLOCKED_INPUT\n"]);
                    assert_eq!(session.borrow().pane(index).pty.bytes_written, bytes);
                }
            }
        } else {
            for (i, pane) in panes.iter().enumerate() {
                assert!(pane.is_mapped());
                let allocation = pane.allocation();
                let expected = s.rects[i];
                let position = pane
                    .translate_coordinates(&view, 0, 0)
                    .expect("same GTK widget hierarchy");
                assert_eq!(
                    (
                        position.0,
                        position.1,
                        allocation.width(),
                        allocation.height()
                    ),
                    expected
                );
            }
        }
        if step == 0 {
            initial = s.rects.clone();
        }
        if step == 1 {
            assert_ne!(initial, s.rects);
        }
        if step == 6 {
            assert!(panes[0].is_focus());
        }
        if step == 7 {
            assert!(panes[1].is_focus());
            before_resize = s.rects.clone();
        }
        if step == 8 && s.rects == before_resize {
            return glib::ControlFlow::Continue;
        }
        if step == 9 && (s.dpr - 1.25).abs() > 0.01 {
            return glib::ControlFlow::Continue;
        }
        if step == 9 {
            assert_ne!(initial, s.rects);
            assert!((view.zoom_level() - 1.25).abs() < 0.01);
            if let Some(session) = &session {
                if terminal_phase == 0 {
                    drop(s);
                    for index in 0..2 {
                        panes[index].grab_focus();
                        state.borrow_mut().set_focus(index as i64);
                        contexts[index].emit_by_name::<()>("commit", &[&"NATIVE_COMMIT\n"]);
                        session.borrow_mut().pane_mut(index).synthetic_commits += 1;
                        session
                            .borrow_mut()
                            .pane_mut(index)
                            .input("RESIZE\n", false);
                        let snapshot = session.borrow().pane(index).terminal.snapshot();
                        selections.borrow_mut()[index].begin(&snapshot, 13.0, 13.0, 1.0);
                        selections.borrow_mut()[index].extend(&snapshot, 35.0, 13.0, 1.0);
                        selections.borrow_mut()[index].dragging = false;
                        let (a, b) = selections.borrow()[index].range().unwrap();
                        let text = session
                            .borrow_mut()
                            .pane_mut(index)
                            .terminal
                            .select_cells(a, b);
                        assert_eq!(text, "GPU");
                        let clipboard = gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD);
                        clipboard.set_text(&text);
                        assert_eq!(clipboard.wait_for_text().as_deref(), Some("GPU"));
                        panes[index].queue_render();
                    }
                    terminal_phase = 1;
                    return glib::ControlFlow::Continue;
                }
                if terminal_phase == 1 {
                    if (0..2).any(|index| {
                        let s = session.borrow();
                        !s.pane(index).contains("COMMIT_OK")
                            || !s.pane(index).contains("RESIZE_OK:")
                    }) {
                        return glib::ControlFlow::Continue;
                    }
                    for index in 0..2 {
                        session.borrow_mut().pane_mut(index).input("EXIT\n", false);
                    }
                    terminal_phase = 2;
                    return glib::ControlFlow::Continue;
                }
                if (0..2).any(|index| {
                    let s = session.borrow();
                    !s.pane(index).eof || s.pane(index).pty.exit_code.is_none()
                }) {
                    return glib::ControlFlow::Continue;
                }
                let mut session = session.borrow_mut();
                assert_eq!(session.pty.exit_code, Some(0));
                assert_eq!(session.replies, 1);
                assert!(session.contains("CHILD_DONE"));
                assert!(session.blocked >= 2);
                assert!(session.contains(&format!("RESIZE_OK:{}:", session.initial_pid)));
                let snapshot = session.terminal.snapshot();
                let line: String = snapshot
                    .cells
                    .iter()
                    .take(snapshot.cols)
                    .map(|c| c.text.as_str())
                    .collect();
                assert!(line.contains("GPU GL glyph atlas"));
                let gpu = gpu_renderers[0].borrow();
                let gpu = gpu.as_ref().unwrap();
                assert!(gpu.pixel_ink > 40);
                assert!(gpu.pixel_red > 20);
                assert!(gpu.glyph_regions >= 8);
                if native_host_spike::terminal_gl::atlas_capacity() == 8 {
                    assert!(gpu.atlas_pages() > 0);
                }
                assert!(gpu.atlas_slots() <= native_host_spike::terminal_gl::atlas_capacity());
                for index in 0..2 {
                    let pane = session.pane_mut(index);
                    assert_eq!(pane.pty.exit_code, Some(0));
                    assert_eq!(pane.replies, 1);
                    assert!(pane.contains("CHILD_DONE"));
                    for stage in 0..9 {
                        assert!(pane.contains(&format!("PULSE:{index}:{stage}")));
                        assert!(!pane.contains(&format!("PULSE:{}:{stage}", 1 - index)));
                    }
                    assert!(pane.contains(&format!("RESIZE_OK:{}:", pane.initial_pid)));
                    pane.pty.shutdown().unwrap();
                    assert_eq!(unsafe { libc::kill(pane.initial_pid as i32, 0) }, -1);
                    println!(
                        "{}",
                        json!({"event":"dual_pty_cleanup","pane":index,"pid":pane.initial_pid,"dsr_replies":pane.replies,"blocked":pane.blocked,"resizes":pane.pty.resize_count,"exit":pane.pty.exit_code})
                    );
                }
                assert_eq!(unsafe { libc::kill(session.initial_pid as i32, 0) }, -1);
                println!(
                    "{}",
                    json!({"event":"native_terminal_smoke_pass","pid":session.initial_pid,"pid_preserved":true,"pty_exit":session.pty.exit_code,"eof":session.eof,"reader_shutdown_joined":true,"dsr_replies":session.replies,"pty_resizes":session.pty.resize_count,"blocked_commit_attempts":session.blocked,"synthetic_gtk_commit_roundtrips":session.synthetic_commits,"manual_ime_composition_tested":false,"snapshot_first_line":line,"snapshot_cols":snapshot.cols,"snapshot_rows":snapshot.rows,"glyph_uploads":gpu.glyph_uploads,"pixel_ink":gpu.pixel_ink,"pixel_red":gpu.pixel_red,"glyph_cell_regions":gpu.glyph_regions,"renderer":gpu.renderer,"software":gpu.renderer.contains("llvmpipe"),"pane0":"actual controlled portable-pty","pane1":"actual independent portable-pty","pid1":session.pane(1).initial_pid,"atlas_pages":gpu.atlas_pages(),"atlas_slots":gpu.atlas_slots(),"presentation":"OpenGL glyph atlas shader in GTK GLArea FBO","full_screen_cpu_texture":false,"selection":"coordinate selection state and GTK clipboard roundtrip; native pointer handlers installed, manual drag untested"})
                );
            }
            println!(
                "{}",
                json!({"event":"webview_window_smoke_pass","backend":gdk::Display::default().unwrap().type_().name(),"gl_renderer":s.renderer,"software_renderer":s.renderer.to_lowercase().contains("llvmpipe"),"svelte_fixture":true,"actual_tauri_webview":true,"same_window":true,"two_panes":true,"split_resize":true,"tab_hide_show":true,"modal_blocks_native":true,"sequence_ack":s.ack,"dpr":s.dpr,"gtk_scale":view.scale_factor(),"webkit_zoom":view.zoom_level(),"native_commits":s.commits,"real_ime_composition_tested":false,"glyph_renderer":terminal_mode,"wgpu_surface_bridge":false,"native_overlay_children":panes.len(),"native_pointer_hit_testing":"per-pane non-pass-through overlay; not manually tested"})
            );
            drop(s);
            handle.close().unwrap();
            return glib::ControlFlow::Break;
        }
        drop(s);
        step += 1;
        if step == 8 {
            handle
                .set_size(tauri::LogicalSize::new(1240.0, 760.0))
                .unwrap();
        }
        if step == 9 {
            view.set_zoom_level(1.25);
        }
        eval(
            &view,
            &format!("window.fixtureStep('{}'); true", steps[step]),
        );
        glib::ControlFlow::Continue
    });
}
fn main() {
    if std::env::args().any(|a| a == "--native-pty-child") {
        native_host_spike::terminal_session::child().unwrap();
        return;
    }
    tauri::Builder::default()
        .setup(|app| {
            let window = tauri::WebviewWindowBuilder::new(
                app,
                "fixture",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Actual Tauri WebviewWindow / isolated Svelte native composition")
            .inner_size(1100.0, 680.0)
            .build()?;
            let handle = window.clone();
            window.with_webview(move |platform| install(platform.inner(), handle))?;
            Ok(())
        })
        .run(tauri::generate_context!("tauri.conf.json"))
        .expect("WebviewWindow runtime");
}
