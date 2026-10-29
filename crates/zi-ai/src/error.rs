// Provider and transport errors for zi-ai.

use thiserror::Error;

/// Errors surfaced before or during a streaming assistant response.
#[derive(Debug, Error)]
pub enum ZiAiError {
    #[error("no API key for provider {provider} (set env or pass api_key)")]
    MissingApiKey { provider: String },

    #[error("unsupported API: {api}")]
    UnsupportedApi { api: String },

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("provider error: {0}")]
    Provider(String),

    #[error("stream protocol error: {0}")]
    Protocol(String),
}
