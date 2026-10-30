// Transcript normalization and provider message conversion (Phase 1 subset).

use serde_json::{json, Value};

use crate::types::{
    AssistantContentBlock, Context, Message, SystemContent, Tool, ToolResultContentBlock,
    TranscriptContext, UserContent, UserContentBlock,
};

/// Fold `Context.system_prompt` / `Context.tools` into the transcript.
///
/// When a leading system message already exists (agent loop path), tools and an
/// empty prompt are merged onto it so `declared_tools` sees the executable set.
/// Otherwise a new leading system message is inserted (pi `normalizeContext`).
pub fn normalize_context(context: Context) -> TranscriptContext {
    let mut messages = context.messages;
    let prompt = context.system_prompt.filter(|s| !s.is_empty());
    let tools = context.tools.filter(|t| !t.is_empty());

    match messages.first_mut() {
        Some(Message::System(sys)) => {
            // Agent loop already carries the prompt on this message; only fill
            // content when empty, and always attach the executable tool set.
            if let Some(prompt) = prompt {
                if matches!(&sys.content, SystemContent::Text(text) if text.is_empty()) {
                    sys.content = SystemContent::Text(prompt);
                }
            }
            if let Some(tools) = tools {
                merge_tools_added(&mut sys.tools_added, tools);
            }
        }
        _ => {
            if prompt.is_some() || tools.is_some() {
                messages.insert(
                    0,
                    Message::System(crate::types::SystemMessage {
                        role: crate::types::SystemRole::System,
                        content: SystemContent::Text(prompt.unwrap_or_default()),
                        sections: None,
                        tools_added: tools,
                        tools_removed: None,
                        timestamp: wall_now_ms(),
                    }),
                );
            }
        }
    }

    TranscriptContext { messages }
}

fn merge_tools_added(existing: &mut Option<Vec<Tool>>, incoming: Vec<Tool>) {
    match existing {
        Some(list) => {
            for tool in incoming {
                if let Some(slot) = list.iter_mut().find(|t| t.name == tool.name) {
                    *slot = tool;
                } else {
                    list.push(tool);
                }
            }
        }
        None => *existing = Some(incoming),
    }
}

/// Collect tool definitions declared on system messages (last wins per name).
pub fn declared_tools(messages: &[Message]) -> Vec<Tool> {
    let mut by_name = std::collections::BTreeMap::new();
    for message in messages {
        if let Message::System(sys) = message {
            if let Some(added) = &sys.tools_added {
                for tool in added {
                    by_name.insert(tool.name.clone(), tool.clone());
                }
            }
            if let Some(removed) = &sys.tools_removed {
                for reference in removed {
                    by_name.remove(&reference.name);
                }
            }
        }
    }
    by_name.into_values().collect()
}

/// Extract plain system prompt text from the leading system message.
pub fn leading_system_text(messages: &[Message]) -> Option<String> {
    let Message::System(sys) = messages.first()? else {
        return None;
    };
    match &sys.content {
        SystemContent::Text(text) if !text.is_empty() => Some(text.clone()),
        SystemContent::Blocks(blocks) => {
            let joined = blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        }
        _ => None,
    }
}

/// Convert transcript messages to Anthropic `messages` array (Phase 1: text-focused).
pub fn anthropic_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for message in messages {
        match message {
            Message::System(_) => continue,
            Message::User(user) => {
                out.push(json!({
                    "role": "user",
                    "content": user_content_to_anthropic(&user.content),
                }));
            }
            Message::Assistant(asst) => {
                let blocks = asst
                    .content
                    .iter()
                    .filter_map(assistant_block_to_anthropic)
                    .collect::<Vec<_>>();
                if !blocks.is_empty() {
                    out.push(json!({ "role": "assistant", "content": blocks }));
                }
            }
            Message::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ToolResultContentBlock::Text(t) => Some(t.text.as_str()),
                        ToolResultContentBlock::Image(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tr.tool_call_id,
                        "content": text,
                        "is_error": tr.is_error,
                    }],
                }));
            }
        }
    }
    out
}

