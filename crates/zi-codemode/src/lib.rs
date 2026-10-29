//! Codemode sandbox for zi (Phase 4).
//!
//! Scripts call registered Rust tools through a host bridge. With feature `quickjs`, scripts
//! run in QuickJS; otherwise a line-oriented DSL is used:
//!
//! ```text
//! await tool("grep", {"pattern": "fn main", "glob": "*.rs"})
//! ```

mod dsl;
mod sandbox;

#[cfg(feature = "quickjs")]
mod quickjs;

pub use sandbox::{
    default_sandbox, tool_map_invoker, CodemodeError, CodemodeSandbox, DslCodemodeSandbox,
    ToolInvoker, ToolInvokerEntry,
};

/// Returns whether QuickJS support was compiled in.
pub fn quickjs_enabled() -> bool {
    cfg!(feature = "quickjs")
}
