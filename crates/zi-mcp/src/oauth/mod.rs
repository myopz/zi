//! MCP OAuth (authorization code + refresh) — not implemented in Phase 1.
//!
//! TODO(Phase 5): port discovery, callback server, and token storage from pi `packages/mcp/src/oauth`.

#![cfg(feature = "oauth")]

use crate::error::McpError;

/// Placeholder until OAuth is implemented.
pub fn not_implemented() -> Result<(), McpError> {
    Err(McpError::Other("MCP OAuth is not implemented yet".into()))
}
