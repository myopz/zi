//! Differential terminal UI for zi (Phase 5).
//!
//! Semantic alignment with pi `packages/tui`: components render to lines;
//! the main screen diffs against the previous frame.

mod app;
mod clipboard;
mod component;
mod components;
mod error;
mod interactive;
mod main_screen;
mod width;
mod wrap;

pub use app::TuiApp;
pub use clipboard::{read_text as clipboard_read, write_text as clipboard_write};
pub use component::{Component, InputOutcome};
pub use components::{Editor, Markdown, Message, MessageList, MessageRole, SlashSuggestion, Text};
pub use error::TuiError;
pub use interactive::{
    run_interactive, run_interactive_boxed, InteractiveExit, InteractiveOptions,
    InteractivePromptError, InteractiveSubmitResult, KeybindingOverrides, LiveUpdate, PromptFuture,
    StatusInfo,
};
pub use main_screen::MainScreen;
pub use width::{truncate_to_width, visible_width};
pub use wrap::wrap_text;
