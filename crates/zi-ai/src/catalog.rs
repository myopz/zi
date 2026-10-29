// Static model catalog subset for Phase 1.

use crate::types::{KnownApi, Model, ModelCost};

/// Claude Sonnet (Anthropic Messages API).
pub fn claude_sonnet_4() -> Model {
    Model {
        id: "claude-sonnet-4-20250514".into(),
        name: "Claude Sonnet 4".into(),
        api: KnownApi::AnthropicMessages.as_str().into(),
        provider: "anthropic".into(),
        base_url: "https://api.anthropic.com".into(),
        reasoning: true,
        context_window: 200_000,
        max_tokens: 16_384,
        cost: ModelCost {
            input: 3.0 / 1_000_000.0,
            output: 15.0 / 1_000_000.0,
            cache_read: 0.30 / 1_000_000.0,
            cache_write: 3.75 / 1_000_000.0,
        },
    }
}

/// GPT-4.1 (OpenAI Responses API).
pub fn gpt_4_1() -> Model {
    Model {
        id: "gpt-4.1".into(),
        name: "GPT-4.1".into(),
        api: KnownApi::OpenaiResponses.as_str().into(),
        provider: "openai".into(),
        base_url: "https://api.openai.com/v1".into(),
        reasoning: false,
        context_window: 1_047_576,
        max_tokens: 32_768,
        cost: ModelCost {
            input: 2.0 / 1_000_000.0,
            output: 8.0 / 1_000_000.0,
            cache_read: 0.50 / 1_000_000.0,
            cache_write: 0.0,
        },
    }
}

/// DeepSeek Flash (OpenAI-compatible Completions; default DeepSeek model).
pub fn deepseek_flash() -> Model {
    Model {
        id: "deepseek-flash".into(),
        name: "DeepSeek Flash".into(),
        api: KnownApi::OpenaiCompletions.as_str().into(),
        provider: "deepseek".into(),
        base_url: "https://api.deepseek.com".into(),
        reasoning: true,
        context_window: 1_000_000,
        max_tokens: 64_000,
        cost: ModelCost {
            input: 0.30 / 1_000_000.0,
            output: 1.20 / 1_000_000.0,
            cache_read: 0.006 / 1_000_000.0,
            cache_write: 0.0,
        },
    }
}

/// 智谱 GLM-4.7-Flash（免费文本，OpenAI-compatible Completions）。
pub fn glm_4_7_flash() -> Model {
    Model {
        id: "glm-4.7-flash".into(),
        name: "GLM-4.7-Flash".into(),
        api: KnownApi::OpenaiCompletions.as_str().into(),
        provider: "zai".into(),
        base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
        reasoning: false,
        context_window: 200_000,
        max_tokens: 16_384,
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    }
}

/// 智谱 GLM-4V-Flash（免费图像理解，OpenAI-compatible Completions）。
pub fn glm_4v_flash() -> Model {
    Model {
        id: "glm-4v-flash".into(),
        name: "GLM-4V-Flash".into(),
        api: KnownApi::OpenaiCompletions.as_str().into(),
        provider: "zai".into(),
        base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
        reasoning: false,
        context_window: 16_000,
        max_tokens: 1_024,
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    }
}

/// OpenRouter auto router (OpenAI-compatible Completions).
pub fn openrouter_auto() -> Model {
    Model {
        id: "openrouter/auto".into(),
        name: "OpenRouter Auto".into(),
        api: KnownApi::OpenaiCompletions.as_str().into(),
        provider: "openrouter".into(),
        base_url: "https://openrouter.ai/api/v1".into(),
        reasoning: false,
        context_window: 128_000,
        max_tokens: 16_384,
        cost: ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
    }
}

/// All bundled catalog models.
pub fn all_models() -> Vec<Model> {
    vec![
        claude_sonnet_4(),
        gpt_4_1(),
        deepseek_flash(),
        glm_4_7_flash(),
        glm_4v_flash(),
        openrouter_auto(),
    ]
}

/// Look up a model by id in the static catalog.
pub fn find_model(id: &str) -> Option<Model> {
    all_models().into_iter().find(|m| m.id == id)
}
