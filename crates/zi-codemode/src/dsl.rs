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
    // Models often emit `const r = await tool(...);`; strip binding + trailing `;`.
    let line = strip_assignment_prefix(line.trim()).trim_end_matches(';').trim();
    let rest = line
        .strip_prefix("await ")
        .or_else(|| line.strip_prefix("await\t"))
        .unwrap_or(line);
    if !rest.starts_with("tool(") {
        return Err(format!(
            "expected `await tool(\"name\", {{}})` (optional `const x =` prefix); got: {line}"
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
        parse_tool_args_json(args_part.trim())?
    };
    Ok((name, args))
}

/// Parse tool args as JSON; tolerate JS-style unquoted object keys (`{goal: "..."}`).
fn parse_tool_args_json(raw: &str) -> Result<Value, String> {
    match serde_json::from_str(raw) {
        Ok(v) => Ok(v),
        Err(first) => {
            let repaired = quote_unquoted_object_keys(raw);
            serde_json::from_str(&repaired).map_err(|e| {
                format!("invalid JSON arguments: {first} (after key repair: {e})")
            })
        }
    }
}

fn quote_unquoted_object_keys(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut chars = raw.chars().peekable();
    let mut in_string = false;
    let mut escape = false;
    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
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
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '{' | ',' => {
                out.push(ch);
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    out.push(chars.next().unwrap());
                }
                if chars.peek().is_some_and(|c| c.is_ascii_alphabetic() || *c == '_' || *c == '$')
                {
                    let mut ident = String::new();
                    while let Some(c) = chars.peek().copied() {
                        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                            ident.push(c);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        ident.push(chars.next().unwrap());
                    }
                    if chars.peek() == Some(&':') {
                        out.push('"');
                        out.push_str(ident.trim_end());
                        out.push('"');
                        let trailing_ws = &ident[ident.trim_end().len()..];
                        out.push_str(trailing_ws);
                    } else {
                        out.push_str(&ident);
                    }
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Drop `const x =` / `let x =` / `var x =` when the RHS is a tool call.
fn strip_assignment_prefix(line: &str) -> &str {
    let Some(eq) = line.find('=') else {
        return line;
    };
    if line.as_bytes().get(eq + 1) == Some(&b'=') {
        return line;
    }
    let lhs = line[..eq].trim();
    let rhs = line[eq + 1..].trim_start();
    let looks_like_binding = !lhs.is_empty()
        && lhs
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c.is_whitespace())
        && (lhs.starts_with("const ")
            || lhs.starts_with("let ")
            || lhs.starts_with("var ")
            || (!lhs.contains(' ') && !lhs.contains('!')));
    if looks_like_binding && (rhs.starts_with("await ") || rhs.starts_with("tool(")) {
        return rhs;
    }
    line
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_const_assignment_prefix() {
        let (name, args) = parse_tool_line(
            r#"const r = await tool("explore", {"goal": "architecture"});"#,
        )
        .unwrap();
        assert_eq!(name, "explore");
        assert_eq!(args["goal"], "architecture");
    }

    #[test]
    fn plain_await_tool_still_works() {
        let (name, args) = parse_tool_line(r#"await tool("ls", {"path": "."})"#).unwrap();
        assert_eq!(name, "ls");
        assert_eq!(args["path"], ".");
    }

    #[test]
    fn rejects_non_tool_js() {
        assert!(parse_tool_line("const x = 1 + 2;").is_err());
    }

    #[test]
    fn accepts_unquoted_object_keys() {
        let (name, args) =
            parse_tool_line(r#"await tool("explore", {goal: "architecture"})"#).unwrap();
        assert_eq!(name, "explore");
        assert_eq!(args["goal"], "architecture");
    }
}