fn user_content_to_anthropic(content: &UserContent) -> Value {
    match content {
        UserContent::Text(text) => json!(text),
        UserContent::Blocks(blocks) => {
            let mapped = blocks
                .iter()
                .map(|block| match block {
                    UserContentBlock::Text(t) => json!({ "type": "text", "text": t.text }),
                    UserContentBlock::Image(img) => json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": img.mime_type,
                            "data": img.data,
                        }
                    }),
                })
                .collect::<Vec<_>>();
            Value::Array(mapped)
        }
    }
}

fn assistant_block_to_anthropic(block: &AssistantContentBlock) -> Option<Value> {
    match block {
        AssistantContentBlock::Text { text, .. } => Some(json!({ "type": "text", "text": text })),
        AssistantContentBlock::Thinking {
            thinking,
            thinking_signature,
            ..
        } => Some(json!({
            "type": "thinking",
            "thinking": thinking,
            "signature": thinking_signature,
        })),
        AssistantContentBlock::ToolCall {
            id,
            name,
            arguments,
            ..
        } => Some(json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": arguments,
        })),
    }
}

/// Convert transcript to OpenAI Responses `input` items (Phase 1 subset).
pub fn openai_responses_input(messages: &[Message]) -> Value {
    let mut items: Vec<Value> = Vec::new();
    if let Some(system) = leading_system_text(messages) {
        items.push(json!({
            "role": "developer",
            "content": [{ "type": "input_text", "text": system }],
        }));
    }
    for message in messages {
        match message {
            Message::System(_) => continue,
            Message::User(user) => {
                items.push(json!({
                    "role": "user",
                    "content": user_content_to_openai(&user.content),
                }));
            }
            Message::Assistant(asst) => {
                for block in &asst.content {
                    match block {
                        AssistantContentBlock::Text { text, .. } => {
                            items.push(json!({
                                "role": "assistant",
                                "content": [{ "type": "output_text", "text": text }],
                            }));
                        }
                        AssistantContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                            ..
                        } => {
                            items.push(json!({
                                "type": "function_call",
                                "call_id": id,
                                "name": name,
                                "arguments": arguments.to_string(),
                            }));
                        }
                        AssistantContentBlock::Thinking { .. } => {}
                    }
                }
            }
            Message::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ToolResultContentBlock::Text(t) => Some(t.text.as_str()),
                        ToolResultContentBlock::Image(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                items.push(json!({
                    "type": "function_call_output",
                    "call_id": tr.tool_call_id,
                    "output": text,
                }));
            }
        }
    }
    Value::Array(items)
}

fn user_content_to_openai(content: &UserContent) -> Value {
    match content {
        UserContent::Text(text) => json!([{ "type": "input_text", "text": text }]),
        UserContent::Blocks(blocks) => {
            let mapped = blocks
                .iter()
                .map(|block| match block {
                    UserContentBlock::Text(t) => json!({ "type": "input_text", "text": t.text }),
                    UserContentBlock::Image(img) => json!({
                        "type": "input_image",
                        "detail": "auto",
                        "image_url": format!("data:{};base64,{}", img.mime_type, img.data),
                    }),
                })
                .collect::<Vec<_>>();
            Value::Array(mapped)
        }
    }
}

pub fn anthropic_tools(tools: &[Tool]) -> Vec<Value> {
    anthropic_tools_with_cache(tools, false)
}

/// Anthropic tool definitions; when `cache_last` is set, the final tool gets `cache_control`.
pub fn anthropic_tools_with_cache(tools: &[Tool], cache_last: bool) -> Vec<Value> {
    let last = tools.len().saturating_sub(1);
    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let mut value = json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.parameters,
            });
            if cache_last && index == last {
                value["cache_control"] = json!({ "type": "ephemeral" });
            }
            value
        })
        .collect()
}

