// Length-prefixed frames (pi-env protocol v1, docs/protocol.md).

use bytes::{Buf, BufMut, BytesMut};
use std::convert::TryInto;
use serde_json::Value;
use std::io;

pub const REQUEST: u8 = 1;
pub const RESULT: u8 = 2;
pub const ERROR: u8 = 3;
pub const EVENT: u8 = 4;
pub const CANCEL: u8 = 5;
pub const PING: u8 = 6;

pub const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Frame {
    pub kind: u8,
    pub id: u32,
    pub json: Value,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(kind: u8, id: u32, json: Value) -> Self {
        Self {
            kind,
            id,
            json,
            payload: Vec::new(),
        }
    }

    pub fn with_payload(kind: u8, id: u32, json: Value, payload: Vec<u8>) -> Self {
        Self {
            kind,
            id,
            json,
            payload,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let body = serde_json::to_vec(&self.json).unwrap_or_default();
        let total = 9 + body.len() + self.payload.len();
        let mut out = BytesMut::with_capacity(4 + total);
        out.put_u32(total as u32);
        out.put_u8(self.kind);
        out.put_u32(self.id);
        out.put_u32(body.len() as u32);
        out.extend_from_slice(&body);
        out.extend_from_slice(&self.payload);
        out.to_vec()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// Decode one frame from `buffer`; returns `None` if more bytes are needed.
pub fn decode_frame(buffer: &mut BytesMut) -> io::Result<Option<Frame>> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let length = u32::from_be_bytes(buffer[0..4].try_into().unwrap()) as usize;
    if !(9..=MAX_FRAME).contains(&length) {
        return Err(invalid("frame length out of range"));
    }
    if buffer.len() < 4 + length {
        return Ok(None);
    }
    let mut frame_bytes = buffer.split_to(4 + length);
    frame_bytes.advance(4);
    let kind = frame_bytes[0];
    let id = u32::from_be_bytes([frame_bytes[1], frame_bytes[2], frame_bytes[3], frame_bytes[4]]);
    let json_length =
        u32::from_be_bytes([frame_bytes[5], frame_bytes[6], frame_bytes[7], frame_bytes[8]]) as usize;
    if 9 + json_length > length {
        return Err(invalid("JSON length out of range"));
    }
    let json_start = 9;
    let json_end = json_start + json_length;
    let json = if json_length == 0 {
        Value::Object(Default::default())
    } else {
        serde_json::from_slice(&frame_bytes[json_start..json_end]).unwrap_or(Value::Null)
    };
    let payload = frame_bytes[json_end..length].to_vec();
    Ok(Some(Frame {
        kind,
        id,
        json,
        payload,
    }))
}
