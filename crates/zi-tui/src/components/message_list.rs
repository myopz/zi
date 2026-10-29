// Scrollable-style message transcript as flat lines.

use serde::{Deserialize, Serialize};

use crate::component::Component;
use crate::components::Text;
use crate::wrap::wrap_text;

/// Role label for a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
    Thinking,
}

/// One message in the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    /// Optional tool name when `role == Tool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Optional tool-call id when `role == Tool` (needed for parallel tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// When true and tools are collapsed, only a one-line summary is shown.
    #[serde(default)]
    pub collapsed_hint: bool,
}

impl Message {
    /// Creates a message with the given role and body.
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_name: None,
            tool_call_id: None,
            collapsed_hint: false,
        }
    }

    /// Tool execution summary / result.
    pub fn tool(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self::tool_with_id(String::new(), name, content)
    }

    /// Tool row keyed by call id so parallel tools do not orphan start rows.
    pub fn tool_with_id(
        id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let id = id.into();
        Self {
            role: MessageRole::Tool,
            content: content.into(),
            tool_name: Some(name.into()),
            tool_call_id: if id.is_empty() { None } else { Some(id) },
            collapsed_hint: true,
        }
    }

    /// Model thinking / reasoning block.
    pub fn thinking(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Thinking,
            content: content.into(),
            tool_name: None,
            tool_call_id: None,
            collapsed_hint: false,
        }
    }
}

/// Renders a list of messages as prefixed wrapped lines.
pub struct MessageList {
    messages: Vec<Message>,
    /// When false, tool bodies are collapsed to a one-line summary.
    pub expand_tools: bool,
    /// When false, thinking messages are omitted.
    pub show_thinking: bool,
}

impl MessageList {
    /// Creates an empty message list.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            expand_tools: false,
            show_thinking: false,
        }
    }

    /// Appends a message to the transcript.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Replaces the last message if it has `role`, otherwise pushes.
    pub fn upsert_last(&mut self, role: MessageRole, content: impl Into<String>) {
        let content = content.into();
        if let Some(last) = self.messages.last_mut() {
            if last.role == role {
                last.content = content;
                return;
            }
        }
        self.messages.push(Message::new(role, content));
    }

    /// Updates this turn's last `role` bubble (after the latest user), or pushes.
    ///
    /// Used when finalizing a streamed assistant reply after trailing system
    /// notices (e.g. TPS) so we do not paint a second assistant bubble.
    pub fn upsert_turn_role(&mut self, role: MessageRole, content: impl Into<String>) {
        let content = content.into();
        let after_user = self
            .messages
            .iter()
            .rposition(|m| m.role == MessageRole::User)
            .map(|i| i + 1)
            .unwrap_or(0);
        if let Some(idx) = self.messages[after_user..]
            .iter()
            .rposition(|m| m.role == role)
            .map(|rel| after_user + rel)
        {
            self.messages[idx].content = content;
            return;
        }
        self.messages.push(Message::new(role, content));
    }

    /// Updates the in-flight tool row for `id`, or pushes a new one.
    ///
    /// Matching only the contiguous last row breaks under parallel tool starts:
    /// start A, start B, end A would otherwise leave A's "running…" row and push
    /// a second A row.
    pub fn upsert_tool(&mut self, id: &str, name: &str, content: impl Into<String>) {
        let content = content.into();
        if !id.is_empty() {
            if let Some(msg) = self.messages.iter_mut().rev().find(|m| {
                m.role == MessageRole::Tool && m.tool_call_id.as_deref() == Some(id)
            }) {
                msg.content = content;
                msg.tool_name = Some(name.to_string());
                return;
            }
        } else if let Some(msg) = self.messages.iter_mut().rev().find(|m| {
            m.role == MessageRole::Tool && m.tool_name.as_deref() == Some(name)
        }) {
            msg.content = content;
            return;
        }
        self.messages
            .push(Message::tool_with_id(id, name, content));
    }

    /// Clears all messages.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Returns the current messages.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Toggle tool body expansion.
    pub fn toggle_tools(&mut self) {
        self.expand_tools = !self.expand_tools;
    }

    /// Toggle thinking visibility.
    pub fn toggle_thinking(&mut self) {
        self.show_thinking = !self.show_thinking;
    }
}

