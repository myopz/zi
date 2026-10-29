//! Load `mcp.json` and connect configured MCP servers.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Deserialize;
use zi_agent::AgentTool;
use zi_mcp::{
    McpClient, McpClientOptions, StdioTransport, StdioTransportOptions, StreamableHttpTransport,
    StreamableHttpTransportOptions, Url,
};

use crate::mcp_bridge::{mount_mcp_tools, McpMountOptions};
use crate::skills::ZI_CONFIG_DIR;

/// One configured MCP server (stdio or HTTP).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        description: Option<String>,
    },
}

/// Top-level mcp.json shape: `{ "mcpServers": { "name": { ... } } }` or flat map.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpJsonFile {
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: HashMap<String, McpServerConfig>,
    /// Allow flat `{ "name": { command: ... } }` without wrapper.
    #[serde(flatten)]
    pub flat: HashMap<String, serde_json::Value>,
}

/// Connected MCP server status for `/mcp`.
#[derive(Debug, Clone)]
pub struct McpServerStatus {
    pub name: String,
    pub kind: String,
    pub ok: bool,
    pub detail: String,
    pub tool_count: usize,
}

/// Runtime MCP state attached to the CLI.
#[derive(Default)]
pub struct McpRuntime {
    pub clients: Vec<(String, Arc<McpClient>)>,
    pub statuses: Vec<McpServerStatus>,
}

impl std::fmt::Debug for McpRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpRuntime")
            .field("clients", &self.clients.iter().map(|(n, _)| n).collect::<Vec<_>>())
            .field("statuses", &self.statuses)
            .finish()
    }
}

impl McpRuntime {
    pub fn summary(&self) -> String {
        if self.statuses.is_empty() {
            return "MCP: no servers configured (add ~/.zi/agent/mcp.json).".into();
        }
        let mut lines = vec!["MCP servers:".into()];
        for s in &self.statuses {
            let mark = if s.ok { "ok" } else { "err" };
            lines.push(format!(
                "  [{mark}] {} ({}) — {} ({} tools)",
                s.name, s.kind, s.detail, s.tool_count
            ));
        }
        lines.join("\n")
    }
}

/// Resolve mcp.json paths (user always; project when trusted).
pub fn mcp_config_paths(cwd: &Path, agent_dir: &Path, include_project: bool) -> Vec<PathBuf> {
    let mut paths = vec![agent_dir.join("mcp.json")];
    if include_project {
        paths.push(cwd.join(ZI_CONFIG_DIR).join("mcp.json"));
    }
    paths
}

/// Parse and merge mcp.json files (later files override same server name).
pub fn load_mcp_servers(
    cwd: &Path,
    agent_dir: &Path,
    include_project: bool,
) -> Result<HashMap<String, McpServerConfig>> {
    let mut servers = HashMap::new();
    for path in mcp_config_paths(cwd, agent_dir, include_project) {
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let parsed = parse_mcp_json(&text)
            .with_context(|| format!("parse {}", path.display()))?;
        servers.extend(parsed);
    }
    Ok(servers)
}

fn parse_mcp_json(text: &str) -> Result<HashMap<String, McpServerConfig>> {
    let file: McpJsonFile = serde_json::from_str(text)?;
    let mut servers = file.mcp_servers;
    // Flattened keys that look like server configs (skip known wrapper keys already taken).
    for (name, value) in file.flat {
        if name == "mcpServers" {
            continue;
        }
        if let Ok(cfg) = serde_json::from_value::<McpServerConfig>(value) {
            servers.insert(name, cfg);
        }
    }
    Ok(servers)
}

