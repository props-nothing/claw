//! Hello World — minimal Claw plugin example.
//!
//! Build with: `cargo build --target wasm32-unknown-unknown --release`
//! Then copy `target/wasm32-unknown-unknown/release/hello_world.wasm` to your
//! plugins directory alongside `plugin.toml`.

use std::alloc::{alloc, Layout};

/// Allocate memory in the guest for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn claw_malloc(size: u32) -> u32 {
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { alloc(layout) as u32 }
}

/// Main entry point — the host calls this with a JSON input.
/// Returns a packed u64: (result_ptr << 32) | result_len
#[unsafe(no_mangle)]
pub extern "C" fn claw_invoke(ptr: u32, len: u32) -> u64 {
    let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let input: serde_json::Value = match serde_json::from_slice(input_bytes) {
        Ok(v) => v,
        Err(e) => {
            return write_json(&serde_json::json!({ "error": format!("bad input: {}", e) }));
        }
    };

    let tool = input["tool"].as_str().unwrap_or("");
    let args = &input["arguments"];

    let result = match tool {
        "greet" => {
            let name = args["name"].as_str().unwrap_or("world");
            serde_json::json!({ "result": format!("Hello, {}! 🦞", name) })
        }
        "echo" => {
            let text = args["text"].as_str().unwrap_or("");
            serde_json::json!({ "result": text })
        }
        _ => serde_json::json!({ "error": format!("unknown tool: {}", tool) }),
    };

    write_json(&result)
}

fn write_json(value: &serde_json::Value) -> u64 {
    let json = serde_json::to_string(value).unwrap();
    let bytes = json.as_bytes();
    let layout = Layout::from_size_align(bytes.len(), 1).unwrap();
    let ptr = unsafe { alloc(layout) };
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    ((ptr as u64) << 32) | (bytes.len() as u64)
}
