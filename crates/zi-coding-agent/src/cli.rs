//! CLI argument definitions for the `zi` binary.

use std::path::PathBuf;

use clap::Parser;

/// zi coding agent — non-interactive print, JSON, and RPC modes.
#[derive(Debug, Parser)]
#[command(name = "zi", about = "zi coding agent CLI", disable_version_flag = true)]
pub struct Cli {
    /// Prompt text (single-shot). Required unless `--mode rpc`.
    #[arg(short = 'p', long = "print")]
    pub prompt: Option<String>,

    /// Output / control mode.
    #[arg(long = "mode", default_value = "text")]
    pub mode: OutputMode,

    /// Model id from the bundled catalog or `~/.zi/agent/models.json`.
    #[arg(long = "model")]
    pub model: Option<String>,

    /// Do not persist session NDJSON under the agent sessions directory.
    #[arg(long = "no-session")]
    pub no_session: bool,

    /// Working directory for tools and session encoding.
    #[arg(long = "cwd")]
    pub cwd: Option<PathBuf>,

    /// Positional prompt fragments (joined with spaces) when `-p` is omitted.
    #[arg(trailing_var_arg = true)]
    pub rest: Vec<String>,
}

/// How the process speaks on stdout / the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum OutputMode {
    /// Print mode with `-p`, or interactive TUI when no prompt and stdin is a TTY.
    #[default]
    Text,
    Json,
    Rpc,
    /// Force interactive TUI (requires a TTY).
    Interactive,
}

impl Cli {
    /// Resolved prompt: `-p` / `--print`, or joined trailing args.
    pub fn prompt_text(&self) -> Option<String> {
        if let Some(p) = &self.prompt {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        if self.rest.is_empty() {
            return None;
        }
        let joined = self.rest.join(" ").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }
}
