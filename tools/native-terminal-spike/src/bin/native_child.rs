use native_terminal_spike::native_input::NativeInput;
use std::collections::VecDeque;
use windows_sys::Win32::{
    Foundation::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::{Ime::*, KeyboardAndMouse::*},
        WindowsAndMessaging::*,
    },
};
use winit::keyboard::{ModifiersState, NamedKey};

#[derive(Default)]
pub struct InputState {
    pub input: NativeInput,
    pub queue: VecDeque<Vec<u8>>,
    pub messages: usize,
    pub ime_results: usize,
    pub app_cursor: bool,
    pub cursor: (i32, i32),
}
fn queue(state: &mut InputState, bytes: Vec<u8>) {
    if !bytes.is_empty() {
        state.queue.push_back(bytes)
    }
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = unsafe { &*(l as *const CREATESTRUCTW) };
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut InputState;
    if pointer.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, w, l) };
    }
    if msg == WM_LBUTTONDOWN {
        unsafe {
            SetFocus(hwnd);
        }
        return 0;
    }
    let s = unsafe { &mut *pointer };
    let gate = unsafe { IsWindowEnabled(hwnd) != 0 && IsWindowVisible(hwnd) != 0 };
    s.input.gate(gate, unsafe { GetFocus() == hwnd });
    match msg {
        WM_SETFOCUS => {
            s.input.gate(gate, true);
            0
        }
        WM_KILLFOCUS => {
            s.input.gate(gate, false);
            0
        }
        WM_GETDLGCODE => (DLGC_WANTALLKEYS | DLGC_WANTCHARS | DLGC_WANTARROWS) as isize,
        WM_CHAR | WM_SYSCHAR => {
            s.messages += 1;
            let mut bytes = s.input.character(w as u16);
            if msg == WM_SYSCHAR && !bytes.is_empty() {
                bytes.insert(0, 27);
            }
            queue(s, bytes);
            0
        }
        WM_UNICHAR => {
            if w == 0xffff {
                return 1;
            }
            s.messages += 1;
            let bytes = s.input.unicode(w as u32);
            queue(s, bytes);
            0
        }
        WM_IME_STARTCOMPOSITION => {
            s.input.composing = true;
            0
        }
        WM_IME_ENDCOMPOSITION => {
            s.input.composing = false;
            0
        }
        WM_IME_CHAR => 0, // Result is consumed once in WM_IME_COMPOSITION, never translated twice.
        WM_IME_COMPOSITION => {
            let context = unsafe { ImmGetContext(hwnd) };
            if !context.is_null() {
                let point = POINT {
                    x: s.cursor.0,
                    y: s.cursor.1,
                };
                let form = CANDIDATEFORM {
                    dwIndex: 0,
                    dwStyle: CFS_CANDIDATEPOS,
                    ptCurrentPos: point,
                    rcArea: RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    },
                };
                let composition = COMPOSITIONFORM {
                    dwStyle: CFS_POINT,
                    ptCurrentPos: point,
                    rcArea: RECT {
                        left: 0,
                        top: 0,
                        right: 0,
                        bottom: 0,
                    },
                };
                unsafe {
                    ImmSetCandidateWindow(context, &form);
                    ImmSetCompositionWindow(context, &composition);
                }
                if l as u32 & GCS_RESULTSTR != 0 {
                    let length = unsafe {
                        ImmGetCompositionStringW(context, GCS_RESULTSTR, std::ptr::null_mut(), 0)
                    };
                    if length > 0 && length <= 65536 {
                        let mut units = vec![0u16; length as usize / 2];
                        unsafe {
                            ImmGetCompositionStringW(
                                context,
                                GCS_RESULTSTR,
                                units.as_mut_ptr().cast(),
                                length as u32,
                            );
                        }
                        let bytes = s.input.commit(&units);
                        queue(s, bytes);
                        s.ime_results += 1;
                    }
                } else if l as u32 & GCS_COMPSTR != 0 {
                    s.input.composing = true;
                }
                unsafe {
                    ImmReleaseContext(hwnd, context);
                }
            }
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let key = match w as u16 {
                VK_UP => Some(NamedKey::ArrowUp),
                VK_DOWN => Some(NamedKey::ArrowDown),
                VK_LEFT => Some(NamedKey::ArrowLeft),
                VK_RIGHT => Some(NamedKey::ArrowRight),
                VK_HOME => Some(NamedKey::Home),
                VK_END => Some(NamedKey::End),
                VK_DELETE => Some(NamedKey::Delete),
                VK_INSERT => Some(NamedKey::Insert),
                VK_PRIOR => Some(NamedKey::PageUp),
                VK_NEXT => Some(NamedKey::PageDown),
                _ => None,
            };
            if let Some(key) = key {
                let mut mods = ModifiersState::empty();
                for (vk, flag) in [
                    (VK_SHIFT, ModifiersState::SHIFT),
                    (VK_CONTROL, ModifiersState::CONTROL),
                    (VK_MENU, ModifiersState::ALT),
                ] {
                    if unsafe { GetKeyState(vk as i32) } < 0 {
                        mods |= flag
                    }
                }
                let bytes = s.input.key(key, mods, s.app_cursor);
                queue(s, bytes);
                0
            } else {
                unsafe { DefWindowProcW(hwnd, msg, w, l) }
            }
        }
        WM_NCDESTROY => unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, w, l)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

pub fn create(parent: HWND, state: &mut InputState) -> Result<HWND, Box<dyn std::error::Error>> {
    let name: Vec<u16> = "AeroTabNativeTerminalSpike\0".encode_utf16().collect();
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
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_TABSTOP,
            20,
            84,
            500,
            400,
            parent,
            std::ptr::null_mut(),
            module,
            (state as *mut InputState).cast(),
        )
    };
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(hwnd)
}
