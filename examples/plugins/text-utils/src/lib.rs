//! text-utils — Claw WASM plugin for text & data processing.
//!
//! Provides 12 tools: base64 encode/decode, hashing (SHA-256/SHA-1/MD5),
//! UUID v4 generation, timestamp formatting, regex match/replace,
//! JSON query/format, text stats, and URL encode/decode.
//!
//! Build: `cargo build --target wasm32-unknown-unknown --release`

use std::alloc::{alloc, Layout};
use std::fmt::Write as FmtWrite;

use sha2::{Sha256, Digest};
use sha1::Sha1;
use md5::Md5;
use regex::Regex;

// ═══════════════════════════════════════════════════════════════
//  WASM ABI exports
// ═══════════════════════════════════════════════════════════════

/// Allocate memory in the guest for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn claw_malloc(size: u32) -> u32 {
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { alloc(layout) as u32 }
}

/// Main entry point called by the host with JSON input.
/// Returns packed u64: (result_ptr << 32) | result_len
#[unsafe(no_mangle)]
pub extern "C" fn claw_invoke(ptr: u32, len: u32) -> u64 {
    let input_bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let input: serde_json::Value = match serde_json::from_slice(input_bytes) {
        Ok(v) => v,
        Err(e) => return write_json(&err_json(&format!("bad input: {e}"))),
    };

    let tool = input["tool"].as_str().unwrap_or("");
    let args = &input["arguments"];

    let result = match tool {
        "base64_encode"  => tool_base64_encode(args),
        "base64_decode"  => tool_base64_decode(args),
        "hash"           => tool_hash(args),
        "uuid"           => tool_uuid(args),
        "timestamp"      => tool_timestamp(args),
        "regex_match"    => tool_regex_match(args),
        "regex_replace"  => tool_regex_replace(args),
        "json_query"     => tool_json_query(args),
        "json_format"    => tool_json_format(args),
        "text_stats"     => tool_text_stats(args),
        "url_encode"     => tool_url_encode(args),
        "url_decode"     => tool_url_decode(args),
        _ => err_json(&format!("unknown tool: {tool}")),
    };

    write_json(&result)
}

// ═══════════════════════════════════════════════════════════════
//  Base64
// ═══════════════════════════════════════════════════════════════

fn tool_base64_encode(args: &serde_json::Value) -> serde_json::Value {
    let text = args["text"].as_str().unwrap_or("");
    let encoded = base64_encode(text.as_bytes());
    serde_json::json!({ "result": encoded })
}

fn tool_base64_decode(args: &serde_json::Value) -> serde_json::Value {
    let encoded = args["encoded"].as_str().unwrap_or("");
    match base64_decode(encoded) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => serde_json::json!({ "result": text }),
            Err(_) => err_json("decoded bytes are not valid UTF-8"),
        },
        Err(e) => err_json(&e),
    }
}

/// Simple base64 encoder (no external crate needed at runtime).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Simple base64 decoder.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u32, String> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 character: {}", c as char)),
        }
    }

    let input = input.trim();
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'\n' && b != b'\r').collect();
    if bytes.len() % 4 != 0 {
        return Err("invalid base64 length".into());
    }

    let mut result = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = (chunk[2] == b'=') as usize + (chunk[3] == b'=') as usize;
        let a = val(chunk[0])?;
        let b = val(chunk[1])?;
        let c = if chunk[2] != b'=' { val(chunk[2])? } else { 0 };
        let d = if chunk[3] != b'=' { val(chunk[3])? } else { 0 };
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        result.push((triple >> 16) as u8);
        if pad < 2 { result.push((triple >> 8) as u8); }
        if pad < 1 { result.push(triple as u8); }
    }
    Ok(result)
}

// ═══════════════════════════════════════════════════════════════
//  Hashing
// ═══════════════════════════════════════════════════════════════

