//! wasmtime-backed WASM extension loader (`wasm-extensions` feature) — ABI v1.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde_json::{json, Value};
use wasmtime::*;
use zi_agent::{AgentTool, AgentToolResult, FnAgentTool};

use super::{
    Extension, ExtensionHook, ExtensionLoadContext, ExtensionUiEffects, SharedToolList,
    HOST_API_VERSION,
};

/// Load and initialize a WASM extension module.
pub fn load_wasm_extension(
    path: &Path,
    bytes: &[u8],
    _tool_list: SharedToolList,
    ui_effects: Arc<RwLock<ExtensionUiEffects>>,
    preferred_id: Option<&str>,
) -> Result<Box<dyn Extension>, String> {
    let engine = Engine::default();
    let module = Module::from_binary(&engine, bytes).map_err(|e| e.to_string())?;

    let id = preferred_id
        .map(|s| format!("wasm:{s}"))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| format!("wasm:{s}"))
                .unwrap_or_else(|| "wasm:loaded".into())
        });

    let mut store = Store::new(
        &engine,
        HostState {
            path: path.to_path_buf(),
            pending_tools: Vec::new(),
            pending_hooks: Vec::new(),
            pending_commands: Vec::new(),
            memory: None,
            ui_effects: Arc::clone(&ui_effects),
        },
    );

    let mut linker = Linker::new(&engine);
    linker
        .func_wrap(
            "zi_host",
            "register_tool",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), anyhow::Error> {
                let json = read_guest_json(&mut caller, ptr, len)?;
                caller.data_mut().pending_tools.push(json);
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            "zi_host",
            "register_command",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), anyhow::Error> {
                let json = read_guest_json(&mut caller, ptr, len)?;
                let name = json
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("register_command: missing name"))?
                    .to_string();
                let summary = json
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                caller
                    .data_mut()
                    .pending_commands
                    .push((name, summary));
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            "zi_host",
            "subscribe_hook",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), anyhow::Error> {
                let json = read_guest_json(&mut caller, ptr, len)?;
                let event = json
                    .get("event")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("subscribe_hook: missing event"))?;
                let hook = ExtensionHook::parse(event)
                    .ok_or_else(|| anyhow::anyhow!(format!("subscribe_hook: unknown event {event}")))?;
                caller.data_mut().pending_hooks.push(hook);
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            "zi_host",
            "emit_log",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), anyhow::Error> {
                let json = read_guest_json(&mut caller, ptr, len)?;
                let level = json
                    .get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("info");
                let message = json
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match level {
                    "warn" => tracing::warn!(target: "zi_wasm", "{message}"),
                    "error" => tracing::error!(target: "zi_wasm", "{message}"),
                    _ => tracing::info!(target: "zi_wasm", "{message}"),
                }
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            "zi_host",
            "ui_notify",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), anyhow::Error> {
                let json = read_guest_json(&mut caller, ptr, len)?;
                let message = json
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Ok(mut guard) = caller.data().ui_effects.write() {
                    guard.notices.push(message);
                }
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap(
            "zi_host",
            "ui_set_widget",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<(), anyhow::Error> {
                let json = read_guest_json(&mut caller, ptr, len)?;
                let text = match json.get("text") {
                    Some(Value::Null) | None => None,
                    Some(v) => Some(v.as_str().unwrap_or("").to_string()),
                };
                if let Ok(mut guard) = caller.data().ui_effects.write() {
                    guard.widget = Some(text);
                }
                Ok(())
            },
        )
        .map_err(|e| e.to_string())?;

    linker
        .func_wrap("zi_host", "host_api_version", || -> i32 {
            HOST_API_VERSION as i32
        })
        .map_err(|e| e.to_string())?;

    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| e.to_string())?;

    if let Some(memory) = instance.get_memory(&mut store, "memory") {
        store.data_mut().memory = Some(memory);
    }

    if let Ok(init) = instance.get_typed_func::<(), i32>(&mut store, "zi_extension_init") {
        let code = init.call(&mut store, ()).map_err(|e| e.to_string())?;
        if code != 0 {
            return Err(format!("zi_extension_init returned {code}"));
        }
    }

    let pending_tools = std::mem::take(&mut store.data_mut().pending_tools);
    let pending_hooks = std::mem::take(&mut store.data_mut().pending_hooks);
    let pending_commands = std::mem::take(&mut store.data_mut().pending_commands);
    let path_buf = path.to_path_buf();

    let runtime = Arc::new(Mutex::new(WasmGuestRuntime {
        store,
        instance,
        path: path_buf.clone(),
    }));

    Ok(Box::new(LoadedWasmExtension {
        id,
        path: path_buf,
        pending_tools,
        pending_hooks,
        pending_commands,
        runtime,
    }))
}

fn read_guest_json(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Result<Value, anyhow::Error> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| anyhow::anyhow!("missing memory export"))?;
    let data = memory
        .data(&*caller)
        .get(ptr as usize..(ptr as usize + len as usize))
        .ok_or_else(|| anyhow::anyhow!("bad ptr/len"))?;
    serde_json::from_slice(data).map_err(|e| anyhow::anyhow!(e.to_string()))
}

struct HostState {
    path: PathBuf,
    pending_tools: Vec<Value>,
    pending_hooks: Vec<ExtensionHook>,
    pending_commands: Vec<(String, String)>,
    memory: Option<Memory>,
    ui_effects: Arc<RwLock<ExtensionUiEffects>>,
}

struct WasmGuestRuntime {
    store: Store<HostState>,
    instance: Instance,
    path: PathBuf,
}

