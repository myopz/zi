//! Minimal codemode DSL: `await tool("name", {..json..})` per line.

use zi_agent::AgentToolResult;
use serde_json::Value;

use crate::sandbox::{CodemodeError, ToolInvoker};

/// Execute a DSL script line by line.
pub async fn run_dsl(source: &str, tools: &ToolInvoker) -> Result<Value, CodemodeError> {
    let mut last: Value = Value::Null;
    for (line_no, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        let (name, args) = parse_tool_line(line).map_err(|msg| CodemodeError::Parse(format!(
            "line {}: {msg}",
            line_no + 1
        )))?;
        let result = (tools)(name.clone(), args).await.map_err(|err| match err {
            CodemodeError::Tool { name, message } => CodemodeError::Tool { name, message },
            other => CodemodeError::Runtime(other.to_string()),
        })?;
        last = agent_result_to_value(result);
    }
    Ok(last)
}

fn parse_tool_line(line: &str) -> Result<(String, Value), String> {
    let rest = line
        .strip_prefix("await ")
        .or_else(|| line.strip_prefix("await\t"))
        .unwrap_or(line);
    if !rest.starts_with("tool(") {
        return Err(format!(
            "expected `await tool(\"name\", {{}})`; got: {line}"
        ));
    }
    let inner = rest
        .strip_prefix("tool(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| "unclosed tool(...)".to_string())?;
    let (name_part, args_part) = split_first_comma(inner)?;
    let name = parse_string_literal(name_part.trim())?;
    let args = if args_part.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(args_part.trim())
            .map_err(|e| format!("invalid JSON arguments: {e}"))?
    };
    Ok((name, args))
}

fn split_first_comma(s: &str) -> Result<(&str, &str), String> {
    let mut in_string = false;
    let mut escape = false;
    let mut depth: usize = 0;
    for (idx, ch) in s.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                return Ok((&s[..idx], &s[idx + 1..]));
            }
            _ => {}
        }
    }
    Ok((s, ""))
}

fn parse_string_literal(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.len() < 2 || !raw.starts_with('"') || !raw.ends_with('"') {
        return Err(format!("tool name must be a string literal, got {raw}"));
    }
    serde_json::from_str(raw).map_err(|e| format!("invalid tool name string: {e}"))
}

fn agent_result_to_value(result: AgentToolResult) -> Value {
    let text = result
        .content
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::json!({
        "text": text,
        "isError": result.is_error.unwrap_or(false),
        "structuredContent": result.structured_content,
        "details": result.details,
    })
}
