//! WASM plugin host (pure-Rust interpreter, no JIT).
//!
//! ## ABI
//!
//! Plugins are `wasm32-unknown-unknown` modules (Rust `cdylib`, C,
//! AssemblyScript, …) that export the following functions:
//!
//! ```text
//! plugin_alloc(len: i32) -> i32           // host calls this to obtain a buffer
//! plugin_free(ptr: i32, len: i32)         // host releases plugin-owned buffers
//! plugin_init() -> i32                    // optional; 0 = OK
//! plugin_invoke(
//!   cmd_ptr: i32, cmd_len: i32,
//!   args_ptr: i32, args_len: i32,
//! ) -> i64                                // packed (ptr<<32 | len), UTF-8 result
//! ```
//!
//! Plugins may import these host functions from module `"aerotab"`:
//!
//! ```text
//! host_log(level: i32, ptr: i32, len: i32)   // 0=trace 1=debug 2=info 3=warn 4=error
//! host_time_unix_ms() -> i64
//! ```
//!
//! wasmi is sandboxed: no filesystem, no network, no syscalls. All data
//! crossing the boundary is UTF-8 in linear memory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;
use wasmi::{Caller, Engine, Extern, Func, Linker, Memory, Module, Store};

#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("wasm: {0}")]
    Wasm(String),
    #[error("plugin missing export `{0}`")]
    MissingExport(&'static str),
    #[error("plugin returned error code {0}")]
    InitFailed(i32),
    #[error("plugin not found: {0}")]
    NotFound(String),
    #[error("plugin returned invalid pointer/length")]
    BadResult,
}

fn wasm_err<E: std::fmt::Display>(e: E) -> WasmError {
    WasmError::Wasm(e.to_string())
}

pub struct PluginState {
    pub name: String,
}

struct LoadedPlugin {
    name: String,
    path: PathBuf,
    // wasmi `Store` executes calls serially; that's fine since plugin
    // invocations are expected to be short-lived & request-scoped.
    store: Mutex<Store<PluginState>>,
    memory: Memory,
    alloc: Func,
    free: Func,
    invoke: Func,
}

impl LoadedPlugin {
    fn write_buffer(
        &self,
        store: &mut Store<PluginState>,
        data: &[u8],
    ) -> Result<(i32, i32), WasmError> {
        let len = data.len() as i32;
        let mut out = [wasmi::Val::I32(0)];
        self.alloc
            .call(&mut *store, &[wasmi::Val::I32(len)], &mut out)
            .map_err(wasm_err)?;
        let ptr = match out[0] {
            wasmi::Val::I32(v) => v,
            _ => return Err(WasmError::BadResult),
        };
        if ptr == 0 && len != 0 {
            return Err(WasmError::Wasm("plugin alloc returned null".into()));
        }
        let mem = self.memory.data_mut(&mut *store);
        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or(WasmError::BadResult)?;
        if end > mem.len() {
            return Err(WasmError::BadResult);
        }
        mem[start..end].copy_from_slice(data);
        Ok((ptr, len))
    }