fn tool_hash(args: &serde_json::Value) -> serde_json::Value {
    let text = args["text"].as_str().unwrap_or("");
    let algo = args["algorithm"].as_str().unwrap_or("sha256");

    let hex = match algo {
        "sha256" | "SHA256" | "SHA-256" => {
            let mut h = Sha256::new();
            h.update(text.as_bytes());
            hex_encode(&h.finalize())
        }
        "sha1" | "SHA1" | "SHA-1" => {
            let mut h = Sha1::new();
            h.update(text.as_bytes());
            hex_encode(&h.finalize())
        }
        "md5" | "MD5" => {
            let mut h = Md5::new();
            h.update(text.as_bytes());
            hex_encode(&h.finalize())
        }
        _ => return err_json(&format!("unsupported algorithm: {algo}. Use sha256, sha1, or md5")),
    };

    serde_json::json!({
        "result": format!("{algo}: {hex}"),
        "data": { "hex": hex, "algorithm": algo }
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

// ═══════════════════════════════════════════════════════════════
//  UUID v4 (deterministic PRNG seeded from input pointer — good enough for IDs)
// ═══════════════════════════════════════════════════════════════

fn tool_uuid(_args: &serde_json::Value) -> serde_json::Value {
    // Simple xorshift64 PRNG seeded from the stack pointer address
    // (different each WASM instantiation due to memory layout)
    let mut seed = (&_args as *const _ as u64)
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);

    let mut bytes = [0u8; 16];
    for b in &mut bytes {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *b = seed as u8;
    }
    // Set version (4) and variant (RFC 4122)
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;

    let hex = hex_encode(&bytes);
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32]
    );

    serde_json::json!({ "result": uuid, "data": { "uuid": uuid } })
}

// ═══════════════════════════════════════════════════════════════
//  Timestamp formatting
// ═══════════════════════════════════════════════════════════════

fn tool_timestamp(args: &serde_json::Value) -> serde_json::Value {
    let epoch = args["epoch"].as_f64();
    let offset_hours = args["offset_hours"].as_f64().unwrap_or(0.0);
    let offset_secs = (offset_hours * 3600.0) as i64;

    match epoch {
        Some(ts) => {
            let ts_i = ts as i64 + offset_secs;
            let iso = unix_to_iso(ts_i, offset_hours);
            serde_json::json!({
                "result": iso,
                "data": { "iso": iso, "epoch": ts as i64, "offset_hours": offset_hours }
            })
        }
        None => {
            serde_json::json!({
                "result": "Provide an 'epoch' parameter (UNIX seconds). Example: 1700000000 → 2023-11-14T22:13:20Z",
                "data": { "example_epoch": 1700000000, "example_iso": "2023-11-14T22:13:20Z" }
            })
        }
    }
}

/// Convert UNIX timestamp to ISO-8601 string.
fn unix_to_iso(secs: i64, offset_hours: f64) -> String {
    // Days from epoch to year, month, day (civil calendar algorithm)
    let days = secs.div_euclid(86400);
    let time_of_day = secs.rem_euclid(86400);

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Algorithm from Howard Hinnant's chrono-compatible date library
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    if offset_hours == 0.0 {
        format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
    } else {
        let sign = if offset_hours >= 0.0 { '+' } else { '-' };
        let oh = offset_hours.abs() as i64;
        let om = ((offset_hours.abs() - oh as f64) * 60.0) as i64;
        format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}{sign}{oh:02}:{om:02}")
    }
}

// ═══════════════════════════════════════════════════════════════
//  Regex tools
// ═══════════════════════════════════════════════════════════════

fn tool_regex_match(args: &serde_json::Value) -> serde_json::Value {
    let text = args["text"].as_str().unwrap_or("");
    let pattern = args["pattern"].as_str().unwrap_or("");

    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => return err_json(&format!("invalid regex: {e}")),
    };

    match re.captures(text) {
        Some(caps) => {
            let full_match = caps.get(0).map(|m| m.as_str()).unwrap_or("");
            let groups: Vec<&str> = caps
                .iter()
                .skip(1)
                .map(|m| m.map(|m| m.as_str()).unwrap_or(""))
                .collect();
            serde_json::json!({
                "result": format!("Match found: {full_match}"),
                "data": {
                    "matched": true,
                    "full_match": full_match,
                    "groups": groups,
                }
            })
        }
        None => serde_json::json!({
            "result": "No match",
            "data": { "matched": false }
        }),
    }
}

fn tool_regex_replace(args: &serde_json::Value) -> serde_json::Value {
    let text = args["text"].as_str().unwrap_or("");
    let pattern = args["pattern"].as_str().unwrap_or("");
    let replacement = args["replacement"].as_str().unwrap_or("");

    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => return err_json(&format!("invalid regex: {e}")),
    };

    let result = re.replace_all(text, replacement).to_string();
    let count = re.find_iter(text).count();

    serde_json::json!({
        "result": result,
        "data": { "replacements": count, "output": result }
    })
}

