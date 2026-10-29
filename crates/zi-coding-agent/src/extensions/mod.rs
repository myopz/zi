//! WASM extension host and in-process built-in extensions.
//!
//! # WASM guest ABI (JSON over host imports)
//!
//! Guest modules are `.wasm` files discovered under `~/.zi/agent/extensions` and
//! `<cwd>/.zi/extensions`. With the `wasm-extensions` feature, `wasmtime` loads the module;
//! without it, [`WasmExtension`] reads bytes and records metadata only (no execution).
//!
//! ## Exports (guest → host)
//!
//! - `zi_extension_info() -> i32` — returns a pointer/length pair encoded as a single i32 is **not**
//!   used; instead the guest calls host imports during `zi_extension_init`.
//! - `zi_extension_init() -> i32` — must return `0` on success. During init the guest may call host
//!   imports below.
//!
//! ## Imports (host → guest, module `zi_host`) — ABI v1
//!
//! All payloads are UTF-8 JSON passed as `(ptr: i32, len: i32)` in the guest's linear memory
//! unless noted.
//!
//! | Import | JSON shape |
//! |--------|------------|
//! | `register_tool` | `{ "name", "description", "parameters" }` |
//! | `register_command` | `{ "name", "summary" }` |
//! | `subscribe_hook` | `{ "event": session_start\|session_switch\|before_agent_start\|agent_start\|agent_end\|tool_call\|tool_result }` |
//! | `emit_log` | `{ "level", "message" }` |
//! | `ui_notify` | `{ "message" }` |
//! | `ui_set_widget` | `{ "text": string\|null }` |
//! | `ui_set_session_name` | `{ "name" }` |
//! | `host_api_version` | `() -> i32` (no JSON; returns host ABI, currently `1`) |
//!
//! Hook delivery uses export `zi_on_hook(ptr, len)` when present; otherwise hooks are ignored.
//! Slash commands registered via `register_command` are invoked through `zi_call_command` when present.
//!
//! Built-in Rust extensions implement [`Extension`] / [`crate::dev_ext::HostExtensionPack`]
//! and do not use this wire format.

mod builtin;

#[cfg(feature = "wasm-extensions")]
mod wasm_runtime;

#[cfg(all(test, feature = "wasm-extensions"))]
mod wasm_hello_test;

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use zi_agent::AgentTool;

pub use builtin::load_builtin_extensions;
pub use zi_host_extensions::ExtensionHook;

/// Host ABI version exposed to WASM guests (`host_api_version` / manifest negotiation).
pub const HOST_API_VERSION: u32 = 1;

/// Registration surface while an extension loads.
pub struct ExtensionLoadContext<'a> {
    registry: &'a mut ExtensionRegistry,
    extension_id: String,
}

impl ExtensionLoadContext<'_> {
    /// Register an LLM-callable tool owned by the loading extension.
    pub fn register_tool(&mut self, tool: Arc<dyn AgentTool>) {
        self.registry.tools.push(tool);
    }

    /// Register a slash command owned by the loading extension.
    #[cfg_attr(not(feature = "wasm-extensions"), allow(dead_code))]
    pub fn register_command(&mut self, name: impl Into<String>, summary: impl Into<String>) {
        self.registry.commands.push(RegisteredCommand {
            extension_id: self.extension_id.clone(),
            name: name.into(),
            summary: summary.into(),
        });
    }

    /// Record interest in a lifecycle hook.
    pub fn subscribe_hook(&mut self, hook: ExtensionHook) {
        self.registry.hooks.push(HookSubscription {
            extension_id: self.extension_id.clone(),
            hook,
        });
    }
}

/// In-process or WASM-backed extension.
pub trait Extension: Send + Sync {
    /// Stable id (`builtin:mcp`, `wasm:my-plugin`, …).
    fn id(&self) -> &str;

