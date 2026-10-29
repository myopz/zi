//! Codemode sandbox host API.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use zi_agent::AgentToolResult;
use serde_json::Value;
use thiserror::Error;

/// Callback invoked when codemode scripts call a registered tool by name.
pub type ToolInvoker = Arc<
    dyn Fn(
            String,
            Value,
        ) -> Pin<Box<dyn Future<Output = Result<AgentToolResult, CodemodeError>> + Send>>
        + Send
        + Sync,
>;

/// Errors from codemode execution.
#[derive(Debug, Error)]
pub enum CodemodeError {
    #[error("codemode parse error: {0}")]
    Parse(String),

    #[error("codemode runtime error: {0}")]
    Runtime(String),

    #[error("tool {name}: {message}")]
    Tool { name: String, message: String },
}

/// Executes codemode scripts against registered Rust tool callbacks.
#[async_trait]
pub trait CodemodeSandbox: Send + Sync {
    /// Human-readable backend id (`dsl` or `quickjs`).
    fn backend(&self) -> &'static str;

    /// Run script source; scripts call tools via the host bridge.
    async fn run(&self, source: &str) -> Result<Value, CodemodeError>;
}

/// DSL fallback: one statement per line, `await tool("name", {..json..})`.
pub struct DslCodemodeSandbox {
    tools: ToolInvoker,
}

impl DslCodemodeSandbox {
    /// Create a sandbox that dispatches to `tools`.
    pub fn new(tools: ToolInvoker) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl CodemodeSandbox for DslCodemodeSandbox {
    fn backend(&self) -> &'static str {
        "dsl"
    }

    async fn run(&self, source: &str) -> Result<Value, CodemodeError> {
        crate::dsl::run_dsl(source, &self.tools).await
    }
}

/// Build the default sandbox (QuickJS when enabled, otherwise DSL).
pub fn default_sandbox(tools: ToolInvoker) -> Arc<dyn CodemodeSandbox> {
    #[cfg(feature = "quickjs")]
    {
        return Arc::new(crate::quickjs::QuickJsCodemodeSandbox::new(tools));
    }
    #[cfg(not(feature = "quickjs"))]
    {
        Arc::new(DslCodemodeSandbox::new(tools))
    }
}

/// Adapter from a name -> handler map into [`ToolInvoker`].
pub fn tool_map_invoker(
    handlers: HashMap<String, ToolInvokerEntry>,
) -> ToolInvoker {
    Arc::new(move |name, args| {
        let handlers = handlers.clone();
        Box::pin(async move {
            let Some(entry) = handlers.get(&name) else {
                return Err(CodemodeError::Tool {
                    name,
                    message: "not registered".into(),
                });
            };
            (entry)(args).await
        })
    })
}

/// One registered tool handler for codemode.
pub type ToolInvokerEntry = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = Result<AgentToolResult, CodemodeError>> + Send>>
        + Send
        + Sync,
>;
