//! Minimal zi WASM ABI v1 guest (Rust → `wasm32-unknown-unknown`).
//!
//! Exports: `zi_extension_init`, `zi_alloc`, `zi_on_hook`, `zi_call_command`.
//! Imports: module `zi_host` (JSON payloads as `(ptr, len)` into linear memory).

#![no_std]

use core::sync::atomic::{AtomicUsize, Ordering};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}

#[link(wasm_import_module = "zi_host")]
unsafe extern "C" {
    fn host_api_version() -> i32;
    fn register_command(ptr: i32, len: i32);
    fn subscribe_hook(ptr: i32, len: i32);
    fn ui_notify(ptr: i32, len: i32);
    fn emit_log(ptr: i32, len: i32);
}

/// Bump allocator base; leave low addresses for static data / result buffers.
static HEAP: AtomicUsize = AtomicUsize::new(0x1_0000);

fn call_json(f: unsafe extern "C" fn(i32, i32), json: &[u8]) {
    unsafe { f(json.as_ptr() as i32, json.len() as i32) }
}

/// Host → guest payload scratch space (offset into linear memory).
#[no_mangle]
pub extern "C" fn zi_alloc(len: i32) -> i32 {
    let n = len.max(0) as usize;
    HEAP.fetch_add(n, Ordering::Relaxed) as i32
}

/// Register slash + hook; return `0` on success.
#[no_mangle]
pub extern "C" fn zi_extension_init() -> i32 {
    unsafe {
        let _ = host_api_version();
    }
    call_json(
        emit_log,
        br#"{"level":"info","message":"hello-rs extension init"}"#,
    );
    call_json(
        register_command,
        br#"{"name":"hello-rs","summary":"ABI v1 hello from Rust"}"#,
    );
    call_json(subscribe_hook, br#"{"event":"agent_end"}"#);
    0
}

/// `agent_end` (and any subscribed hook): surface a notify.
#[no_mangle]
pub extern "C" fn zi_on_hook(_ptr: i32, _len: i32) -> i32 {
    call_json(
        ui_notify,
        br#"{"message":"hello from rust wasm agent_end"}"#,
    );
    0
}

/// Length-prefixed UTF-8 at return ptr: `u32` LE len + bytes (or `0` for empty).
#[no_mangle]
pub extern "C" fn zi_call_command(_ptr: i32, _len: i32) -> i32 {
    const MSG: &[u8] = b"hello from /hello-rs";
    // Fixed scratch in linear memory (static data segment).
    static mut OUT: [u8; 64] = [0; 64];
    unsafe {
        let len = MSG.len() as u32;
        OUT[0..4].copy_from_slice(&len.to_le_bytes());
        OUT[4..4 + MSG.len()].copy_from_slice(MSG);
        OUT.as_ptr() as i32
    }
}