impl WasmGuestRuntime {
    fn call_tool(&mut self, name: &str, args: &Value) -> Result<String, String> {
        let payload = json!({
            "type": "tool_call",
            "name": name,
            "arguments": args,
        });
        self.call_json_export("zi_call_tool", &payload)
    }

    fn call_command(&mut self, name: &str, args: &str) -> Result<String, String> {
        let payload = json!({
            "type": "command",
            "name": name,
            "args": args,
        });
        match self.call_json_export("zi_call_command", &payload) {
            Ok(text) => Ok(text),
            Err(err) if err.contains("missing export") => Ok(String::new()),
            Err(err) => Err(err),
        }
    }

    fn call_hook(&mut self, hook: ExtensionHook, payload: &Value) -> Result<(), String> {
        let mut body = payload.clone();
        if let Some(obj) = body.as_object_mut() {
            obj.insert("event".into(), json!(hook.as_str()));
        }
        match self.call_json_export("zi_on_hook", &body) {
            Ok(_) => Ok(()),
            Err(err) if err.contains("missing export") => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn call_json_export(&mut self, export: &str, payload: &Value) -> Result<String, String> {
        let bytes = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| "missing memory export".to_string())?;

        let alloc = match self
            .instance
            .get_typed_func::<i32, i32>(&mut self.store, "zi_alloc")
        {
            Ok(f) => f,
            Err(_) => {
                return Err(format!(
                    "WASM module {} missing zi_alloc; cannot pass payload to {export}",
                    self.path.display()
                ));
            }
        };
        let ptr = alloc
            .call(&mut self.store, bytes.len() as i32)
            .map_err(|e| e.to_string())?;
        memory
            .write(&mut self.store, ptr as usize, &bytes)
            .map_err(|e| e.to_string())?;

        let func = self
            .instance
            .get_typed_func::<(i32, i32), i32>(&mut self.store, export)
            .map_err(|_| format!("missing export {export}"))?;
        let result_ptr = func
            .call(&mut self.store, (ptr, bytes.len() as i32))
            .map_err(|e| e.to_string())?;

        // Convention: result is a length-prefixed UTF-8 string at result_ptr (u32 LE len + bytes),
        // or 0 for empty/ok with no body.
        if result_ptr == 0 {
            return Ok(String::new());
        }
        let data = memory.data(&self.store);
        let start = result_ptr as usize;
        if start + 4 > data.len() {
            return Err("tool result pointer out of bounds".into());
        }
        let len = u32::from_le_bytes(data[start..start + 4].try_into().unwrap()) as usize;
        let body_start = start + 4;
        if body_start + len > data.len() {
            return Err("tool result body out of bounds".into());
        }
        String::from_utf8(data[body_start..body_start + len].to_vec()).map_err(|e| e.to_string())
    }
}

struct LoadedWasmExtension {
    id: String,
    path: PathBuf,
    pending_tools: Vec<Value>,
    pending_hooks: Vec<ExtensionHook>,
    pending_commands: Vec<(String, String)>,
    runtime: Arc<Mutex<WasmGuestRuntime>>,
}

impl Extension for LoadedWasmExtension {
    fn id(&self) -> &str {
        &self.id
    }

    fn source_path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn load(&self, ctx: &mut ExtensionLoadContext<'_>) {
        for hook in &self.pending_hooks {
            ctx.subscribe_hook(*hook);
        }
        for (name, summary) in &self.pending_commands {
            ctx.register_command(name.clone(), summary.clone());
        }
        for spec in &self.pending_tools {
            let Some(tool) = tool_from_json_spec(spec, Arc::clone(&self.runtime)) else {
                tracing::warn!(?spec, "WASM register_tool: invalid payload");
                continue;
            };
            ctx.register_tool(tool);
        }
    }

    fn on_hook(&self, hook: ExtensionHook, payload: &Value) {
        if let Ok(mut rt) = self.runtime.lock() {
            if let Err(err) = rt.call_hook(hook, payload) {
                tracing::warn!(
                    path = %self.path.display(),
                    hook = hook.as_str(),
                    "WASM hook failed: {err}"
                );
            }
        }
    }

    fn on_command(&self, name: &str, args: &str) -> Option<String> {
        let mut rt = self.runtime.lock().ok()?;
        match rt.call_command(name, args) {
            Ok(text) => Some(text),
            Err(err) => {
                tracing::warn!(
                    path = %self.path.display(),
                    command = name,
                    "WASM command failed: {err}"
                );
                Some(format!("wasm command error: {err}"))
            }
        }
    }
}

fn tool_from_json_spec(
    spec: &Value,
    runtime: Arc<Mutex<WasmGuestRuntime>>,
) -> Option<Arc<dyn AgentTool>> {
    let name = spec.get("name")?.as_str()?;
    let description = spec
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or(name);
    let parameters = spec
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| json!({"type":"object","properties":{}}));
    let tool_name = name.to_string();
    Some(FnAgentTool::new(
        tool_name.clone(),
        description,
        parameters,
        move |_id, args, _on_update| {
            let tool_name = tool_name.clone();
            let runtime = Arc::clone(&runtime);
            async move {
                let result = tokio::task::spawn_blocking(move || {
                    let mut rt = runtime.lock().map_err(|e| e.to_string())?;
                    rt.call_tool(&tool_name, &args)
                })
                .await;
                match result {
                    Ok(Ok(text)) if !text.is_empty() => AgentToolResult::text(text),
                    Ok(Ok(_)) => AgentToolResult::text("ok"),
                    Ok(Err(err)) => AgentToolResult::error(err),
                    Err(err) => AgentToolResult::error(format!("WASM tool join error: {err}")),
                }
            }
        },
    ))
}