/// Attach Anthropic ephemeral `cache_control` to the last content block of the last
/// user-role message (user or tool_result aggregated as user).
pub fn anthropic_attach_message_cache(messages: &mut [Value]) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    let role = last.get("role").and_then(|v| v.as_str()).unwrap_or("");
    if role != "user" && role != "system" {
        return;
    }
    match last.get_mut("content") {
        Some(Value::Array(blocks)) => {
            if let Some(block) = blocks.last_mut() {
                if block.is_object() {
                    block["cache_control"] = json!({ "type": "ephemeral" });
                }
            }
        }
        Some(Value::String(text)) => {
            *last.get_mut("content").unwrap() = json!([{
                "type": "text",
                "text": text.clone(),
                "cache_control": { "type": "ephemeral" },
            }]);
        }
        _ => {}
    }
}

pub fn openai_tools(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        })
        .collect()
}

/// OpenAI Chat Completions `messages` array (Phase 4 subset).
pub fn openai_chat_messages(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(system) = leading_system_text(messages) {
        out.push(json!({ "role": "system", "content": system }));
    }
    for message in messages {
        match message {
            Message::System(_) => continue,
            Message::User(user) => {
                out.push(json!({
                    "role": "user",
                    "content": user_content_to_openai_chat(&user.content),
                }));
            }
            Message::Assistant(asst) => {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for block in &asst.content {
                    match block {
                        AssistantContentBlock::Text { text, .. } => text_parts.push(text.clone()),
                        AssistantContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                            ..
                        } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments.to_string(),
                                }
                            }));
                        }
                        AssistantContentBlock::Thinking { .. } => {}
                    }
                }
                let mut msg = json!({ "role": "assistant" });
                if !text_parts.is_empty() {
                    msg["content"] = json!(text_parts.join("\n"));
                }
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = json!(tool_calls);
                }
                if msg.get("content").is_some() || msg.get("tool_calls").is_some() {
                    out.push(msg);
                }
            }
            Message::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ToolResultContentBlock::Text(t) => Some(t.text.as_str()),
                        ToolResultContentBlock::Image(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push(json!({
                    "role": "tool",
                    "tool_call_id": tr.tool_call_id,
                    "content": text,
                }));
            }
        }
    }
    out
}

fn user_content_to_openai_chat(content: &UserContent) -> Value {
    match content {
        UserContent::Text(text) => json!(text),
        UserContent::Blocks(blocks) => {
            let mapped = blocks
                .iter()
                .map(|block| match block {
                    UserContentBlock::Text(t) => json!({ "type": "text", "text": t.text }),
                    UserContentBlock::Image(img) => json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", img.mime_type, img.data) }
                    }),
                })
                .collect::<Vec<_>>();
            Value::Array(mapped)
        }
    }
}

pub fn openai_chat_tools(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            })
        })
        .collect()
}

/// Google Generative AI `contents` plus optional `systemInstruction`.
pub fn google_contents(messages: &[Message]) -> (Option<Value>, Vec<Value>) {
    let system = leading_system_text(messages).map(|text| json!({ "parts": [{ "text": text }] }));
    let mut contents: Vec<Value> = Vec::new();
    for message in messages {
        match message {
            Message::System(_) => continue,
            Message::User(user) => {
                contents.push(json!({
                    "role": "user",
                    "parts": user_parts_google(&user.content),
                }));
            }
            Message::Assistant(asst) => {
                let mut parts: Vec<Value> = Vec::new();
                for block in &asst.content {
                    match block {
                        AssistantContentBlock::Text { text, .. } => {
                            parts.push(json!({ "text": text }));
                        }
                        AssistantContentBlock::ToolCall { name, arguments, .. } => {
                            parts.push(json!({
                                "functionCall": { "name": name, "args": arguments }
                            }));
                        }
                        AssistantContentBlock::Thinking { thinking, .. } => {
                            parts.push(json!({ "text": thinking, "thought": true }));
                        }
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({ "role": "model", "parts": parts }));
                }
            }
            Message::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ToolResultContentBlock::Text(t) => Some(t.text.as_str()),
                        ToolResultContentBlock::Image(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                contents.push(json!({
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "name": tr.tool_name,
                            "response": { "content": text }
                        }
                    }],
                }));
            }
        }
    }
    (system, contents)
}

