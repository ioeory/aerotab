//! Minimal Tabby WASM plugin demonstrating the host ABI.
//!
//! Build with: `cargo build --target wasm32-unknown-unknown --release`
//! The artifact lands at:
//!   `target/wasm32-unknown-unknown/release/hello_wasm.wasm`
//! Drop it in `<app-data>/plugins/` and restart, or invoke `plugin.load`.

extern "C" {
    fn host_log(level: i32, ptr: i32, len: i32);
    fn host_time_unix_ms() -> i64;
}

fn log(level: i32, msg: &str) {
    unsafe { host_log(level, msg.as_ptr() as i32, msg.len() as i32) };
}

/// Host calls this once at load time; non-zero return aborts the load.
#[no_mangle]
pub extern "C" fn plugin_init() -> i32 {
    log(2, "hello_wasm: init");
    0
}

/// Allocate `size` bytes; host writes input buffers here.
#[no_mangle]
pub extern "C" fn plugin_alloc(size: i32) -> i32 {
    let mut buf = Vec::<u8>::with_capacity(size as usize);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr as i32
}

/// Free a buffer previously returned by `plugin_alloc` or `plugin_invoke`.
#[no_mangle]
pub extern "C" fn plugin_free(ptr: i32, size: i32) {
    if ptr == 0 {
        return;
    }
    unsafe {
        let _ = Vec::from_raw_parts(ptr as *mut u8, 0, size as usize);
    }
}

/// Dispatch `command` with the UTF-8 `args` payload.
/// Returns a packed `(ptr << 32) | len` pointing at a `plugin_alloc`-managed
/// UTF-8 reply that the host will hand back to `plugin_free` after copying.
#[no_mangle]
pub extern "C" fn plugin_invoke(
    cmd_ptr: i32,
    cmd_len: i32,
    args_ptr: i32,
    args_len: i32,
) -> i64 {
    let cmd =
        unsafe { core::slice::from_raw_parts(cmd_ptr as *const u8, cmd_len as usize) };
    let args =
        unsafe { core::slice::from_raw_parts(args_ptr as *const u8, args_len as usize) };
    let cmd = core::str::from_utf8(cmd).unwrap_or("");
    let args = core::str::from_utf8(args).unwrap_or("");

    let reply = match cmd {
        "say-hi" => format!("Hello, {}!", if args.is_empty() { "world" } else { args }),
        "time" => format!("{}", unsafe { host_time_unix_ms() }),
        other => format!("unknown command: {}", other),
    };

    let bytes = reply.into_bytes();
    let len = bytes.len() as i32;
    let ptr = plugin_alloc(len);
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, len as usize) };
    ((ptr as i64) << 32) | (len as i64 & 0xffff_ffff)
}
