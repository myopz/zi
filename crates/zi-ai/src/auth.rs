// API key resolution from environment (aligned with pi env-api-keys.ts).

use std::collections::HashMap;
use std::env;

use crate::error::ZiAiError;

pub const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
pub const OPENAI_API_KEY_ENV: &str = "OPENAI_API_KEY";
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";
pub const ZAI_API_KEY_ENV: &str = "ZAI_API_KEY";
pub const GLM_API_KEY_ENV: &str = "GLM_API_KEY";
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";
pub const GOOGLE_API_KEY_ENV: &str = "GOOGLE_API_KEY";
pub const GEMINI_API_KEY_ENV: &str = "GEMINI_API_KEY";

/// Provider-scoped environment overrides; values beat process env.
pub type ProviderEnv = HashMap<String, String>;

fn env_value(name: &str, overrides: Option<&ProviderEnv>) -> Option<String> {
    if let Some(map) = overrides {
        if let Some(value) = map.get(name) {
            if !value.is_empty() {
                return Some(value.clone());
            }
        }
    }
    env::var(name).ok().filter(|v| !v.is_empty())
}

/// Names of env vars that can supply a key for this provider (discovery).
pub fn find_env_keys(provider: &str) -> Option<Vec<&'static str>> {
    match provider {
        "anthropic" => Some(vec![ANTHROPIC_API_KEY_ENV]),
        "openai" => Some(vec![OPENAI_API_KEY_ENV]),
        "deepseek" => Some(vec![DEEPSEEK_API_KEY_ENV]),
        "zai" | "glm" => Some(vec![ZAI_API_KEY_ENV, GLM_API_KEY_ENV]),
        "openrouter" => Some(vec![OPENROUTER_API_KEY_ENV]),
        "google" => Some(vec![GOOGLE_API_KEY_ENV, GEMINI_API_KEY_ENV]),
        _ => None,
    }
}

/// Resolve an API key for a known provider from env overrides and process env.
pub fn get_env_api_key(provider: &str, env: Option<&ProviderEnv>) -> Option<String> {
    let keys = find_env_keys(provider)?;
    for key in keys {
        if let Some(value) = env_value(key, env) {
            return Some(value);
        }
    }
    None
}

/// Resolve API key for a provider, or return a structured error.
pub fn require_api_key(
    provider: &str,
    explicit: Option<&str>,
    env: Option<&ProviderEnv>,
) -> Result<String, ZiAiError> {
    if let Some(key) = explicit.filter(|k| !k.is_empty()) {
        return Ok(key.to_string());
    }
    get_env_api_key(provider, env).ok_or(ZiAiError::MissingApiKey {
        provider: provider.to_string(),
    })
}