fn user_parts_google(content: &UserContent) -> Vec<Value> {
    match content {
        UserContent::Text(text) => vec![json!({ "text": text })],
        UserContent::Blocks(blocks) => blocks
            .iter()
            .map(|block| match block {
                UserContentBlock::Text(t) => json!({ "text": t.text }),
                UserContentBlock::Image(img) => json!({
                    "inlineData": { "mimeType": img.mime_type, "data": img.data }
                }),
            })
            .collect(),
    }
}

pub fn google_tools(tools: &[Tool]) -> Value {
    json!([{
        "functionDeclarations": tools.iter().map(|tool| json!({
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.parameters,
        })).collect::<Vec<_>>()
    }])
}

fn wall_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SystemMessage, SystemRole, Tool};

    #[test]
    fn anthropic_tools_cache_last_only() {
        let tools = vec![
            Tool {
                name: "a".into(),
                description: "A".into(),
                parameters: json!({ "type": "object" }),
            },
            Tool {
                name: "b".into(),
                description: "B".into(),
                parameters: json!({ "type": "object" }),
            },
        ];
        let cached = anthropic_tools_with_cache(&tools, true);
        assert!(cached[0].get("cache_control").is_none());
        assert_eq!(
            cached[1].get("cache_control"),
            Some(&json!({ "type": "ephemeral" }))
        );
    }

    #[test]
    fn anthropic_message_cache_attaches_to_last_user() {
        let mut messages = vec![
            json!({ "role": "assistant", "content": [{ "type": "text", "text": "hi" }] }),
            json!({ "role": "user", "content": [{ "type": "text", "text": "go" }] }),
        ];
        anthropic_attach_message_cache(&mut messages);
        assert_eq!(
            messages[1]["content"][0]["cache_control"],
            json!({ "type": "ephemeral" })
        );
        assert!(messages[0]["content"][0].get("cache_control").is_none());
    }

    fn bash_tool() -> Tool {
        Tool {
            name: "bash".into(),
            description: "run a shell command".into(),
            parameters: json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
            }),
        }
    }

    #[test]
    fn normalize_merges_tools_onto_existing_system() {
        let messages = vec![Message::System(SystemMessage {
            role: SystemRole::System,
            content: SystemContent::Text("You are a coding agent.".into()),
            sections: None,
            tools_added: None,
            tools_removed: None,
            timestamp: 0,
        })];
        let transcript = normalize_context(Context {
            system_prompt: None,
            messages,
            tools: Some(vec![bash_tool()]),
        });
        let tools = declared_tools(&transcript.messages);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "bash");
        assert_eq!(
            leading_system_text(&transcript.messages).as_deref(),
            Some("You are a coding agent.")
        );
        let chat_tools = openai_chat_tools(&tools);
        assert_eq!(chat_tools[0]["type"], "function");
        assert_eq!(chat_tools[0]["function"]["name"], "bash");
    }

    #[test]
    fn normalize_inserts_system_when_messages_lack_one() {
        let transcript = normalize_context(Context {
            system_prompt: Some("hi".into()),
            messages: Vec::new(),
            tools: Some(vec![bash_tool()]),
        });
        assert!(matches!(transcript.messages.first(), Some(Message::System(_))));
        assert_eq!(declared_tools(&transcript.messages).len(), 1);
        assert_eq!(leading_system_text(&transcript.messages).as_deref(), Some("hi"));
    }
}