    /// Register tools and hooks.
    fn load(&self, ctx: &mut ExtensionLoadContext<'_>);

    /// Optional path on disk (WASM files only).
    fn source_path(&self) -> Option<&Path> {
        None
    }

    /// Deliver a lifecycle hook the extension subscribed to.
    fn on_hook(&self, _hook: ExtensionHook, _payload: &serde_json::Value) {}

    /// Handle a slash command registered by this extension. Returns notice text when handled.
    fn on_command(&self, _name: &str, _args: &str) -> Option<String> {
        None
    }
}

/// One extension's interest in a hook.
#[derive(Debug, Clone)]
pub struct HookSubscription {
    pub extension_id: String,
    pub hook: ExtensionHook,
}

/// Slash command registered by a WASM (or other) extension.
#[derive(Debug, Clone)]
pub struct RegisteredCommand {
    pub extension_id: String,
    pub name: String,
    pub summary: String,
}

/// UI side effects produced while dispatching WASM hooks / commands.
#[derive(Debug, Default, Clone)]
pub struct ExtensionUiEffects {
    pub notices: Vec<String>,
    pub widget: Option<Option<String>>,
    pub session_name: Option<String>,
}

/// Tools and hooks collected from extensions.
#[derive(Default)]
pub struct ExtensionRegistry {
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub hooks: Vec<HookSubscription>,
    pub commands: Vec<RegisteredCommand>,
    pub errors: Vec<ExtensionError>,
    pub warnings: Vec<String>,
}

/// Failed extension load.
#[derive(Debug, Clone)]
pub struct ExtensionError {
    pub path: PathBuf,
    pub message: String,
}

/// Shared tool list for builtins (e.g. codemode invoker, tool_search listing).
pub type SharedToolList = Arc<RwLock<Vec<Arc<dyn AgentTool>>>>;

/// Discovers and loads extensions for a workspace.
pub struct ExtensionHost {
    extensions: Vec<Box<dyn Extension>>,
    registry: ExtensionRegistry,
    tool_list: SharedToolList,
    ui_effects: Arc<RwLock<ExtensionUiEffects>>,
}

impl ExtensionHost {
    /// Create an empty host bound to `cwd` (used for discovery paths only).
    pub fn new(_cwd: &Path) -> Self {
        Self {
            extensions: Vec::new(),
            registry: ExtensionRegistry::default(),
            tool_list: Arc::new(RwLock::new(Vec::new())),
            ui_effects: Arc::new(RwLock::new(ExtensionUiEffects::default())),
        }
    }

    /// Load built-in Rust extensions (mcp, codemode, tool_search).
    pub fn load_builtins(&mut self, base_tools: &[Arc<dyn AgentTool>]) {
        {
            let mut guard = self.tool_list.write().expect("tool list lock");
            *guard = base_tools.to_vec();
        }
        for ext in load_builtin_extensions(Arc::clone(&self.tool_list)) {
            self.load_extension(ext);
        }
    }

    /// Scan `~/.zi/agent/extensions` and `<cwd>/.zi/extensions` for `.wasm` files.
    pub fn load_wasm_extensions(&mut self, cwd: &Path, agent_dir: &Path, include_project: bool) {
        let discovered = crate::dev_ext::discover_extensions(cwd, agent_dir, include_project);
        let mut loaded_paths = std::collections::HashSet::new();

        for ext in &discovered {
            if ext.kind != crate::dev_ext::ExtensionKind::Wasm || !ext.enabled {
                continue;
            }
            if ext.api_version > HOST_API_VERSION {
                self.registry.errors.push(ExtensionError {
                    path: ext.manifest_path.clone(),
                    message: format!(
                        "extension apiVersion {} > host {} ({})",
                        ext.api_version, HOST_API_VERSION, ext.id
                    ),
                });
                continue;
            }
            let wasm_path = ext.root.join("extension.wasm");
            if wasm_path.is_file() {
                self.load_wasm_file(&wasm_path, Some(ext.id.as_str()));
                loaded_paths.insert(wasm_path);
            } else {
                self.registry.warnings.push(format!(
                    "wasm extension `{}` has no extension.wasm at {}",
                    ext.id,
                    wasm_path.display()
                ));
            }
        }

        let mut dirs = vec![agent_dir.join("extensions")];
        if include_project {
            dirs.push(cwd.join(crate::skills::ZI_CONFIG_DIR).join("extensions"));
        }
        for path in crate::packages::extension_paths_for(cwd, agent_dir, include_project) {
            if path.extension().and_then(|e| e.to_str()) == Some("wasm") && path.is_file() {
                if loaded_paths.insert(path.clone()) {
                    self.load_wasm_file(&path, None);
                }
            } else if path.is_dir() {
                discover_wasm_in_dir(&path, &mut |p| {
                    if loaded_paths.insert(p.clone()) {
                        self.load_wasm_file(&p, None);
                    }
                });
            }
        }
        for dir in dirs {
            discover_wasm_in_dir(&dir, &mut |path| {
                if loaded_paths.insert(path.clone()) {
                    self.load_wasm_file(&path, None);
                }
            });
        }
    }

    /// Dispatch `hook` to every extension that subscribed; return UI effects from guests.
    pub fn dispatch_hook(
        &self,
        hook: ExtensionHook,
        payload: &serde_json::Value,
    ) -> ExtensionUiEffects {
        {
            let mut guard = self.ui_effects.write().expect("ui effects lock");
            *guard = ExtensionUiEffects::default();
        }
        let interested: Vec<_> = self
            .registry
            .hooks
            .iter()
            .filter(|s| s.hook == hook)
            .map(|s| s.extension_id.clone())
            .collect();
        if !interested.is_empty() {
            for ext in &self.extensions {
                if interested.iter().any(|id| id == ext.id()) {
                    tracing::debug!(
                        extension = ext.id(),
                        hook = hook.as_str(),
                        "dispatching extension hook"
                    );
                    ext.on_hook(hook, payload);
                }
            }
        }
        self.take_ui_effects()
    }