impl Default for MessageList {
    fn default() -> Self {
        Self::new()
    }
}

fn role_prefix(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "you> ",
        MessageRole::Assistant => "zi> ",
        MessageRole::System => "sys> ",
        MessageRole::Tool => "tool> ",
        MessageRole::Thinking => "think> ",
    }
}

impl Component for MessageList {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for message in &self.messages {
            if message.role == MessageRole::Thinking && !self.show_thinking {
                continue;
            }
            let prefix = if message.role == MessageRole::Tool {
                if let Some(name) = &message.tool_name {
                    // Dynamic prefix length: use fixed "tool> " and put name in body.
                    let _ = name;
                }
                role_prefix(message.role)
            } else {
                role_prefix(message.role)
            };
            let body = if message.role == MessageRole::Tool && !self.expand_tools {
                let name = message.tool_name.as_deref().unwrap_or("tool");
                let first = message.content.lines().next().unwrap_or("");
                let summary = if first.len() > 80 {
                    format!("{name}: {}…  (Ctrl+O expand)", &first[..80])
                } else if first.is_empty() {
                    format!("{name}  (Ctrl+O expand)")
                } else {
                    format!("{name}: {first}  (Ctrl+O expand)")
                };
                summary
            } else if message.role == MessageRole::Tool {
                let name = message.tool_name.as_deref().unwrap_or("tool");
                format!("[{name}]\n{}", message.content)
            } else {
                message.content.clone()
            };
            let prefix_width = prefix.len();
            let content_width = width.saturating_sub(prefix_width).max(1);
            let wrapped = wrap_text(&body, content_width);
            for (i, line) in wrapped.into_iter().enumerate() {
                if i == 0 {
                    lines.push(format!("{prefix}{line}"));
                } else {
                    lines.push(format!("{}{}", " ".repeat(prefix_width), line));
                }
            }
            lines.push(String::new());
        }
        lines
    }
}

impl MessageList {
    /// Renders using the shared [`Text`] helper for a single system banner.
    pub fn render_banner(banner: &str, width: usize) -> Vec<String> {
        Text::new(banner, 0).render(width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_tool_updates_by_id_under_parallel_starts() {
        let mut list = MessageList::new();
        list.upsert_tool("a", "bash", "running…");
        list.upsert_tool("b", "ls", "running…");
        list.upsert_tool("a", "bash", "bash done");
        list.upsert_tool("b", "ls", "ls done");

        assert_eq!(list.messages().len(), 2);
        assert_eq!(list.messages()[0].content, "bash done");
        assert_eq!(list.messages()[1].content, "ls done");
    }

    #[test]
    fn upsert_turn_role_updates_assistant_after_system_notice() {
        let mut list = MessageList::new();
        list.push(Message::new(MessageRole::User, "hello"));
        list.upsert_last(MessageRole::Assistant, "partial");
        list.push(Message::new(MessageRole::System, "TPS 1.0"));
        list.upsert_turn_role(MessageRole::Assistant, "final");

        assert_eq!(list.messages().len(), 3);
        assert_eq!(list.messages()[1].role, MessageRole::Assistant);
        assert_eq!(list.messages()[1].content, "final");
        assert_eq!(list.messages()[2].role, MessageRole::System);
    }

    #[test]
    fn upsert_turn_role_does_not_rewrite_prior_turn() {
        let mut list = MessageList::new();
        list.push(Message::new(MessageRole::User, "one"));
        list.push(Message::new(MessageRole::Assistant, "first"));
        list.push(Message::new(MessageRole::User, "two"));
        list.upsert_turn_role(MessageRole::Assistant, "second");

        assert_eq!(list.messages().len(), 4);
        assert_eq!(list.messages()[1].content, "first");
        assert_eq!(list.messages()[3].content, "second");
    }
}
