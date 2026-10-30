//! Default workspace tools for the coding agent CLI.

mod bash;
mod bash_guard;
mod edit;
mod explore;
mod find;
mod grep;
mod ls;
mod path;
mod read;
mod spill;
mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use zi_agent::AgentTool;
use zi_ai::{Model, SimpleStreamOptions};

pub use bash::BashTool;
pub use edit::EditTool;
pub use explore::ExploreTool;
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

/// Built-in tools bound to a workspace directory (no explore; needs model).
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

/// Parent-session tools + deferred explore (mounted on first `explore` call).
pub fn default_tools_with_explore(
    cwd: PathBuf,
    model: Model,
    stream_options: SimpleStreamOptions,
) -> (Vec<Arc<dyn AgentTool>>, Vec<Arc<dyn AgentTool>>) {
    let tools = default_tools(cwd.clone());
    let deferred = vec![Arc::new(ExploreTool::new(cwd, model, stream_options)) as Arc<dyn AgentTool>];
    (tools, deferred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zi_ai::ModelCost;

    fn test_model() -> Model {
        Model {
            id: "test".into(),
            name: "Test".into(),
            api: "openai-completions".into(),
            provider: "test".into(),
            base_url: String::new(),
            reasoning: false,
            context_window: 128_000,
            max_tokens: 4096,
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        }
    }

    #[test]
    fn parent_tools_defer_explore() {
        let (tools, deferred) = default_tools_with_explore(
            PathBuf::from("/tmp"),
            test_model(),
            SimpleStreamOptions::default(),
        );
        let names: Vec<_> = tools.iter().map(|t| t.name().to_string()).collect();
        assert_eq!(
            names,
            vec!["read", "bash", "edit", "write", "grep", "find", "ls"]
        );
        assert_eq!(
            deferred.iter().map(|t| t.name()).collect::<Vec<_>>(),
            vec!["explore"]
        );
    }

    #[test]
    fn default_tools_without_explore_keep_search() {
        let names: Vec<_> = default_tools(PathBuf::from("/tmp"))
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        assert!(names.contains(&"grep".to_string()));
        assert!(names.contains(&"find".to_string()));
        assert!(names.contains(&"ls".to_string()));
        assert!(!names.contains(&"explore".to_string()));
    }
}

// Phase 3 merge: optional `extra_*` tool modules can be wired here with `#[cfg(feature = "extra-tools")]`.