/// Connect all configured servers and mount their tools.
pub async fn connect_mcp_servers(
    cwd: &Path,
    agent_dir: &Path,
    include_project: bool,
) -> (McpRuntime, Vec<Arc<dyn AgentTool>>) {
    let mut runtime = McpRuntime::default();
    let mut tools: Vec<Arc<dyn AgentTool>> = Vec::new();

    let servers = match load_mcp_servers(cwd, agent_dir, include_project) {
        Ok(s) => s,
        Err(err) => {
            runtime.statuses.push(McpServerStatus {
                name: "(config)".into(),
                kind: "error".into(),
                ok: false,
                detail: format!("{err:#}"),
                tool_count: 0,
            });
            return (runtime, tools);
        }
    };

    if servers.is_empty() {
        return (runtime, tools);
    }

    for (name, config) in servers {
        match connect_one(&name, &config, cwd).await {
            Ok((client, mounted)) => {
                let count = mounted.len();
                tools.extend(mounted);
                runtime.statuses.push(McpServerStatus {
                    name: name.clone(),
                    kind: match &config {
                        McpServerConfig::Stdio { .. } => "stdio".into(),
                        McpServerConfig::Http { .. } => "http".into(),
                    },
                    ok: true,
                    detail: mcp_status_detail(&config, "connected"),
                    tool_count: count,
                });
                runtime.clients.push((name, client));
            }
            Err(err) => {
                runtime.statuses.push(McpServerStatus {
                    name,
                    kind: match &config {
                        McpServerConfig::Stdio { .. } => "stdio".into(),
                        McpServerConfig::Http { .. } => "http".into(),
                    },
                    ok: false,
                    detail: mcp_status_detail(&config, &format!("{err:#}")),
                    tool_count: 0,
                });
            }
        }
    }

    (runtime, tools)
}

fn mcp_status_detail(config: &McpServerConfig, fallback: &str) -> String {
    let description = match config {
        McpServerConfig::Stdio { description, .. }
        | McpServerConfig::Http { description, .. } => description.as_deref(),
    };
    match description.map(str::trim).filter(|s| !s.is_empty()) {
        Some(desc) => format!("{fallback} — {desc}"),
        None => fallback.to_string(),
    }
}

async fn connect_one(
    name: &str,
    config: &McpServerConfig,
    cwd: &Path,
) -> Result<(Arc<McpClient>, Vec<Arc<dyn AgentTool>>)> {
    let client = Arc::new(McpClient::new(McpClientOptions::new("zi", env!("CARGO_PKG_VERSION"))));
    match config {
        McpServerConfig::Stdio {
            command,
            args,
            env,
            cwd: server_cwd,
            ..
        } => {
            let mut opts = StdioTransportOptions::new(command.clone());
            opts.args = args.clone();
            opts.env = env.clone();
            opts.cwd = server_cwd
                .clone()
                .or_else(|| Some(cwd.display().to_string()));
            let transport = StdioTransport::new(opts);
            client
                .connect_with(transport)
                .await
                .with_context(|| format!("connect stdio MCP `{name}`"))?;
        }
        McpServerConfig::Http { url, headers, .. } => {
            let parsed = url
                .parse::<Url>()
                .with_context(|| format!("invalid MCP url for `{name}`: {url}"))?;
            let mut opts = StreamableHttpTransportOptions::new(parsed);
            opts.headers = headers.clone();
            let transport = StreamableHttpTransport::new(opts);
            client
                .connect_with(transport)
                .await
                .with_context(|| format!("connect HTTP MCP `{name}`"))?;
        }
    }

    let prefix = format!("mcp_{name}_");
    let mounted = mount_mcp_tools(
        Arc::clone(&client),
        McpMountOptions {
            name_prefix: prefix,
            namespace: Some(name.to_string()),
        },
    )
    .await
    .with_context(|| format!("list tools for MCP `{name}`"))?;

    Ok((client, mounted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wrapped_and_flat() {
        let wrapped = r#"{
          "mcpServers": {
            "echo": { "command": "npx", "args": ["-y", "foo"] }
          }
        }"#;
        let map = parse_mcp_json(wrapped).unwrap();
        assert!(matches!(map.get("echo"), Some(McpServerConfig::Stdio { .. })));

        let flat = r#"{ "demo": { "url": "https://example.com/mcp" } }"#;
        let map = parse_mcp_json(flat).unwrap();
        assert!(matches!(map.get("demo"), Some(McpServerConfig::Http { .. })));
    }
}
