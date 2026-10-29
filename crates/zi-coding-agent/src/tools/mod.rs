//! Default workspace tools for the coding agent CLI.

mod bash;
mod edit;
mod find;
mod grep;
mod ls;
mod path;
mod read;
mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zi_agent::AgentTool;

pub use bash::BashTool;
pub use edit::EditTool;
pub use find::find_tool;
pub use grep::grep_tool;
pub use ls::ls_tool;
pub use read::ReadTool;
pub use write::WriteTool;

/// Resolve `path` relative to `cwd` (absolute paths unchanged).
pub fn resolve_in_workspace(cwd: &Path, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        cwd.join(candidate)
    }
}

/// grep, find, and ls (Phase 4 wideners).
pub fn search_tools(cwd: PathBuf) -> Vec<Arc<dyn AgentTool>> {
    vec![
        grep_tool(cwd.clone()) as Arc<dyn AgentTool>,
        find_tool(cwd.clone()) as Arc<dyn AgentTool>,
        ls_tool(cwd) as Arc<dyn AgentTool>,
    ]
}

/// Built-in tools bound to a workspace directory.
pub fn default_tools(cwd: PathBuf) -> Vec<Arc<dyn AgentTool>> {
    let mut tools: Vec<Arc<dyn AgentTool>> = vec![
        Arc::new(ReadTool::new(cwd.clone())),
        Arc::new(BashTool::new(cwd.clone())),
        Arc::new(EditTool::new(cwd.clone())),
        Arc::new(WriteTool::new(cwd.clone())),
    ];
    tools.extend(search_tools(cwd));
    tools
}

// Phase 3 merge: optional `extra_*` tool modules can be wired here with `#[cfg(feature = "extra-tools")]`.