    fn read_result(
        &self,
        store: &mut Store<PluginState>,
        packed: i64,
    ) -> Result<String, WasmError> {
        let ptr = (packed >> 32) as i32;
        let len = (packed & 0xffff_ffff) as i32;
        if len == 0 {
            return Ok(String::new());
        }
        if ptr <= 0 || len < 0 {
            return Err(WasmError::BadResult);
        }
        let s = {
            let mem = self.memory.data(&*store);
            let start = ptr as usize;
            let end = start
                .checked_add(len as usize)
                .ok_or(WasmError::BadResult)?;
            if end > mem.len() {
                return Err(WasmError::BadResult);
            }
            std::str::from_utf8(&mem[start..end])
                .map_err(|e| WasmError::Wasm(format!("utf8: {e}")))?
                .to_string()
        };
        // Best-effort free; a misbehaving plugin can't poison the host.
        let _ = self.free.call(
            &mut *store,
            &[wasmi::Val::I32(ptr), wasmi::Val::I32(len)],
            &mut [],
        );
        Ok(s)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub path: String,
}

#[derive(Default)]
pub struct WasmHost {
    plugins: Mutex<HashMap<String, Arc<LoadedPlugin>>>,
    dir: Mutex<Option<PathBuf>>,
}

impl WasmHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind a plugins directory and load every `*.wasm` inside it. Returns
    /// the number of plugins successfully loaded.
    pub async fn load_dir(&self, dir: &Path) -> Result<usize, WasmError> {
        *self.dir.lock().await = Some(dir.to_path_buf());
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
        let mut count = 0usize;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "wasm") {
                match self.load_file(&path).await {
                    Ok(name) => {
                        tracing::info!(plugin = %name, "wasm plugin loaded");
                        count += 1;
                    }
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "plugin load failed");
                    }
                }
            }
        }
        Ok(count)
    }

    /// Load a single .wasm file. The plugin name is the file stem.
    pub async fn load_file(&self, path: &Path) -> Result<String, WasmError> {
        let bytes = std::fs::read(path)?;
        self.load_bytes(
            path.file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| WasmError::Wasm("invalid plugin filename".into()))?,
            &bytes,
            path,
        )
        .await
    }

    /// Load a plugin from raw wasm bytes (used by `load_file` and tests).
    pub async fn load_bytes(
        &self,
        name: &str,
        bytes: &[u8],
        source_path: &Path,
    ) -> Result<String, WasmError> {
        let engine = Engine::default();
        let module = Module::new(&engine, bytes).map_err(wasm_err)?;
        let mut store = Store::new(
            &engine,
            PluginState {
                name: name.to_string(),
            },
        );

        let mut linker = <Linker<PluginState>>::new(&engine);

        linker
            .func_wrap(
                "aerotab",
                "host_log",
                |caller: Caller<'_, PluginState>, level: i32, ptr: i32, len: i32| {
                    let Some(Extern::Memory(mem)) = caller.get_export("memory") else {
                        return;
                    };
                    let data = mem.data(&caller);
                    let start = ptr as usize;
                    let end = start.saturating_add(len as usize);
                    if end > data.len() {
                        return;
                    }
                    let msg = String::from_utf8_lossy(&data[start..end]).into_owned();
                    let plugin = caller.data().name.clone();
                    match level {
                        0 => tracing::trace!(target: "wasm_plugin", plugin = %plugin, "{msg}"),
                        1 => tracing::debug!(target: "wasm_plugin", plugin = %plugin, "{msg}"),
                        2 => tracing::info!(target: "wasm_plugin", plugin = %plugin, "{msg}"),
                        3 => tracing::warn!(target: "wasm_plugin", plugin = %plugin, "{msg}"),
                        _ => tracing::error!(target: "wasm_plugin", plugin = %plugin, "{msg}"),
                    }
                },
            )
            .map_err(wasm_err)?;

        linker
            .func_wrap("aerotab", "host_time_unix_ms", || -> i64 {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0)
            })
            .map_err(wasm_err)?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(wasm_err)?
            .start(&mut store)
            .map_err(wasm_err)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or(WasmError::MissingExport("memory"))?;
        let alloc = instance
            .get_func(&mut store, "plugin_alloc")
            .ok_or(WasmError::MissingExport("plugin_alloc"))?;
        let free = instance
            .get_func(&mut store, "plugin_free")
            .ok_or(WasmError::MissingExport("plugin_free"))?;
        let invoke = instance
            .get_func(&mut store, "plugin_invoke")
            .ok_or(WasmError::MissingExport("plugin_invoke"))?;

        // Optional init.
        if let Some(init) = instance.get_func(&mut store, "plugin_init") {
            let mut out = [wasmi::Val::I32(0)];
            init.call(&mut store, &[], &mut out).map_err(wasm_err)?;
            if let wasmi::Val::I32(code) = out[0] {
                if code != 0 {
                    return Err(WasmError::InitFailed(code));
                }
            }
        }

        let plugin = Arc::new(LoadedPlugin {
            name: name.to_string(),
            path: source_path.to_path_buf(),
            store: Mutex::new(store),
            memory,
            alloc,
            free,
            invoke,
        });
        self.plugins.lock().await.insert(name.to_string(), plugin);
        Ok(name.to_string())
    }

    /// Reload every plugin from the bound directory. Useful for dev iteration.
    pub async fn reload(&self) -> Result<usize, WasmError> {
        let dir = self
            .dir
            .lock()
            .await
            .clone()
            .ok_or_else(|| WasmError::Wasm("plugins directory not configured".into()))?;
        self.plugins.lock().await.clear();
        self.load_dir(&dir).await
    }

    pub async fn list(&self) -> Vec<PluginInfo> {
        let mut v: Vec<PluginInfo> = self
            .plugins
            .lock()
            .await
            .values()
            .map(|p| PluginInfo {
                name: p.name.clone(),
                path: p.path.display().to_string(),
            })
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Invoke `command` on `name` with a UTF-8 `args` blob; returns the
    /// plugin's UTF-8 reply (may be JSON, plain text, anything).
    pub async fn invoke(&self, name: &str, command: &str, args: &str) -> Result<String, WasmError> {
        let plugin = {
            let guard = self.plugins.lock().await;
            guard
                .get(name)
                .cloned()
                .ok_or_else(|| WasmError::NotFound(name.to_string()))?
        };
        let mut store = plugin.store.lock().await;
        let (cmd_ptr, cmd_len) = plugin.write_buffer(&mut store, command.as_bytes())?;
        let (args_ptr, args_len) = plugin.write_buffer(&mut store, args.as_bytes())?;

        let mut out = [wasmi::Val::I64(0)];
        plugin
            .invoke
            .call(
                &mut *store,
                &[
                    wasmi::Val::I32(cmd_ptr),
                    wasmi::Val::I32(cmd_len),
                    wasmi::Val::I32(args_ptr),
                    wasmi::Val::I32(args_len),
                ],
                &mut out,
            )
            .map_err(wasm_err)?;
        // Free the input buffers — plugin is done with them now.
        let _ = plugin.free.call(
            &mut *store,
            &[wasmi::Val::I32(cmd_ptr), wasmi::Val::I32(cmd_len)],
            &mut [],
        );
        let _ = plugin.free.call(
            &mut *store,
            &[wasmi::Val::I32(args_ptr), wasmi::Val::I32(args_len)],
            &mut [],
        );

        let packed = match out[0] {
            wasmi::Val::I64(v) => v,
            _ => return Err(WasmError::BadResult),
        };
        plugin.read_result(&mut store, packed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// Tiny WAT plugin: bump allocator + `plugin_invoke` returns "cmd:args".
    const ECHO_WAT: &str = r#"
        (module
          (memory (export "memory") 1)
          (global $bump (mut i32) (i32.const 1024))

          (func (export "plugin_alloc") (param $len i32) (result i32)
            (local $p i32)
            (local.set $p (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $len)))
            (local.get $p))

          (func (export "plugin_free") (param i32) (param i32))

          (func (export "plugin_invoke")
                (param $cp i32) (param $cl i32) (param $ap i32) (param $al i32)
                (result i64)
            (local $out i32)
            (local $tot i32)
            (local.set $tot
              (i32.add (i32.add (local.get $cl) (i32.const 1)) (local.get $al)))
            (local.set $out (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $tot)))
            (memory.copy (local.get $out) (local.get $cp) (local.get $cl))
            (i32.store8
              (i32.add (local.get $out) (local.get $cl))
              (i32.const 58))
            (memory.copy
              (i32.add (i32.add (local.get $out) (local.get $cl)) (i32.const 1))
              (local.get $ap)
              (local.get $al))
            (i64.or
              (i64.shl (i64.extend_i32_u (local.get $out)) (i64.const 32))
              (i64.extend_i32_u (local.get $tot)))))
    "#;

    #[tokio::test]
    async fn echo_plugin_roundtrip() {
        let wasm = wat::parse_str(ECHO_WAT).expect("wat compile");
        let host = WasmHost::new();
        let dummy_path = std::env::temp_dir().join(format!("aerotab-wasm-{}.wasm", Uuid::new_v4()));
        host.load_bytes("echo", &wasm, &dummy_path).await.unwrap();

        let r = host.invoke("echo", "greet", "world").await.unwrap();
        assert_eq!(r, "greet:world");

        // Second call exercises the per-plugin Mutex re-entry path.
        let r = host.invoke("echo", "x", "y").await.unwrap();
        assert_eq!(r, "x:y");

        let list = host.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "echo");
    }

    #[tokio::test]
    async fn load_missing_dir_creates_it() {
        let dir = std::env::temp_dir().join(format!("aerotab-wasm-dir-{}", Uuid::new_v4()));
        let host = WasmHost::new();
        let n = host.load_dir(&dir).await.unwrap();
        assert_eq!(n, 0);
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn invoke_unknown_plugin_errors() {
        let host = WasmHost::new();
        let err = host.invoke("nope", "x", "y").await.unwrap_err();
        assert!(matches!(err, WasmError::NotFound(_)));
    }
}
