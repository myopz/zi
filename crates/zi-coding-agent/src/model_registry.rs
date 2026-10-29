//! Load and compose `models.json` over the static zi-ai catalog.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use zi_ai::{all_models, KnownApi, Model, ModelCost};
use serde::Deserialize;

const DEFAULT_CONTEXT_WINDOW: u32 = 128_000;
const DEFAULT_MAX_TOKENS: u32 = 16_384;

/// Composed model list plus optional per-provider keys from `models.json`.
#[derive(Debug, Clone)]
pub struct ModelRegistry {
    pub models: Vec<Model>,
    /// Resolved API keys from `providers.*.apiKey` only.
    pub provider_api_keys: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsFile {
    #[serde(default)]
    providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConfig {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    models: Vec<ModelConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelConfig {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    api: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    context_window: Option<u32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    cost: Option<CostConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CostConfig {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_read: f64,
    #[serde(default)]
    cache_write: f64,
}

impl ModelRegistry {
    /// Load `<agent_dir>/models.json` (optional) and merge over the static catalog.
    pub fn load(agent_dir: &Path) -> Result<Self> {
        let path = agent_dir.join("models.json");
        let file = if path.is_file() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            serde_json::from_str::<ModelsFile>(&text)
                .with_context(|| format!("parse {}", path.display()))?
        } else {
            ModelsFile {
                providers: HashMap::new(),
            }
        };
        Self::compose(all_models(), file)
    }

    /// Compose builtin models with a parsed config (for tests).
    fn compose(builtin: Vec<Model>, file: ModelsFile) -> Result<Self> {
        let mut models = builtin;
        let mut provider_api_keys = HashMap::new();

        for (provider_id, cfg) in file.providers {
            if let Some(raw) = cfg.api_key.as_deref() {
                if let Some(resolved) = resolve_config_value(raw)? {
                    provider_api_keys.insert(provider_id.clone(), resolved);
                }
            }

            let has_builtin = models.iter().any(|m| m.provider == provider_id);

            if has_builtin {
                for model in models.iter_mut().filter(|m| m.provider == provider_id) {
                    if let Some(url) = cfg.base_url.as_deref() {
                        model.base_url = url.to_string();
                    }
                    if let Some(api) = cfg.api.as_deref() {
                        validate_api(api)?;
                        model.api = api.to_string();
                    }
                }
            }

            for entry in &cfg.models {
                let api = entry
                    .api
                    .as_deref()
                    .or(cfg.api.as_deref())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "provider `{provider_id}` model `{}` needs `api` on the model or provider",
                            entry.id
                        )
                    })?;
                validate_api(api)?;

                let base_url = entry
                    .base_url
                    .as_deref()
                    .or(cfg.base_url.as_deref())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "provider `{provider_id}` model `{}` needs `baseUrl` on the model or provider",
                            entry.id
                        )
                    })?;

                let cost = entry.cost.as_ref().map_or_else(
                    || ModelCost {
                        input: 0.0,
                        output: 0.0,
                        cache_read: 0.0,
                        cache_write: 0.0,
                    },
                    |c| ModelCost {
                        input: c.input,
                        output: c.output,
                        cache_read: c.cache_read,
                        cache_write: c.cache_write,
                    },
                );

                let built = Model {
                    id: entry.id.clone(),
                    name: entry
                        .name
                        .clone()
                        .or_else(|| cfg.name.clone())
                        .unwrap_or_else(|| entry.id.clone()),
                    api: api.to_string(),
                    provider: provider_id.clone(),
                    base_url: base_url.to_string(),
                    reasoning: entry.reasoning.unwrap_or(false),
                    context_window: entry.context_window.unwrap_or(DEFAULT_CONTEXT_WINDOW),
                    max_tokens: entry.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
                    cost,
                };

                if let Some(existing) = models
                    .iter_mut()
                    .find(|m| m.provider == provider_id && m.id == entry.id)
                {
                    *existing = built;
                } else {
                    models.push(built);
                }
            }
        }

        Ok(Self {
            models,
            provider_api_keys,
        })
    }

    /// Look up a model by id.
    pub fn find(&self, id: &str) -> Option<&Model> {
        self.models.iter().find(|m| m.id == id)
    }

    /// API key from `models.json` for a provider, if any.
    pub fn api_key_for(&self, provider: &str) -> Option<&str> {
        self.provider_api_keys.get(provider).map(String::as_str)
    }
}

fn validate_api(api: &str) -> Result<()> {
    if KnownApi::parse(api).is_none() {
        bail!(
            "unsupported api `{api}` (expected anthropic-messages, openai-responses, openai-completions, or google-generative-ai)"
        );
    }
    Ok(())
}

/// Resolve a models.json value: literal, `$NAME`, or `${NAME}`. Empty env → None.
fn resolve_config_value(raw: &str) -> Result<Option<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if let Some(name) = env_ref_name(trimmed) {
        let value = std::env::var(name).unwrap_or_default();
        if value.is_empty() {
            return Ok(None);
        }
        return Ok(Some(value));
    }
    Ok(Some(trimmed.to_string()))
}

