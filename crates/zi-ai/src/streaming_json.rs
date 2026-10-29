// Incremental JSON object assembly for streamed tool arguments.

use serde_json::{Map, Value};

/// Accumulates partial JSON for a tool call and parses on finish.
#[derive(Debug, Default)]
pub struct ToolCallArgumentsStream {
    buffer: String,
}

impl ToolCallArgumentsStream {
    /// Append a JSON fragment and return the current best-effort object.
    pub fn append(&mut self, delta: &str) -> Value {
        self.buffer.push_str(delta);
        serde_json::from_str::<Value>(&self.buffer)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Map::new()))
    }

    /// Finalize arguments, returning the tool call fields and optional parse error text.
    pub fn finish(&self) -> (Value, Option<String>) {
        match serde_json::from_str::<Value>(&self.buffer) {
            Ok(Value::Object(map)) => (Value::Object(map), None),
            Ok(_) => (
                Value::Object(Map::new()),
                Some(format!("tool arguments were not a JSON object: {}", self.buffer)),
            ),
            Err(err) => (
                Value::Object(Map::new()),
                Some(format!("{err}; raw: {}", self.buffer)),
            ),
        }
    }
}
