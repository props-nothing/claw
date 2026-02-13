# Example Plugins

Example WASM plugins for the Claw plugin system.

## Plugin ABI

Claw plugins are WebAssembly modules that export:

- `memory` — linear memory
- `claw_malloc(size: u32) -> u32` — allocate bytes in guest memory
- `claw_invoke(ptr: u32, len: u32) -> u64` — invoke a tool

### Input (JSON written to guest memory)

```json
{ "tool": "tool_name", "arguments": { ... } }
```

### Output (packed as `(ptr << 32) | len` pointing to JSON)

```json
{ "result": "text result", "data": { ... } }
```

Or on error:

```json
{ "error": "error message" }
```

## Building

```bash
# Install the WASM target
rustup target add wasm32-unknown-unknown

# Build a plugin
cd hello-world
cargo build --target wasm32-unknown-unknown --release

# Copy the .wasm to the Claw plugins directory
cp target/wasm32-unknown-unknown/release/hello_world.wasm ~/.claw/plugins/hello-world/
```

## Creating a New Plugin

Use the CLI scaffold command:

```bash
claw plugin create my-plugin
```
