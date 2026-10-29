//! Tool result content blocks and LLM mapping.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

/// Standalone content object shape (MCP schema); prefer [`ContentBlock`] for tool results.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ContentAnnotations>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ContentAnnotations>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ContentAnnotations>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLinkContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ContentAnnotations>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextResourceContents {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobResourceContents {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub blob: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedResourceContent {
    #[serde(rename = "type")]
    pub kind: String,
    pub resource: EmbeddedResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ContentAnnotations>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddedResource {
    Text(TextResourceContents),
    Blob(BlobResourceContents),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<ContentAnnotations>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<ContentAnnotations>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    Audio {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<ContentAnnotations>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    #[serde(rename = "resource_link")]
    ResourceLink {
        uri: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "mimeType")]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<ContentAnnotations>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
    Resource {
        resource: EmbeddedResource,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<ContentAnnotations>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
        meta: Option<Value>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

/// Text and image blocks suitable for LLM APIs (aligned with pi `LlmContent`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LlmContent {
    Text { text: String },
    Image { data: String, #[serde(rename = "mimeType")] mime_type: String },
}

/// Map MCP tool result content to LLM-facing text and images.
pub fn to_llm_content(result: &CallToolResult) -> Vec<LlmContent> {
    let mut out: Vec<LlmContent> = result.content.iter().map(block_to_llm_content).collect();
    if out.is_empty() {
        if let Some(structured) = &result.structured_content {
            let text = serde_json::to_string_pretty(structured).unwrap_or_else(|_| structured.to_string());
            out.push(LlmContent::Text { text });
        }
    }
    out
}

fn block_to_llm_content(block: &ContentBlock) -> LlmContent {
    match block {
        ContentBlock::Text { text, .. } => LlmContent::Text { text: text.clone() },
        ContentBlock::Image { data, mime_type, .. } => LlmContent::Image {
            data: data.clone(),
            mime_type: mime_type.clone(),
        },
        ContentBlock::Audio { mime_type, .. } => LlmContent::Text {
            text: format!("[audio {mime_type} omitted]"),
        },
        ContentBlock::ResourceLink { name, uri, .. } => LlmContent::Text {
            text: format!("{name}: {uri}"),
        },
        ContentBlock::Resource { resource, .. } => match resource {
            EmbeddedResource::Text(t) => LlmContent::Text { text: t.text.clone() },
            EmbeddedResource::Blob(b) => {
                if b.mime_type.as_deref().is_some_and(|m| m.starts_with("image/")) {
                    LlmContent::Image {
                        data: b.blob.clone(),
                        mime_type: b.mime_type.clone().unwrap_or_else(|| "image/*".into()),
                    }
                } else {
                    LlmContent::Text {
                        text: format!(
                            "[binary resource {} ({}) omitted]",
                            b.uri,
                            b.mime_type.as_deref().unwrap_or("unknown type")
                        ),
                    }
                }
            }
        },
    }
}
