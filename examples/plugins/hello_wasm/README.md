# hello_wasm

Minimal Tabby WASM plugin demo.

## Build

```bash
rustup target add wasm32-unknown-unknown
cd examples/plugins/hello_wasm
cargo build --target wasm32-unknown-unknown --release
```

Artifact: `target/wasm32-unknown-unknown/release/hello_wasm.wasm`

## Install

Copy the `.wasm` into your Tabby data dir under `plugins/`, e.g.:

- Linux:   `~/.local/share/com.tabby.app/plugins/hello_wasm.wasm`
- Windows: `%APPDATA%\com.tabby.app\plugins\hello_wasm.wasm`

Or load on demand via JSON-RPC:

```jsonc
{"jsonrpc":"2.0","id":1,"method":"plugin.load","params":{"path":"/abs/path/to/hello_wasm.wasm"}}
```

## Invoke

```jsonc
{"jsonrpc":"2.0","id":2,"method":"plugin.invoke","params":{"name":"hello_wasm","command":"say-hi","args":"Tabby"}}
// → {"result":"Hello, Tabby!"}
{"jsonrpc":"2.0","id":3,"method":"plugin.invoke","params":{"name":"hello_wasm","command":"time","args":""}}
// → {"result":"1734…"}
```

## ABI summary

The host expects four exported functions:

| Export          | Signature                              | Purpose                                  |
|-----------------|----------------------------------------|------------------------------------------|
| `plugin_init`   | `() -> i32` (optional)                 | Called once; non-zero aborts load.       |
| `plugin_alloc`  | `(i32) -> i32`                         | Allocate `n` bytes, return pointer.      |
| `plugin_free`   | `(i32, i32)`                           | Free a buffer.                           |
| `plugin_invoke` | `(i32, i32, i32, i32) -> i64`          | Returns `(ptr << 32) | len` of reply.    |

The host provides two imports under module `tabby`:

| Import              | Signature                | Purpose                       |
|---------------------|--------------------------|-------------------------------|
| `host_log`          | `(i32 level, i32 ptr, i32 len)` | Emit a tracing log line. |
| `host_time_unix_ms` | `() -> i64`              | Wall-clock millis since 1970. |
