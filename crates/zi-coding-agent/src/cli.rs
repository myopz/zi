//! CLI argument definitions for the `zi` binary.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// zi coding agent — non-interactive print, JSON, RPC, and web modes.
#[derive(Debug, Parser)]
#[command(name = "zi", about = "zi coding agent CLI", disable_version_flag = true)]
pub struct Cli {
    /// Prompt text (single-shot). Required unless `--mode rpc` / `--mode web` or a subcommand.
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

    /// Web mode listen port (default 3080).
    #[arg(long = "port", default_value = "3080")]
    pub port: u16,

    /// Web mode bind host (default loopback).
    #[arg(long = "host", default_value = "127.0.0.1")]
    pub host: String,

    /// Web mode: do not open a browser tab on start.
    #[arg(long = "no-open")]
    pub no_open: bool,

    /// Admin / management subcommands (e.g. `zi web seed-user`).
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Positional prompt fragments (joined with spaces) when `-p` is omitted.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub rest: Vec<String>,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Web UI admin commands (seed users, list users).
    Web(WebCli),
}

/// `zi web …` admin CLI.
#[derive(Debug, Parser)]
pub struct WebCli {
    #[command(subcommand)]
    pub command: WebCommand,
}

/// Web admin actions.
#[derive(Debug, Subcommand)]
pub enum WebCommand {
    /// Create or rotate a long-lived login token for a username.
    SeedUser {
        /// Username (created if missing).
        username: String,
        /// Raw token to store (hashed). Generated when omitted.
        #[arg(long)]
        token: Option<String>,
    },
    /// List usernames in the web SQLite database.
    ListUsers,
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
    /// HTTP/WS multi-user web host (axum).
    Web,
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
