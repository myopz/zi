// Server-sent events line parser for provider streaming bodies.

use bytes::BytesMut;
use futures::{Stream, StreamExt};
use serde_json::Value;

/// One SSE event with optional event name and JSON or raw data payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Parse `data` as JSON when present.
pub fn parse_json_data(data: &str) -> Option<Value> {
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str(data).ok()
}

/// Incrementally parse SSE frames from a byte stream.
pub fn decode_sse<S, E>(byte_stream: S) -> impl Stream<Item = Result<SseEvent, E>>
where
    S: Stream<Item = Result<bytes::Bytes, E>>,
{
    async_stream::stream! {
        let mut buffer = BytesMut::new();
        let mut event_name: Option<String> = None;
        let mut data_lines: Vec<String> = Vec::new();

        futures::pin_mut!(byte_stream);

        while let Some(chunk) = byte_stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };
            buffer.extend_from_slice(&chunk);

            while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                let line_bytes = buffer.split_to(pos + 1);
                let line = String::from_utf8_lossy(&line_bytes).trim_end_matches(['\r', '\n']).to_string();

                if line.is_empty() {
                    if !data_lines.is_empty() || event_name.is_some() {
                        let data = data_lines.join("\n");
                        yield Ok(SseEvent {
                            event: event_name.take(),
                            data,
                        });
                        data_lines.clear();
                    }
                    continue;
                }

                if let Some(rest) = line.strip_prefix("event:") {
                    event_name = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.trim_start().to_string());
                }
            }
        }

        if !data_lines.is_empty() || event_name.is_some() {
            yield Ok(SseEvent {
                event: event_name.take(),
                data: data_lines.join("\n"),
            });
        }
    }
}
