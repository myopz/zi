//! Built-in extensions shipped with the zi CLI (Rust modules, not WASM).

mod codemode;
mod mcp;
mod tool_search;

use std::sync::Arc;

use super::{Extension, SharedToolList};

/// Which built-in extension to load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinExtensionKind {
    Mcp,
    Codemode,
    ToolSearch,
}

/// Construct all default built-in extensions.
pub fn load_builtin_extensions(tool_list: SharedToolList) -> Vec<Box<dyn Extension>> {
    [
        BuiltinExtensionKind::Mcp,
        BuiltinExtensionKind::Codemode,
        BuiltinExtensionKind::ToolSearch,
    ]
    .into_iter()
    .map(|kind| match kind {
        BuiltinExtensionKind::Mcp => Box::new(mcp::McpBuiltin::new()) as Box<dyn Extension>,
        BuiltinExtensionKind::Codemode => {
            Box::new(codemode::CodemodeBuiltin::new(Arc::clone(&tool_list)))
        }
        BuiltinExtensionKind::ToolSearch => {
            Box::new(tool_search::ToolSearchBuiltin::new(Arc::clone(&tool_list)))
        }
    })
    .collect()
}
