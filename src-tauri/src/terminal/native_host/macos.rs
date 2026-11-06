//! Main-thread AppKit child view of the existing canonical TerminalEngine.
//! No NSApplication run loop, parser, PTY, Channel or helper process is created.
use super::{input, macos_policy, macos_renderer::Renderer, *};
use crate::terminal::engine::TerminalEngine;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSEvent, NSEventModifierFlags, NSPasteboard, NSPasteboardTypeString, NSResponder,
    NSTextInputClient, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindowOcclusionState,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSObjectProtocol, NSPoint, NSRange, NSRect,
    NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};
use std::{
    cell::{Cell as StateCell, RefCell},
    ptr::NonNull,
    sync::Arc,
};
use tauri::{Emitter, Manager};
thread_local! { static VIEWS: RefCell<HashMap<Uuid, Attachment>> = RefCell::default(); }
struct ViewIvars {
    id: Uuid,
    generation: Uuid,
    live: StateCell<bool>,
    marked: RefCell<macos_policy::Preedit>,
    pointer: RefCell<input::PointerState>,
    vertical: RefCell<input::WheelState>,
    horizontal: RefCell<input::WheelState>,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}
define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewIvars]
    struct TerminalView;
    unsafe impl NSObjectProtocol for TerminalView {}
    impl TerminalView {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_focus(&self) -> bool { self.ivars().live.get() && with_view(self, |v| v.visible && v.flow.ready && v.error.is_none()).unwrap_or(false) }
        #[unsafe(method(resignFirstResponder))]
        fn resign(&self) -> bool { self.clear_preedit(); if let Some(context)=self.inputContext() {context.discardMarkedText();} true }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            if let Some(window) = self.window() { window.makeFirstResponder(Some(self)); }
            self.pointer_event(event, 0);
        }
        #[unsafe(method(rightMouseDown:))]
        fn right_down(&self, event: &NSEvent) { self.pointer_event(event, 0); }
        #[unsafe(method(otherMouseDown:))]
        fn other_down(&self, event: &NSEvent) { self.pointer_event(event, 0); }
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) { self.pointer_event(event, 1); }
        #[unsafe(method(rightMouseUp:))]
        fn right_up(&self, event: &NSEvent) { self.pointer_event(event, 1); }
        #[unsafe(method(otherMouseUp:))]
        fn other_up(&self, event: &NSEvent) { self.pointer_event(event, 1); }
        #[unsafe(method(mouseDragged:))]
        fn mouse_drag(&self, event: &NSEvent) { self.pointer_event(event, 2); }
        #[unsafe(method(rightMouseDragged:))]
        fn right_drag(&self, event: &NSEvent) { self.pointer_event(event, 2); }
        #[unsafe(method(otherMouseDragged:))]
        fn other_drag(&self, event: &NSEvent) { self.pointer_event(event, 2); }
        #[unsafe(method(mouseMoved:))]
        fn mouse_move(&self, event: &NSEvent) { self.pointer_event(event, 2); }
        #[unsafe(method(scrollWheel:))]
        fn scroll(&self, event: &NSEvent) { self.scroll_event(event); }
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) {
            unsafe {let _:()=msg_send![super(self),updateTrackingAreas];}
            if let Some(old)=self.ivars().tracking.borrow_mut().take() {self.removeTrackingArea(&old);}
            let tracking=unsafe {NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(),self.bounds(),NSTrackingAreaOptions::MouseMoved|NSTrackingAreaOptions::ActiveInKeyWindow|NSTrackingAreaOptions::InVisibleRect,Some(self),None)};
            self.addTrackingArea(&tracking);
            *self.ivars().tracking.borrow_mut()=Some(tracking);
        }
        #[unsafe(method(viewDidMoveToWindow))]
        fn moved_to_window(&self) {
            if self.window().is_none() { self.ivars().live.set(false); self.clear_preedit(); }
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) { self.enqueue_key(event); }
        #[unsafe(method(localKey:))]
        fn local_key(&self, event: &NSEvent) {
            if !self.active() {return;}
            if !self.ivars().marked.borrow().text.is_empty() {
                let events=NSArray::from_slice(&[event]);
                unsafe {let _:()=msg_send![self,interpretKeyEvents:&*events];}
                return;
            }
            let m = modifiers(event);
            let key = key(event);
            if let Some(action)=input::clipboard_action(key,m,!self.ivars().marked.borrow().text.is_empty()) {
                match action {input::ClipboardAction::Copy=>self.copy(),input::ClipboardAction::Paste=>self.paste()}
                return;
            }
            // Command chords belong to the application, never to the remote host.
            if m.super_key {
                if matches!(key, input::Key::Character('v'|'V')) { self.paste(); }
                else if matches!(key,input::Key::Character('c'|'C')) { self.copy(); }
                else { unsafe { let _: () = msg_send![super(self), keyDown: event]; } }
                return;
            }
            if self.ivars().marked.borrow().text.is_empty() {
                // The production HotkeyManager has already had first refusal;
                // an unbound Ctrl chord must now reach the terminal.
                // Option text/dead keys must be interpreted by the macOS input method.
                if !m.alt || input::shortcut_before_ime(m, false) && m.control {
                    let app = with_view(self, |v| v.engine.mode_bits() & alacritty_terminal::term::TermMode::APP_CURSOR.bits() != 0).unwrap_or(false);
                    if let Some(text) = input::encode(key,m,app) { self.bytes(text.as_bytes()); return; }
                }
            }
            let events = NSArray::from_slice(&[event]);
            unsafe { let _: () = msg_send![self, interpretKeyEvents: &*events]; }
        }
        #[unsafe(method(paste:))]
        fn paste_action(&self, _sender: Option<&AnyObject>) { self.paste(); }
        #[unsafe(method(copy:))]
        fn copy_action(&self, _sender: Option<&AnyObject>) { self.copy(); }
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            if !self.ivars().live.get() || self.window().is_none() {
                let removed=VIEWS.with(|views|views.try_borrow_mut().ok().and_then(|mut v| {
                    if v.get(&self.ivars().id).is_some_and(|v|v.generation==self.ivars().generation) {v.remove(&self.ivars().id)} else {None}
                }));
                drop(removed);return;
            }
            let active=self.active();
            VIEWS.with(|views| {
                let Ok(mut views) = views.try_borrow_mut() else { return; };
                let Some(v) = views.get_mut(&self.ivars().id) else { return; };
                if v.generation != self.ivars().generation { return; }
                if !active || NSEvent::pressedMouseButtons()&1==0 {v.drag=None;}
                if v.drag_tick.elapsed()>=std::time::Duration::from_millis(50) {
                    v.drag_tick=std::time::Instant::now();
                    if let Some((x,y))=v.drag {
                        let delta=macos_policy::edge_scroll(y,v.geometry.height,f64::from(v.cell.1));
                        if delta!=0 {
                            // Same canonical engine, synchronous order: scroll then extend.
                            v.engine.scroll(crate::terminal::engine::ScrollCmd::Delta(delta));
                            let (cols,rows)=v.engine.dims();let p=input::cell_position(x,y,v.cell.0,v.cell.1,cols,rows);
                            v.engine.selection_update(usize::from(p.col),i32::from(p.row),crate::terminal::engine::CellSide::Right);
                        }
                    }
                }
                if v.key_started.is_some_and(|start|start.elapsed()>std::time::Duration::from_secs(2)) {
                    v.keys.clear();v.key_epoch=v.key_epoch.wrapping_add(1);v.key_started=None;v.flow.freeze();v.error=Some(HostError::new("shortcut_timeout","shortcut resolver timed out"));
                }
                if let Err(error) = v.draw() { v.error = Some(error); v.flow.freeze(); }
            });
        }
    }
    unsafe impl NSTextInputClient for TerminalView {
        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insert_text(&self, value: &AnyObject, range: NSRange) {
            let replacement=(range.location!=usize::MAX).then_some((range.location,range.length));
            let text=object_text(value).and_then(|text|self.ivars().marked.borrow().commit(&text,replacement));
            // Unsupported committed-document replacement is rejected, not translated
            // into destructive backspaces against unknown remote application state.
            if let Some(text)=text {self.clear_preedit();self.bytes(text.as_bytes());}
        }
        #[unsafe(method(doCommandBySelector:))]
        unsafe fn command(&self, selector: Sel) {
            let text = if selector == sel!(insertNewline:) { Some("\r") }
                else if selector == sel!(insertTab:) { Some("\t") }
                else if selector == sel!(deleteBackward:) { Some("\x7f") }
                else if selector == sel!(cancelOperation:) { Some("\x1b") } else { None };
            if let Some(text) = text { self.bytes(text.as_bytes()); }
        }
        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn set_marked(&self, value: &AnyObject, selected: NSRange, replacement: NSRange) {
            if self.active() {
                self.ivars().marked.borrow_mut().replace(object_text(value).unwrap_or_default(), selected.location, selected.length, (replacement.location!=usize::MAX).then_some((replacement.location,replacement.length)));
                if let Some(context)=self.inputContext() {context.invalidateCharacterCoordinates();}
            }
        }
        #[unsafe(method(unmarkText))]
        fn unmark(&self) { self.clear_preedit(); }
        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            let p = self.ivars().marked.borrow();
            if p.text.is_empty() { NSRange::new(usize::MAX, 0) } else { NSRange::new(p.selected.0, p.selected.1) }
        }
        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let n = macos_policy::utf16_len(&self.ivars().marked.borrow().text);
            NSRange::new(if n == 0 {usize::MAX} else {0},n)
        }
        #[unsafe(method(hasMarkedText))]
        fn has_marked(&self) -> bool { !self.ivars().marked.borrow().text.is_empty() }
        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn substring(&self, range: NSRange, actual: *mut NSRange) -> Option<Retained<NSAttributedString>> {
            let result = self.ivars().marked.borrow().substring(range.location, range.length);
            if !actual.is_null() { unsafe { *actual = result.as_ref().map_or(NSRange::new(usize::MAX, 0), |(_, r)|NSRange::new(r.0,r.1)); } }
            result.map(|(text,_)| NSAttributedString::initWithString(NSAttributedString::alloc(), &NSString::from_str(&text)))
        }
        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSString>> { NSArray::new() }
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn first_rect(&self, range: NSRange, actual: *mut NSRange) -> NSRect {
            let p=self.ivars().marked.borrow();
            let location=range.location.min(macos_policy::utf16_len(&p.text));
            let location=p.substring(location,0).map_or(location,|(_,r)|r.0);
            drop(p);
            if !actual.is_null() { unsafe { *actual = NSRange::new(if self.ivars().marked.borrow().text.is_empty() {usize::MAX}else{location},0); } }
            let rect = self.composition_layout(|layout|layout.caret(location)).flatten().map(|(x,y,h)|NSRect::new(NSPoint::new(x,y),NSSize::new(1.,h))).unwrap_or_else(|| {
                with_view(self, |v| {
                    let snap=v.engine.native_snapshot_envelope();let cursor=Snapshot::decode(&snap.bytes,&v.cfg).map(|s|s.cursor).unwrap_or((0,0));
                    NSRect::new(NSPoint::new(12.+cursor.0 as f64*f64::from(v.cell.0),12.+cursor.1 as f64*f64::from(v.cell.1)),NSSize::new(1.,f64::from(v.cell.1)))
                }).unwrap_or_else(||self.bounds())
            });
            let in_window: NSRect = unsafe { msg_send![self, convertRect: rect, toView: Option::<&NSView>::None] };
            self.window().map_or(in_window, |w| w.convertRectToScreen(in_window))
        }
        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, point: NSPoint) -> usize {
            let Some(window)=self.window() else {return usize::MAX;};
            let rect=window.convertRectFromScreen(NSRect::new(point,NSSize::new(0.,0.)));
            let local=self.convertPoint_fromView(rect.origin,None);
            let bounds=self.bounds();
            if local.x<0.||local.y<0.||local.x>bounds.size.width||local.y>bounds.size.height {return usize::MAX;}
            self.composition_layout(|layout|layout.hit(local.x,local.y)).flatten().unwrap_or(usize::MAX)
        }
    }
);
fn object_text(value: &AnyObject) -> Option<String> {
    if let Some(s) = value.downcast_ref::<NSString>() {
        Some(s.to_string())
    } else {
        value
            .downcast_ref::<NSAttributedString>()
            .map(|s| s.string().to_string())
    }
}
fn with_view<T>(view: &TerminalView, f: impl FnOnce(&Attachment) -> T) -> Option<T> {
    if !view.ivars().live.get() {
        return None;
    }
    VIEWS.with(|views| {
        let views = views.try_borrow().ok()?;
        let v = views.get(&view.ivars().id)?;
        (v.generation == view.ivars().generation && std::ptr::eq(&*v.view, view)).then(|| f(v))
    })
}
/// Route through the existing production DOM HotkeyManager and its persisted
/// bindings/guards, not a second hard-coded resolver. No terminal rendering here.
fn route_key(id: Uuid, generation: Uuid) {
    let current = VIEWS.with(|views| {
        let mut views = views.try_borrow_mut().ok()?;
        let v = views.get_mut(&id)?;
        if v.generation != generation || v.keys.is_empty() || v.flow.frozen {
            return None;
        }
        v.key_started = Some(std::time::Instant::now());
        Some((
            v.window.clone(),
            v.keys.front()?.clone(),
            v.key_epoch,
            v.geometry,
        ))
    });
    let Some((window, event, epoch, geometry)) = current else {
        return;
    };
    let m = modifiers(&event);
    let js = super::shortcut::shortcut_script(key(&event), m, geometry, id);
    let app = window.app_handle().clone();
    let result = window.eval_with_callback(js, move |result| {
        let parsed = serde_json::from_str::<serde_json::Value>(&result).ok();
        let decision = parsed
            .as_ref()
            .and_then(|v| v.get("consumed"))
            .and_then(|v| v.as_bool());
        let _ = app.run_on_main_thread(move || {
            let pending = VIEWS.with(|views| {
                let Ok(mut views) = views.try_borrow_mut() else {
                    return None;
                };
                let v = views.get_mut(&id)?;
                if v.generation != generation || v.key_epoch != epoch {
                    return None;
                }
                let event = v.keys.pop_front()?;
                v.key_started = None;
                Some((v.view.clone(), event))
            });
            let Some((view, event)) = pending else {
                return;
            };
            match decision {
                Some(false) => view.local_key(sel!(localKey:), &event),
                Some(true) => {}
                None => {
                    VIEWS.with(|views| {
                        if let Some(v) = views.borrow_mut().get_mut(&id) {
                            v.flow.freeze();
                            v.error = Some(HostError::new(
                                "shortcut_bridge",
                                "shortcut resolver unavailable",
                            ));
                        }
                    });
                }
            }
            route_key(id, generation);
        });
    });
    if result.is_err() {
        VIEWS.with(|views| {
            if let Some(v) = views.borrow_mut().get_mut(&id) {
                v.keys.clear();
                v.flow.freeze();
                v.error = Some(HostError::new(
                    "shortcut_bridge",
                    "shortcut dispatch failed",
                ));
            }
        });
    }
}
fn modifiers(event: &NSEvent) -> input::Modifiers {
    let flags = event.modifierFlags();
    input::Modifiers {
        shift: flags.contains(NSEventModifierFlags::Shift),
        alt: flags.contains(NSEventModifierFlags::Option),
        control: flags.contains(NSEventModifierFlags::Control),
        super_key: flags.contains(NSEventModifierFlags::Command),
        level_three: false,
    }
}
fn mouse_mode(engine: &TerminalEngine) -> input::MouseMode {
    use alacritty_terminal::term::TermMode;
    let mode = TermMode::from_bits_retain(engine.mode_bits());
    input::MouseMode {
        click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
        drag: mode.contains(TermMode::MOUSE_DRAG),
        motion: mode.contains(TermMode::MOUSE_MOTION),
        sgr: mode.contains(TermMode::SGR_MOUSE),
        utf8: mode.contains(TermMode::UTF8_MOUSE),
    }
}
impl TerminalView {
    fn composition_layout<T>(
        &self,
        f: impl FnOnce(&macos_policy::PreeditLayout) -> T,
    ) -> Option<T> {
        let p = self.ivars().marked.borrow().clone();
        if p.text.is_empty() {
            return None;
        }
        VIEWS.with(|views| {
            let mut views = views.try_borrow_mut().ok()?;
            let v = views.get_mut(&self.ivars().id)?;
            if v.generation != self.ivars().generation || !self.ivars().live.get() {
                return None;
            }
            let snap = v.engine.native_snapshot_envelope();
            let snapshot = Snapshot::decode(&snap.bytes, &v.cfg).ok()?;
            let renderer = v.renderer.as_mut()?;
            renderer.preedit_buffer(
                &p,
                &v.cfg,
                v.cell,
                snapshot.cursor,
                [v.geometry.width, v.geometry.height],
            );
            Some(f(&renderer.layout))
        })
    }
    fn enqueue_key(&self, event: &NSEvent) {
        if !self.active() {
            return;
        }
        // Composition belongs exclusively to NSTextInputClient, never DOM shortcuts.
        if !self.ivars().marked.borrow().text.is_empty() {
            self.local_key(sel!(localKey:), event);
            return;
        }
        let start = VIEWS.with(|views| {
            let Ok(mut views) = views.try_borrow_mut() else {
                return false;
            };
            let Some(v) = views.get_mut(&self.ivars().id) else {
                return false;
            };
            if v.generation != self.ivars().generation {
                return false;
            }
            if v.keys.len() >= 256 {
                v.flow.freeze();
                v.error = Some(HostError::new(
                    "shortcut_overflow",
                    "native key queue exceeded",
                ));
                return false;
            }
            let start = v.keys.is_empty();
            v.keys.push_back(
                unsafe { Retained::retain(event as *const NSEvent as *mut NSEvent) }
                    .expect("non-null NSEvent"),
            );
            start
        });
        if start {
            route_key(self.ivars().id, self.ivars().generation);
        }
    }
    fn pointer_context(
        &self,
        event: &NSEvent,
    ) -> Option<(input::CellPosition, input::MouseMode, InputDispatch, Uuid)> {
        if !self.active() {
            return None;
        }
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        with_view(self, |v| {
            let (cols, rows) = v.engine.dims();
            (
                input::cell_position(point.x, point.y, v.cell.0, v.cell.1, cols, rows),
                mouse_mode(&v.engine),
                v.dispatch.clone(),
                v.id,
            )
        })
    }
    fn pointer_event(&self, event: &NSEvent, kind: u8) {
        let Some((pos, mode, dispatch, id)) = self.pointer_context(event) else {
            return;
        };
        if !self.ivars().marked.borrow().text.is_empty() {
            if let Some(context) = self.inputContext() {
                if context.handleEvent(event) {
                    return;
                }
            }
        }
        let mut m = modifiers(event);
        let reporting = mode.active() && !m.shift;
        let button = macos_policy::terminal_button(event.buttonNumber());
        let held = NSEvent::pressedMouseButtons();
        let held_button = if held & 1 != 0 {
            Some(0)
        } else if held & 4 != 0 {
            Some(1)
        } else if held & 2 != 0 {
            Some(2)
        } else {
            None
        };
        let mut select = None;
        let action = {
            let mut pointer = self.ivars().pointer.borrow_mut();
            match kind {
                0 => button.and_then(|b| {
                    if pointer.press(b, reporting) {
                        select = Some(true);
                    }
                    reporting.then_some(input::MouseAction::Press(b))
                }),
                1 => button.and_then(|b| {
                    let (local, reported) = pointer.release(b);
                    if local {
                        select = Some(false);
                    }
                    if reported {
                        m.shift = false;
                        Some(input::MouseAction::Release(b))
                    } else {
                        None
                    }
                }),
                _ => {
                    if pointer.selecting(held & 1 != 0) {
                        select = Some(false);
                        None
                    } else {
                        Some(input::MouseAction::Motion(held_button))
                    }
                }
            }
        };
        if let Some(start) = select {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            let auto = if kind == 1 {
                None
            } else {
                Some((point.x, point.y))
            };
            VIEWS.with(|views| {
                if let Ok(mut views) = views.try_borrow_mut() {
                    if let Some(v) = views.get_mut(&id) {
                        v.drag = auto;
                    }
                }
            });
            let mut p = serde_json::json!({"id":id,"col":pos.col,"line":pos.row,"side":if start {"left"}else{"right"}});
            if start {
                p["kind"] =
                    serde_json::json!(macos_policy::selection_kind(event.clickCount(), m.alt));
            }
            dispatch(
                if start {
                    "terminal.selectionStart"
                } else {
                    "terminal.selectionUpdate"
                },
                p,
            );
        }
        if let Some(action) = action {
            if let Some(bytes) = input::encode_mouse(mode, action, pos, m) {
                self.bytes(&bytes);
            }
        }
    }
    fn scroll_event(&self, event: &NSEvent) {
        let Some((pos, mode, dispatch, id)) = self.pointer_context(event) else {
            return;
        };
        let m = modifiers(event);
        let reporting = mode.active() && !m.shift;
        if !reporting && (m.control || m.super_key) {
            unsafe {
                let _: () = msg_send![super(self),scrollWheel:event];
            }
            return;
        }
        let line = with_view(self, |v| f64::from(v.cell.1)).unwrap_or(20.);
        let precise = event.hasPreciseScrollingDeltas();
        let vy = self.ivars().vertical.borrow_mut().steps(
            macos_policy::wheel_delta(event.scrollingDeltaY(), precise, line),
            reporting,
        );
        let hx = self.ivars().horizontal.borrow_mut().steps(
            macos_policy::wheel_delta(event.scrollingDeltaX(), precise, line),
            reporting,
        );
        if reporting {
            for (steps, negative, positive) in [(vy, 64, 65), (hx, 66, 67)] {
                for _ in 0..steps.unsigned_abs() {
                    if let Some(bytes) = input::encode_mouse(
                        mode,
                        input::MouseAction::Wheel(if steps < 0 { negative } else { positive }),
                        pos,
                        m,
                    ) {
                        self.bytes(&bytes);
                    }
                }
            }
        } else if vy != 0 {
            dispatch(
                "terminal.scroll",
                serde_json::json!({"id":id,"cmd":{"delta":-vy*3}}),
            );
        }
    }
    fn clear_preedit(&self) {
        self.ivars().marked.borrow_mut().clear();
        *self.ivars().pointer.borrow_mut() = input::PointerState::default();
        *self.ivars().vertical.borrow_mut() = input::WheelState::default();
        *self.ivars().horizontal.borrow_mut() = input::WheelState::default();
    }
    fn active(&self) -> bool {
        with_view(self, |v| {
            v.visible && v.flow.ready && !v.flow.frozen && v.error.is_none()
        })
        .unwrap_or(false)
            && !self.isHiddenOrHasHiddenAncestor()
            && self.window().is_some_and(|w| {
                w.isKeyWindow()
                    && w.firstResponder()
                        .is_some_and(|r| std::ptr::eq(&*r, self as &NSResponder))
            })
    }
    fn bytes(&self, bytes: &[u8]) {
        if bytes.is_empty() || !self.active() {
            return;
        }
        VIEWS.with(|views| {
            let Ok(mut views) = views.try_borrow_mut() else { return; };
            let Some(v) = views.get_mut(&self.ivars().id) else { return; };
            if v.generation != self.ivars().generation || !self.ivars().live.get() { return; }
            let Some(seq) = v.flow.reserve(bytes.len()) else { v.flow.freeze(); v.error = Some(HostError::new("input_overflow","native input bridge capacity exceeded")); return; };
            if v.app.emit_to(tauri::EventTarget::webview_window("main"),"aerotab:native-input",serde_json::json!({"sessionId":v.id,"generation":v.generation,"seq":seq,"data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,bytes)})).is_err() {
                v.flow.freeze(); v.error = Some(HostError::new("input_bridge","native input event delivery failed"));
            }
        });
    }
    fn paste(&self) {
        if !self.active() || !self.ivars().marked.borrow().text.is_empty() {
            return;
        }
        let Some(text) =
            (unsafe { NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString) })
        else {
            return;
        };
        VIEWS.with(|views| {
            let Ok(mut views) = views.try_borrow_mut() else { return; };
            let Some(v) = views.get_mut(&self.ivars().id) else { return; };
            if v.generation != self.ivars().generation { return; }
            let Some(request) = v.paste_request.checked_add(1) else { v.flow.freeze(); return; };
            v.paste_request = request;
            // Existing confirmation/bracketed-paste route, never a direct PTY write.
            if v.app.emit_to(tauri::EventTarget::webview_window("main"),"aerotab:native-paste-request",serde_json::json!({"sessionId":v.id,"generation":v.generation,"requestId":request,"text":text.to_string()})).is_err() { v.flow.freeze(); v.error = Some(HostError::new("input_bridge","paste event delivery failed")); }
        });
    }
    fn copy(&self) {
        if !self.active() {
            return;
        }
        if let Some(Some(text)) = with_view(self, |v| v.engine.selection_text()) {
            unsafe {
                let pasteboard = NSPasteboard::generalPasteboard();
                pasteboard.clearContents();
                pasteboard.setString_forType(&NSString::from_str(&text), NSPasteboardTypeString);
            }
        }
    }
}
fn key(event: &NSEvent) -> input::Key {
    use input::Key;
    match event.keyCode() {
        36 | 76 => Key::Enter,
        51 => Key::Backspace,
        48 => Key::Tab,
        53 => Key::Escape,
        123 => Key::Left,
        124 => Key::Right,
        125 => Key::Down,
        126 => Key::Up,
        115 => Key::Home,
        119 => Key::End,
        114 => Key::Insert,
        117 => Key::Delete,
        116 => Key::PageUp,
        121 => Key::PageDown,
        122 => Key::Function(1),
        120 => Key::Function(2),
        99 => Key::Function(3),
        118 => Key::Function(4),
        96 => Key::Function(5),
        97 => Key::Function(6),
        98 => Key::Function(7),
        100 => Key::Function(8),
        101 => Key::Function(9),
        109 => Key::Function(10),
        103 => Key::Function(11),
        111 => Key::Function(12),
        105 => Key::Function(13),
        107 => Key::Function(14),
        113 => Key::Function(15),
        106 => Key::Function(16),
        64 => Key::Function(17),
        79 => Key::Function(18),
        80 => Key::Function(19),
        90 => Key::Function(20),
        _ => event
            .charactersIgnoringModifiers()
            .and_then(|s| s.to_string().chars().next())
            .map_or(Key::Unknown, Key::Character),
    }
}
struct Attachment {
    // Explicit Drop releases renderer before removing the NSView / its CAMetalLayer.
    renderer: Option<Renderer>,
    timer: Retained<NSTimer>,
    view: Retained<TerminalView>,
    parent: Retained<NSView>,
    webview: Retained<NSView>,
    id: Uuid,
    generation: Uuid,
    engine: Arc<TerminalEngine>,
    dispatch: InputDispatch,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    keys: std::collections::VecDeque<Retained<NSEvent>>,
    key_epoch: u64,
    key_started: Option<std::time::Instant>,
    drag: Option<(f64, f64)>,
    drag_tick: std::time::Instant,
    backing_scale: f64,
    geometry: Geometry,
    cfg: ViewConfig,
    visible: bool,
    flow: input::InputFlow,
    barrier: macos_policy::Barrier,
    handoff: u64,
    last_seq: Option<u32>,
    error: Option<HostError>,
    cell: (u16, u16),
    grid: Option<(u16, u16, u16, u16)>,
    paste_request: u32,
}
impl Drop for Attachment {
    fn drop(&mut self) {
        self.view.ivars().live.set(false);
        self.barrier.revoke();
        self.flow.freeze();
        self.timer.invalidate();
        if let Some(tracking) = self.view.ivars().tracking.borrow_mut().take() {
            self.view.removeTrackingArea(&tracking);
        }
        self.view.clear_preedit();
        self.view.setHidden(true);
        self.renderer.take();
        self.view.removeFromSuperview();
    }
}
impl Attachment {
    fn place(&self) -> Result<macos_policy::Placement, HostError> {
        let window = self
            .view
            .window()
            .ok_or_else(|| HostError::new("closed_window", "AppKit window is gone"))?;
        let p = converted_placement(
            &self.webview,
            &self.parent,
            self.geometry,
            window.backingScaleFactor(),
        )?;
        self.view.setFrame(NSRect::new(
            NSPoint::new(p.frame[0], p.frame[1]),
            NSSize::new(p.frame[2], p.frame[3]),
        ));
        self.view.setHidden(!self.visible);
        Ok(p)
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
            serde_json::json!({"id":self.id,"cols":cols,"rows":rows}),
        );
    }
    fn draw(&mut self) -> Result<(), HostError> {
        if !self.visible || self.view.isHiddenOrHasHiddenAncestor() {
            return Ok(());
        }
        let Some(window) = self.view.window() else {
            return Ok(());
        };
        if !window.isVisible()
            || window.isMiniaturized()
            || !window
                .occlusionState()
                .contains(NSWindowOcclusionState::Visible)
        {
            self.view.clear_preedit();
            return Ok(());
        }
        let p = self.place()?;
        let scale = window.backingScaleFactor();
        if scale != self.backing_scale {
            // Old-size GPU receipts cannot authorize a handoff after display migration.
            self.backing_scale = scale;
            self.barrier.drawn = None;
            self.last_seq = None;
            if let Some(context) = self.view.inputContext() {
                context.invalidateCharacterCoordinates();
            }
        }
        let backing = self.view.convertRectToBacking(self.view.bounds());
        if (backing.size.width.round() as u32).abs_diff(p.pixels[0]) > 1
            || (backing.size.height.round() as u32).abs_diff(p.pixels[1]) > 1
        {
            return Ok(());
        }
        let envelope = self.engine.native_snapshot_envelope();
        let snapshot = Snapshot::decode(&envelope.bytes, &self.cfg)?;
        let revision = self.barrier.revision;
        let completed = self.renderer.as_mut().unwrap().draw(
            &snapshot,
            &self.cfg,
            p.pixels,
            window.backingScaleFactor(),
            self.cell,
            &self.view.ivars().marked.borrow(),
        )?;
        if self.barrier.complete(
            self.generation.as_u128(),
            revision,
            envelope.output_seq,
            completed,
        ) {
            self.last_seq = Some(snapshot.seq);
        }
        Ok(())
    }
    fn state(&self) -> ViewState {
        ViewState {
            session_id: self.id,
            generation: self.generation,
            geometry: self.geometry,
            visible: self.visible,
            last_draw_seq: self.last_seq,
            revision: self.barrier.revision,
            drawn_revision: self.barrier.drawn.map(|v| v.0),
            handoff_output_seq: self.handoff.to_string(),
            drawn_output_seq: self.barrier.drawn.map(|v| v.1.to_string()),
            input_accepted: self.flow.seq,
            input_ack: self.flow.ack,
            input_frozen: self.flow.frozen,
            error: self.error.clone(),
            backend: "appkit-metal-glyphon",
        }
    }
}
fn converted_placement(
    webview: &NSView,
    parent: &NSView,
    geometry: Geometry,
    scale: f64,
) -> Result<macos_policy::Placement, HostError> {
    let bounds = webview.bounds();
    let local = macos_policy::placement(geometry, bounds.size.height, webview.isFlipped(), scale)?;
    let local = NSRect::new(
        NSPoint::new(
            bounds.origin.x + local.frame[0],
            bounds.origin.y + local.frame[1],
        ),
        NSSize::new(geometry.width, geometry.height),
    );
    let frame = webview.convertRect_toView(local, Some(parent));
    let backing = webview.convertRectToBacking(local);
    Ok(macos_policy::Placement {
        frame: [
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        ],
        pixels: [
            backing.size.width.round().max(1.) as u32,
            backing.size.height.round().max(1.) as u32,
        ],
    })
}
fn main_thread() -> Result<MainThreadMarker, HostError> {
    MainThreadMarker::new()
        .ok_or_else(|| HostError::new("wrong_thread", "AppKit main thread required"))
}
fn missing() -> HostError {
    HostError::new("not_attached", "native view not attached")
}
pub fn attach(
    window: &tauri::WebviewWindow,
    id: Uuid,
    geometry: Geometry,
    cfg: ViewConfig,
    engine: Arc<TerminalEngine>,
    dispatch: InputDispatch,
) -> Result<ViewState, HostError> {
    let mtm = main_thread()?;
    if window.label() != "main" {
        return Err(HostError::new("permission_denied", "main window required"));
    }
    if VIEWS.with(|v| v.borrow().contains_key(&id)) {
        return Err(HostError::new("already_attached", "detach old view first"));
    }
    let ns_window = window
        .ns_window()
        .map_err(|e| HostError::new("appkit_window", e.to_string()))?
        as *mut objc2_app_kit::NSWindow;
    let ns_window = unsafe { ns_window.as_ref() }
        .ok_or_else(|| HostError::new("appkit_window", "null NSWindow"))?;
    let parent = ns_window
        .contentView()
        .ok_or_else(|| HostError::new("appkit_window", "no content view"))?;
    // PlatformWebview::inner is the actual WKWebView; ns_view() is only
    // NSWindow.contentView and must not be used as a guessed CSS origin.
    let raw = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let output = raw.clone();
    window
        .with_webview(move |webview| {
            output.store(
                webview.inner() as usize,
                std::sync::atomic::Ordering::Release,
            )
        })
        .map_err(|e| HostError::new("appkit_webview", e.to_string()))?;
    let webview =
        unsafe { Retained::retain(raw.load(std::sync::atomic::Ordering::Acquire) as *mut NSView) }
            .ok_or_else(|| {
                HostError::new(
                    "appkit_webview",
                    "WKWebView handle was not delivered synchronously on the main thread",
                )
            })?;
    let generation = Uuid::new_v4();
    let allocated = TerminalView::alloc(mtm).set_ivars(ViewIvars {
        id,
        generation,
        live: StateCell::new(true),
        marked: RefCell::default(),
        pointer: RefCell::default(),
        vertical: RefCell::default(),
        horizontal: RefCell::default(),
        tracking: RefCell::default(),
    });
    let view: Retained<TerminalView> = unsafe {
        msg_send![super(allocated), initWithFrame: NSRect::new(NSPoint::new(0.,0.),NSSize::new(1.,1.))]
    };
    let p = converted_placement(&webview, &parent, geometry, ns_window.backingScaleFactor())?;
    // Sibling above WKWebView, not an external window and not WebView rendering.
    // Validate before parenting so failure cannot orphan a retained native child.
    parent.addSubview(&view);
    view.setFrame(NSRect::new(
        NSPoint::new(p.frame[0], p.frame[1]),
        NSSize::new(p.frame[2], p.frame[3]),
    ));
    let renderer = match unsafe { Renderer::new(NonNull::from(&*view).cast(), p.pixels) } {
        Ok(renderer) => renderer,
        Err(e) => {
            view.ivars().live.set(false);
            view.removeFromSuperview();
            return Err(e);
        }
    };
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            1. / 30.,
            &view,
            sel!(tick:),
            None,
            true,
        )
    };
    unsafe {
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
    let handoff = engine.native_snapshot_envelope().output_seq;
    let mut attachment = Attachment {
        renderer: Some(renderer),
        timer,
        view,
        parent,
        webview,
        id,
        generation,
        engine,
        dispatch,
        app: window.app_handle().clone(),
        window: window.clone(),
        keys: std::collections::VecDeque::new(),
        key_epoch: 0,
        key_started: None,
        drag: None,
        drag_tick: std::time::Instant::now(),
        backing_scale: ns_window.backingScaleFactor(),
        geometry,
        cfg,
        visible: true,
        flow: input::InputFlow::default(),
        barrier: macos_policy::Barrier::new(generation.as_u128()),
        handoff,
        last_seq: None,
        error: None,
        cell: (1, 1),
        grid: None,
        paste_request: 0,
    };
    attachment.resize();
    let state = attachment.state();
    VIEWS.with(|v| v.borrow_mut().insert(id, attachment));
    Ok(state)
}
pub fn state(id: Uuid) -> Result<ViewState, HostError> {
    main_thread()?;
    VIEWS.with(|v| {
        v.try_borrow()
            .map_err(|_| HostError::new("host_busy", "reentrant native state query"))?
            .get(&id)
            .map(Attachment::state)
            .ok_or_else(missing)
    })
}
pub fn focus(id: Uuid) -> Result<(), HostError> {
    main_thread()?;
    let view = VIEWS.with(|v| {
        let v = v
            .try_borrow()
            .map_err(|_| HostError::new("host_busy", "reentrant native focus"))?;
        let v = v.get(&id).ok_or_else(missing)?;
        if !v.visible || !v.flow.ready || v.flow.frozen || v.error.is_some() {
            return Err(HostError::new(
                "input_not_ready",
                "inactive or frozen attachment",
            ));
        }
        Ok(v.view.clone())
    })?;
    if !view
        .window()
        .is_some_and(|w| w.makeFirstResponder(Some(&view)))
    {
        return Err(HostError::new("appkit_focus", "first responder rejected"));
    }
    Ok(())
}
pub fn update(
    id: Uuid,
    geometry: Geometry,
    visible: bool,
    cfg: Option<ViewConfig>,
) -> Result<ViewState, HostError> {
    main_thread()?;
    VIEWS.with(|views| {
        let mut views = views
            .try_borrow_mut()
            .map_err(|_| HostError::new("host_busy", "reentrant native host operation"))?;
        let v = views.get_mut(&id).ok_or_else(missing)?;
        let cfg =
            cfg.ok_or_else(|| HostError::new("config_revision", "epoch and revision required"))?;
        if cfg.expected_generation != Some(v.generation) {
            return Err(HostError::new("stale_generation", "attachment replaced"));
        }
        if cfg.input_generation.is_some_and(|g| g != v.generation) || cfg.input_ack > v.flow.seq {
            return Err(HostError::new(
                "input_ack",
                "invalid input generation or acknowledgement",
            ));
        }
        if cfg.revision == v.barrier.revision {
            // Exact idempotent replay only. A new ACK/freeze is a new revision.
            if serde_json::to_value(&cfg).ok() != serde_json::to_value(&v.cfg).ok()
                || serde_json::to_value(geometry).ok() != serde_json::to_value(v.geometry).ok()
                || visible != v.visible
            {
                return Err(HostError::new(
                    "config_revision",
                    "same revision has different payload",
                ));
            }
            return Ok(v.state());
        }
        if !v.barrier.apply(v.generation.as_u128(), cfg.revision) {
            return Err(HostError::new(
                "config_revision",
                "stale configuration revision",
            ));
        }
        if cfg.input_generation == Some(v.generation) {
            if cfg.input_frozen {
                v.flow.freeze();
            }
            v.flow.acknowledge(cfg.input_ready, cfg.input_ack);
        }
        v.cfg = cfg;
        v.geometry = geometry;
        v.visible = visible;
        if !visible || v.flow.frozen {
            v.key_epoch = v.key_epoch.wrapping_add(1);
            v.keys.clear();
            v.drag = None;
            v.view.clear_preedit();
            if let Some(context) = v.view.inputContext() {
                context.discardMarkedText();
            }
        }
        v.resize();
        v.place()?;
        Ok(v.state())
    })
}
pub fn detach(id: Uuid) -> Result<(), HostError> {
    main_thread()?;
    let v = VIEWS
        .with(|v| {
            v.try_borrow_mut()
                .map(|mut views| views.remove(&id))
                .map_err(|_| HostError::new("host_busy", "reentrant native detach"))
        })?
        .ok_or_else(missing)?;
    drop(v);
    Ok(())
}
