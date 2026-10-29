//! Bash tool — run shell commands in the workspace cwd.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use zi_agent::{AgentTool, AgentToolResult};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::Command;

const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// Execute shell commands with cwd set to the workspace.
pub struct BashTool {
    cwd: PathBuf,
}

impl BashTool {
    /// Bind command execution to `cwd`.
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }

    fn shell_program() -> String {
        if let Ok(shell) = std::env::var("SHELL") {
            if !shell.is_empty() {
                return shell;
            }
        }
        if cfg!(unix) {
            "/bin/sh".to_string()
        } else {
            "cmd.exe".to_string()
        }
    }
}

#[async_trait]
impl AgentTool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command in the workspace (stdout and stderr combined)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "timeout": { "type": "number", "description": "Timeout in seconds (optional)" }
            },
            "required": ["command"]
        })
    }

    async fn execute(
        &self,
        _tool_call_id: &str,
        args: Value,
        on_update: Option<zi_agent::AgentToolUpdateCallback>,
    ) -> AgentToolResult {
        let command = match args.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return AgentToolResult::error("missing required parameter: command"),
        };
        let timeout_secs = args.get("timeout").and_then(|v| v.as_f64());

        if !self.cwd.is_dir() {
            return AgentToolResult::error(format!(
                "working directory does not exist: {}",
                self.cwd.display()
            ));
        }

        let started = Instant::now();
        let shell = Self::shell_program();
        let mut cmd = if cfg!(windows) && shell.ends_with("cmd.exe") {
            let mut c = Command::new(&shell);
            c.args(["/C", command]);
            c
        } else {
            let mut c = Command::new(&shell);
            c.arg("-lc").arg(command);
            c
        };
        cmd.current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(err) => return AgentToolResult::error(format!("failed to spawn shell: {err}")),
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let read_stream = |mut reader: BufReader<tokio::process::ChildStdout>, mut combined: Vec<u8>| async move {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break combined,
                    Ok(n) => {
                        combined.extend_from_slice(&buf[..n]);
                        if combined.len() > MAX_OUTPUT_BYTES {
                            combined.truncate(MAX_OUTPUT_BYTES);
                            break combined;
                        }
                    }
                    Err(_) => break combined,
                }
            }
        };

        let read_err_stream =
            |mut reader: BufReader<tokio::process::ChildStderr>, mut combined: Vec<u8>| async move {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break combined,
                        Ok(n) => {
                            combined.extend_from_slice(&buf[..n]);
                            if combined.len() > MAX_OUTPUT_BYTES {
                                combined.truncate(MAX_OUTPUT_BYTES);
                                break combined;
                            }
                        }
                        Err(_) => break combined,
                    }
                }
            };

        let read_out = async move {
            let mut combined = Vec::new();
            let (out_task, err_task) = (
                stdout.map(|out| read_stream(BufReader::new(out), Vec::new())),
                stderr.map(|err| read_err_stream(BufReader::new(err), Vec::new())),
            );
            match (out_task, err_task) {
                (Some(out_fut), Some(err_fut)) => {
                    let (mut out_bytes, mut err_bytes) = tokio::join!(out_fut, err_fut);
                    combined.append(&mut out_bytes);
                    combined.append(&mut err_bytes);
                }
                (Some(out_fut), None) => combined = out_fut.await,
                (None, Some(err_fut)) => combined = err_fut.await,
                (None, None) => {}
            }
            String::from_utf8_lossy(&combined).into_owned()
        };

        let output = if let Some(secs) = timeout_secs {
            if secs <= 0.0 || !secs.is_finite() {
                return AgentToolResult::error("invalid timeout: must be a positive number of seconds");
            }
            let duration = tokio::time::Duration::from_secs_f64(secs);
            match tokio::time::timeout(duration, read_out).await {
                Ok(text) => text,
                Err(_) => {
                    let _ = child.kill().await;
                    return AgentToolResult::error(format!("command timed out after {secs}s"));
                }
            }
        } else {
            read_out.await
        };

        if let Some(cb) = &on_update {
            cb(AgentToolResult::text(output.clone()));
        }

        let status = match child.wait().await {
            Ok(s) => s,
            Err(err) => return AgentToolResult::error(format!("wait failed: {err}")),
        };
        let exit_code = status.code().unwrap_or(1);
        let wall = started.elapsed().as_secs_f64();

        let mut text = output;
        if text.len() > MAX_OUTPUT_BYTES {
            text.truncate(MAX_OUTPUT_BYTES);
            text.push_str("\n… (output truncated)");
        }
        text.push_str(&format!("\n\nexit_code: {exit_code}\nwall_time_seconds: {wall:.3}"));

        if exit_code == 0 {
            AgentToolResult::text(text)
        } else {
            AgentToolResult::error(text)
        }
    }
}
