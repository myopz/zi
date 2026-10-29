//! Shared axum state: SQLite handle and per-(user, session) agent runtimes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use tokio::sync::Mutex;
use zi_ai::SimpleStreamOptions;

use super::db::WebDb;
use zi_ai::Message;

use crate::modes::{
    build_runtime, load_model_registry, resolve_api_key, resolve_model, system_message, CliRuntime,
};
use crate::settings;

/// Process-wide web host state.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<WebDb>,
    pub runtimes: Arc<RuntimeMap>,
    /// Optional `--model` override from the CLI host process.
    pub host_model: Option<String>,
}

impl AppState {
    /// Build state around an opened database.
    pub fn new(db: WebDb, host_model: Option<String>) -> Self {
        Self {
            db: Arc::new(db),
            runtimes: Arc::new(RuntimeMap::default()),
            host_model,
        }
    }
}

/// In-memory agent runtimes keyed by `(user_id, session_id)`.
#[derive(Default)]
pub struct RuntimeMap {
    inner: Mutex<HashMap<(String, String), Arc<SessionRuntime>>>,
}

/// One live agent for a web session.
pub struct SessionRuntime {
    pub runtime: Mutex<CliRuntime>,
}

impl RuntimeMap {
    /// Drop a runtime if present (e.g. session deleted).
    pub async fn remove(&self, user_id: &str, session_id: &str) {
        let mut map = self.inner.lock().await;
        map.remove(&(user_id.to_string(), session_id.to_string()));
    }

    /// Borrow a loaded runtime without creating one.
    pub async fn get(&self, user_id: &str, session_id: &str) -> Option<Arc<SessionRuntime>> {
        let map = self.inner.lock().await;
        map.get(&(user_id.to_string(), session_id.to_string()))
            .cloned()
    }

    /// Whether a runtime exists and the agent is currently running a prompt.
    pub async fn is_running(&self, user_id: &str, session_id: &str) -> bool {
        let map = self.inner.lock().await;
        let Some(handle) = map.get(&(user_id.to_string(), session_id.to_string())) else {
            return false;
        };
        let rt = handle.runtime.lock().await;
        rt.agent.is_running()
    }

    /// Get or lazily create a runtime for the session (cwd from workspace).
    pub async fn get_or_create(
        &self,
        state: &AppState,
        user_id: &str,
        session_id: &str,
    ) -> Result<Arc<SessionRuntime>> {
        let key = (user_id.to_string(), session_id.to_string());
        {
            let map = self.inner.lock().await;
            if let Some(existing) = map.get(&key) {
                return Ok(Arc::clone(existing));
            }
        }

        let session = state
            .db
            .get_session(user_id, session_id)?
            .context("session not found")?;
        let workspace = state
            .db
            .get_workspace(user_id, &session.workspace_id)?
            .context("workspace not found")?;
        let cwd = PathBuf::from(&workspace.cwd);
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);

        let registry = load_model_registry()?;
        let model_id = session
            .model
            .as_deref()
            .or(state.host_model.as_deref());
        let model = resolve_model(model_id, &registry)?;
        let api_key = resolve_api_key(&model, &registry);
        // Web authority is SQLite; skip NDJSON session files.
        let runtime = build_runtime(cwd.clone(), model, true, api_key, registry).await?;

        // SQLite only stores MessageEnd turns (user/assistant/tool), not the
        // initial system prompt. Re-inject it so web matches TUI transcripts.
        let stored = state.db.load_agent_messages(session_id)?;
        if !stored.is_empty() {
            let mut messages = stored;
            let has_system = messages.iter().any(|m| matches!(m, Message::System(_)));
            if !has_system {
                messages.insert(0, system_message(&cwd));
            }
            runtime.agent.replace_messages(messages).await;
        }

        let handle = Arc::new(SessionRuntime {
            runtime: Mutex::new(runtime),
        });

        let mut map = self.inner.lock().await;
        Ok(Arc::clone(map.entry(key).or_insert(handle)))
    }

    /// Apply a model id to a live runtime (if loaded).
    pub async fn set_model_if_loaded(
        &self,
        user_id: &str,
        session_id: &str,
        model_id: &str,
    ) -> Result<()> {
        let map = self.inner.lock().await;
        let Some(handle) = map.get(&(user_id.to_string(), session_id.to_string())) else {
            return Ok(());
        };
        let registry = load_model_registry()?;
        let model = resolve_model(Some(model_id), &registry)?;
        let api_key = resolve_api_key(&model, &registry);
        let stream_options = SimpleStreamOptions {
            stream: zi_ai::StreamOptions {
                api_key,
                ..Default::default()
            },
            ..Default::default()
        };
        let rt = handle.runtime.lock().await;
        rt.agent.set_model(model, stream_options).await;
        if let Err(err) = settings::save_default_model(model_id) {
            tracing::warn!("failed to persist defaultModel: {err:#}");
        }
        Ok(())
    }
}