fn env_ref_name(raw: &str) -> Option<&str> {
    if let Some(rest) = raw.strip_prefix("${") {
        return rest.strip_suffix('}');
    }
    raw.strip_prefix('$').filter(|name| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zi_ai::KnownApi;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn empty_builtin() -> Vec<Model> {
        vec![Model {
            id: "claude-sonnet-4-20250514".into(),
            name: "Claude Sonnet 4".into(),
            api: KnownApi::AnthropicMessages.as_str().into(),
            provider: "anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            reasoning: true,
            context_window: 200_000,
            max_tokens: 16_384,
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        }]
    }

    #[test]
    fn missing_file_keeps_builtin() {
        let dir = std::env::temp_dir().join(format!(
            "zi-models-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let reg = ModelRegistry::load(&dir).unwrap();
        assert!(reg.find("claude-sonnet-4-20250514").is_some() || !reg.models.is_empty());
    }

    #[test]
    fn overlays_base_url_on_builtin_provider() {
        let file = ModelsFile {
            providers: HashMap::from([(
                "anthropic".into(),
                ProviderConfig {
                    name: None,
                    base_url: Some("https://proxy.example.com".into()),
                    api: None,
                    api_key: None,
                    models: vec![],
                },
            )]),
        };
        let reg = ModelRegistry::compose(empty_builtin(), file).unwrap();
        let m = reg.find("claude-sonnet-4-20250514").unwrap();
        assert_eq!(m.base_url, "https://proxy.example.com");
    }

    #[test]
    fn adds_custom_provider_model() {
        let file = ModelsFile {
            providers: HashMap::from([(
                "ollama".into(),
                ProviderConfig {
                    name: Some("Ollama".into()),
                    base_url: Some("http://localhost:11434/v1".into()),
                    api: Some("openai-completions".into()),
                    api_key: Some("ollama".into()),
                    models: vec![ModelConfig {
                        id: "qwen2.5".into(),
                        name: None,
                        api: None,
                        base_url: None,
                        reasoning: None,
                        context_window: Some(32_000),
                        max_tokens: None,
                        cost: None,
                    }],
                },
            )]),
        };
        let reg = ModelRegistry::compose(empty_builtin(), file).unwrap();
        let m = reg.find("qwen2.5").unwrap();
        assert_eq!(m.provider, "ollama");
        assert_eq!(m.base_url, "http://localhost:11434/v1");
        assert_eq!(m.context_window, 32_000);
        assert_eq!(reg.api_key_for("ollama"), Some("ollama"));
    }

    #[test]
    fn rejects_unknown_api() {
        let file = ModelsFile {
            providers: HashMap::from([(
                "x".into(),
                ProviderConfig {
                    name: None,
                    base_url: Some("https://example.com".into()),
                    api: Some("not-a-real-api".into()),
                    api_key: None,
                    models: vec![ModelConfig {
                        id: "m1".into(),
                        name: None,
                        api: None,
                        base_url: None,
                        reasoning: None,
                        context_window: None,
                        max_tokens: None,
                        cost: None,
                    }],
                },
            )]),
        };
        let err = ModelRegistry::compose(empty_builtin(), file).unwrap_err();
        assert!(err.to_string().contains("unsupported api"), "{err:#}");
    }

    #[test]
    fn resolves_dollar_env() {
        let _g = ENV_LOCK.lock().unwrap();
        let key = format!("ZI_TEST_MODELS_KEY_{}", std::process::id());
        std::env::set_var(&key, "secret-value");
        let file = ModelsFile {
            providers: HashMap::from([(
                "custom".into(),
                ProviderConfig {
                    name: None,
                    base_url: Some("https://example.com/v1".into()),
                    api: Some("openai-completions".into()),
                    api_key: Some(format!("${{{key}}}")),
                    models: vec![ModelConfig {
                        id: "custom-1".into(),
                        name: None,
                        api: None,
                        base_url: None,
                        reasoning: None,
                        context_window: None,
                        max_tokens: None,
                        cost: None,
                    }],
                },
            )]),
        };
        let reg = ModelRegistry::compose(vec![], file).unwrap();
        assert_eq!(reg.api_key_for("custom"), Some("secret-value"));
        std::env::remove_var(&key);
    }

    #[test]
    fn replaces_matching_model_id() {
        let file = ModelsFile {
            providers: HashMap::from([(
                "anthropic".into(),
                ProviderConfig {
                    name: None,
                    base_url: None,
                    api: None,
                    api_key: None,
                    models: vec![ModelConfig {
                        id: "claude-sonnet-4-20250514".into(),
                        name: Some("Proxy Claude".into()),
                        api: Some("anthropic-messages".into()),
                        base_url: Some("https://proxy.example.com".into()),
                        reasoning: Some(false),
                        context_window: Some(100_000),
                        max_tokens: Some(4096),
                        cost: None,
                    }],
                },
            )]),
        };
        let reg = ModelRegistry::compose(empty_builtin(), file).unwrap();
        let m = reg.find("claude-sonnet-4-20250514").unwrap();
        assert_eq!(m.name, "Proxy Claude");
        assert_eq!(m.base_url, "https://proxy.example.com");
        assert!(!m.reasoning);
        assert_eq!(
            reg.models
                .iter()
                .filter(|m| m.id == "claude-sonnet-4-20250514")
                .count(),
            1
        );
    }
}