    /// Run a WASM-registered slash command; returns notice text when handled.
    pub fn dispatch_command(&self, name: &str, args: &str) -> Option<String> {
        {
            let mut guard = self.ui_effects.write().expect("ui effects lock");
            *guard = ExtensionUiEffects::default();
        }
        let lower = name.to_ascii_lowercase();
        let Some(cmd) = self
            .registry
            .commands
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(&lower))
        else {
            return None;
        };
        let extension_id = cmd.extension_id.clone();
        for ext in &self.extensions {
            if ext.id() == extension_id {
                if let Some(text) = ext.on_command(name, args) {
                    let mut effects = self.take_ui_effects();
                    if !text.is_empty() {
                        effects.notices.insert(0, text);
                    }
                    return Some(effects.notices.join("\n"));
                }
            }
        }
        let effects = self.take_ui_effects();
        if effects.notices.is_empty() {
            None
        } else {
            Some(effects.notices.join("\n"))
        }
    }

    fn take_ui_effects(&self) -> ExtensionUiEffects {
        let mut guard = self.ui_effects.write().expect("ui effects lock");
        std::mem::take(&mut *guard)
    }

    fn load_wasm_file(&mut self, path: &Path, preferred_id: Option<&str>) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(err) => {
                self.registry.errors.push(ExtensionError {
                    path: path.to_path_buf(),
                    message: err.to_string(),
                });
                return;
            }
        };
        if bytes.len() < 4 || &bytes[0..4] != b"\0asm" {
            self.registry.errors.push(ExtensionError {
                path: path.to_path_buf(),
                message: "not a WASM module (missing \\0asm magic)".into(),
            });
            return;
        }

        #[cfg(feature = "wasm-extensions")]
        {
            match wasm_runtime::load_wasm_extension(
                path,
                &bytes,
                Arc::clone(&self.tool_list),
                Arc::clone(&self.ui_effects),
                preferred_id,
            ) {
                Ok(ext) => self.load_extension(ext),
                Err(msg) => self.registry.errors.push(ExtensionError {
                    path: path.to_path_buf(),
                    message: msg,
                }),
            }
        }

        #[cfg(not(feature = "wasm-extensions"))]
        {
            let id = preferred_id
                .map(|s| format!("wasm:{s}"))
                .unwrap_or_else(|| "wasm:stub".into());
            let ext = WasmExtension::new(path.to_path_buf(), bytes, id);
            self.registry.warnings.push(format!(
                "loaded WASM metadata for {} (enable feature `wasm-extensions` to execute)",
                path.display()
            ));
            self.load_extension(Box::new(ext));
        }
    }

    fn load_extension(&mut self, extension: Box<dyn Extension>) {
        tracing::debug!(
            id = extension.id(),
            source = ?extension.source_path().map(|p| p.display().to_string()),
            "loading extension"
        );
        let extension_id = extension.id().to_string();
        let mut ctx = ExtensionLoadContext {
            registry: &mut self.registry,
            extension_id,
        };
        extension.load(&mut ctx);
        self.extensions.push(extension);
    }

    /// Final merged tool list: `base` plus extension-registered tools (deduped by name).
    pub fn merge_tools(&self, base: Vec<Arc<dyn AgentTool>>) -> Vec<Arc<dyn AgentTool>> {
        let mut out = base;
        let mut seen: std::collections::HashSet<String> =
            out.iter().map(|t| t.name().to_string()).collect();
        for tool in &self.registry.tools {
            if seen.insert(tool.name().to_string()) {
                out.push(Arc::clone(tool));
            }
        }
        if let Ok(mut guard) = self.tool_list.write() {
            *guard = out.clone();
        }
        out
    }

    pub fn registry(&self) -> &ExtensionRegistry {
        &self.registry
    }

    pub fn extensions(&self) -> &[Box<dyn Extension>] {
        &self.extensions
    }
}

/// Stub WASM extension when built without `wasm-extensions` (metadata only).
#[cfg(not(feature = "wasm-extensions"))]
pub struct WasmExtension {
    path: PathBuf,
    bytes: Vec<u8>,
    id: String,
}

#[cfg(not(feature = "wasm-extensions"))]
impl WasmExtension {
    pub fn new(path: PathBuf, bytes: Vec<u8>, id: String) -> Self {
        Self { path, bytes, id }
    }
}

#[cfg(not(feature = "wasm-extensions"))]
impl Extension for WasmExtension {
    fn id(&self) -> &str {
        &self.id
    }

    fn source_path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn load(&self, ctx: &mut ExtensionLoadContext<'_>) {
        tracing::debug!(
            path = %self.path.display(),
            bytes = self.bytes.len(),
            "WASM extension registered (stub; enable `wasm-extensions` for execution)"
        );
        ctx.subscribe_hook(ExtensionHook::SessionStart);
    }
}

fn discover_wasm_in_dir(dir: &Path, on_file: &mut dyn FnMut(PathBuf)) {
    if !dir.is_dir() {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wasm") && path.is_file() {
            on_file(path);
            continue;
        }
        if path.is_dir() {
            discover_wasm_in_dir(&path, on_file);
        }
    }
}
