//! QuickJS backend (feature `quickjs`).

use async_trait::async_trait;
use rquickjs::{Context, Runtime};
use serde_json::Value;

use crate::sandbox::{CodemodeError, CodemodeSandbox, ToolInvoker};

/// QuickJS sandbox; tool calls use the DSL `await tool(...)` lines (same as the default backend).
pub struct QuickJsCodemodeSandbox {
    tools: ToolInvoker,
}

impl QuickJsCodemodeSandbox {
    /// Create a QuickJS codemode host.
    pub fn new(tools: ToolInvoker) -> Self {
        Self { tools }
    }
}

#[async_trait]
impl CodemodeSandbox for QuickJsCodemodeSandbox {
    fn backend(&self) -> &'static str {
        "quickjs"
    }

    async fn run(&self, source: &str) -> Result<Value, CodemodeError> {
        if source.contains("tool(") {
            return crate::dsl::run_dsl(source, &self.tools).await;
        }
        let script = source.to_string();
        tokio::task::spawn_blocking(move || eval_expression(&script))
            .await
            .map_err(|e| CodemodeError::Runtime(format!("quickjs task failed: {e}")))?
    }
}

fn eval_expression(source: &str) -> Result<Value, CodemodeError> {
    let rt = Runtime::new().map_err(|e| CodemodeError::Runtime(e.to_string()))?;
    let ctx = Context::full(&rt).map_err(|e| CodemodeError::Runtime(e.to_string()))?;
    ctx.with(|cx| {
        let _: () = cx
            .eval(source.to_string())
            .map_err(|e| CodemodeError::Runtime(e.to_string()))?;
        Ok(Value::Null)
    })
}