// ═══════════════════════════════════════════════════════════════
//  JSON tools
// ═══════════════════════════════════════════════════════════════

fn tool_json_query(args: &serde_json::Value) -> serde_json::Value {
    let json_str = args["json"].as_str().unwrap_or("{}");
    let path = args["path"].as_str().unwrap_or("");

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => return err_json(&format!("invalid JSON: {e}")),
    };

    let mut current = &parsed;
    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }
        // Try array index first
        if let Ok(idx) = segment.parse::<usize>() {
            if let Some(v) = current.get(idx) {
                current = v;
                continue;
            }
        }
        // Then object key
        if let Some(v) = current.get(segment) {
            current = v;
        } else {
            return serde_json::json!({
                "result": format!("path not found: key '{segment}' does not exist"),
                "data": { "found": false, "path": path }
            });
        }
    }

    let display = match current {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };

    serde_json::json!({
        "result": display,
        "data": { "found": true, "value": current }
    })
}

fn tool_json_format(args: &serde_json::Value) -> serde_json::Value {
    let json_str = args["json"].as_str().unwrap_or("{}");
    let minify = args["minify"].as_bool().unwrap_or(false);

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => return err_json(&format!("invalid JSON: {e}")),
    };

    let formatted = if minify {
        serde_json::to_string(&parsed).unwrap_or_default()
    } else {
        serde_json::to_string_pretty(&parsed).unwrap_or_default()
    };

    serde_json::json!({ "result": formatted })
}

// ═══════════════════════════════════════════════════════════════
//  Text stats
// ═══════════════════════════════════════════════════════════════

fn tool_text_stats(args: &serde_json::Value) -> serde_json::Value {
    let text = args["text"].as_str().unwrap_or("");

    let chars = text.chars().count();
    let bytes = text.len();
    let words = text.split_whitespace().count();
    let lines = if text.is_empty() { 0 } else { text.lines().count() };

    // Simple sentence count: count sentence-ending punctuation followed by space or end
    let has_alpha = text.chars().any(|c| c.is_alphanumeric());
    let text_chars: Vec<char> = text.chars().collect();
    let mut sentence_count = 0usize;
    for (i, &c) in text_chars.iter().enumerate() {
        if matches!(c, '.' | '!' | '?') {
            let next = text_chars.get(i + 1).copied().unwrap_or(' ');
            if next.is_whitespace() || next == '"' {
                sentence_count += 1;
            }
        }
    }
    if sentence_count == 0 && has_alpha {
        sentence_count = 1; // Text with words but no punctuation counts as 1 sentence
    }
    let sentences = sentence_count;

    serde_json::json!({
        "result": format!("{chars} chars, {words} words, {lines} lines, {sentences} sentences ({bytes} bytes)"),
        "data": { "characters": chars, "words": words, "lines": lines, "sentences": sentences, "bytes": bytes }
    })
}

// ═══════════════════════════════════════════════════════════════
//  URL encode / decode
// ═══════════════════════════════════════════════════════════════

fn tool_url_encode(args: &serde_json::Value) -> serde_json::Value {
    let text = args["text"].as_str().unwrap_or("");
    let mut encoded = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                let _ = write!(encoded, "%{b:02X}");
            }
        }
    }
    serde_json::json!({ "result": encoded })
}

fn tool_url_decode(args: &serde_json::Value) -> serde_json::Value {
    let text = args["text"].as_str().unwrap_or("");
    let mut decoded = Vec::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(
                &String::from_utf8_lossy(&bytes[i + 1..i + 3]),
                16,
            ) {
                decoded.push(val);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            decoded.push(b' ');
        } else {
            decoded.push(bytes[i]);
        }
        i += 1;
    }
    match String::from_utf8(decoded) {
        Ok(s) => serde_json::json!({ "result": s }),
        Err(_) => err_json("decoded bytes are not valid UTF-8"),
    }
}

// ═══════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════

fn err_json(msg: &str) -> serde_json::Value {
    serde_json::json!({ "error": msg })
}

fn write_json(value: &serde_json::Value) -> u64 {
    let json = serde_json::to_string(value).unwrap();
    let bytes = json.as_bytes();
    let layout = Layout::from_size_align(bytes.len(), 1).unwrap();
    let ptr = unsafe { alloc(layout) };
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    ((ptr as u64) << 32) | (bytes.len() as u64)
}
