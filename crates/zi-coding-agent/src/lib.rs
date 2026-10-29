//! zi coding agent library (CLI modes and session wiring).

pub mod cli;
pub mod context;
pub mod dev_ext;
pub mod env_file;
pub mod extensions;
pub mod keybindings;
pub mod mcp_bridge;
pub mod mcp_config;
pub mod model_registry;
pub mod modes;
pub mod packages;
pub mod prompts;
pub mod session_store;
pub mod settings;
pub mod skills;
pub mod slash;
pub mod system_prompt;
pub mod themes;
pub mod tools;
pub mod trust;
pub mod web;

pub use modes::interactive::{
    boxed_prompt, run as run_interactive_mode, run_with as run_interactive_with, PromptCallback,
};
pub use mcp_bridge::{list_mcp_tool_descriptors, mount_mcp_tools, McpMountOptions, McpToolMount};
pub use zi_tui::{
    InteractiveExit, InteractiveOptions, InteractivePromptError, InteractiveSubmitResult,
    SlashSuggestion, TuiError,
};
pub use extensions::{
    Extension, ExtensionHost, ExtensionHook, ExtensionRegistry, ExtensionUiEffects,
    HOST_API_VERSION,
};
pub use prompts::{
    expand_prompt_template, load_default_prompts, load_prompt_templates, parse_command_args,
    substitute_args, LoadPromptsOptions, LoadPromptsResult, PromptSource, PromptTemplate,
};
pub use skills::{
    format_skills_for_prompt, load_default_skills, load_skills, LoadSkillsOptions, LoadSkillsResult,
    Skill, SkillSource,
};
pub use system_prompt::coding_agent_system_prompt;
pub use tools::{default_tools, search_tools};
pub use trust::{is_project_trusted, set_project_trust};
