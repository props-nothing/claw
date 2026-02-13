//! Calculator — Claw plugin for math operations.
//!
//! Demonstrates a more complex plugin with multiple tools and
//! actual computation logic.

use std::alloc::{alloc, Layout};

#[unsafe(no_mangle)]
pub extern "C" fn claw_malloc(size: u32) -> u32 {
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { alloc(layout) as u32 }
}

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
        "calculate" => {
            let expr = args["expression"].as_str().unwrap_or("");
            match eval_simple(expr) {
                Ok(val) => serde_json::json!({
                    "result": format!("{}", val),
                    "data": { "value": val }
                }),
                Err(e) => serde_json::json!({ "error": e }),
            }
        }
        "convert" => {
            let value = args["value"].as_f64().unwrap_or(0.0);
            let from = args["from"].as_str().unwrap_or("");
            let to = args["to"].as_str().unwrap_or("");
            match convert_units(value, from, to) {
                Ok(result) => serde_json::json!({
                    "result": format!("{} {} = {} {}", value, from, result, to),
                    "data": { "value": result }
                }),
                Err(e) => serde_json::json!({ "error": e }),
            }
        }
        _ => serde_json::json!({ "error": format!("unknown tool: {}", tool) }),
    };

    write_json(&result)
}

/// Very simple expression evaluator (supports +, -, *, /, parentheses).
fn eval_simple(expr: &str) -> Result<f64, String> {
    let tokens: Vec<char> = expr.chars().filter(|c| !c.is_whitespace()).collect();
    let mut pos = 0;
    let result = parse_expr(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(format!("unexpected character at position {}", pos));
    }
    Ok(result)
}

fn parse_expr(tokens: &[char], pos: &mut usize) -> Result<f64, String> {
    let mut left = parse_term(tokens, pos)?;
    while *pos < tokens.len() && (tokens[*pos] == '+' || tokens[*pos] == '-') {
        let op = tokens[*pos];
        *pos += 1;
        let right = parse_term(tokens, pos)?;
        left = if op == '+' { left + right } else { left - right };
    }
    Ok(left)
}

fn parse_term(tokens: &[char], pos: &mut usize) -> Result<f64, String> {
    let mut left = parse_factor(tokens, pos)?;
    while *pos < tokens.len() && (tokens[*pos] == '*' || tokens[*pos] == '/') {
        let op = tokens[*pos];
        *pos += 1;
        let right = parse_factor(tokens, pos)?;
        left = if op == '*' { left * right } else { left / right };
    }
    Ok(left)
}

fn parse_factor(tokens: &[char], pos: &mut usize) -> Result<f64, String> {
    if *pos >= tokens.len() {
        return Err("unexpected end of expression".into());
    }

    if tokens[*pos] == '(' {
        *pos += 1;
        let result = parse_expr(tokens, pos)?;
        if *pos >= tokens.len() || tokens[*pos] != ')' {
            return Err("missing closing parenthesis".into());
        }
        *pos += 1;
        return Ok(result);
    }

    if tokens[*pos] == '-' {
        *pos += 1;
        return Ok(-parse_factor(tokens, pos)?);
    }

    // Parse number
    let start = *pos;
    while *pos < tokens.len() && (tokens[*pos].is_ascii_digit() || tokens[*pos] == '.') {
        *pos += 1;
    }
    if start == *pos {
        return Err(format!("expected number at position {}", start));
    }
    let num_str: String = tokens[start..*pos].iter().collect();
    num_str.parse::<f64>().map_err(|e| format!("invalid number: {}", e))
}

fn convert_units(value: f64, from: &str, to: &str) -> Result<f64, String> {
    match (from.to_lowercase().as_str(), to.to_lowercase().as_str()) {
        ("km", "miles") | ("kilometers", "miles") => Ok(value * 0.621371),
        ("miles", "km") | ("miles", "kilometers") => Ok(value * 1.60934),
        ("celsius", "fahrenheit") | ("c", "f") => Ok(value * 9.0 / 5.0 + 32.0),
        ("fahrenheit", "celsius") | ("f", "c") => Ok((value - 32.0) * 5.0 / 9.0),
        ("kg", "lbs") | ("kilograms", "pounds") => Ok(value * 2.20462),
        ("lbs", "kg") | ("pounds", "kilograms") => Ok(value * 0.453592),
        ("m", "ft") | ("meters", "feet") => Ok(value * 3.28084),
        ("ft", "m") | ("feet", "meters") => Ok(value * 0.3048),
        _ => Err(format!("unsupported conversion: {} to {}", from, to)),
    }
}

fn write_json(value: &serde_json::Value) -> u64 {
    let json = serde_json::to_string(value).unwrap();
    let bytes = json.as_bytes();
    let layout = Layout::from_size_align(bytes.len(), 1).unwrap();
    let ptr = unsafe { alloc(layout) };
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len()) };
    ((ptr as u64) << 32) | (bytes.len() as u64)
}
