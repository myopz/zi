//! REST routes for health, auth, workspaces, sessions, models, and status.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::auth::{authenticate_token, AuthUser};
use super::db::{SessionRow, WorkspaceRow};
use super::state::AppState;
use super::ws;
use crate::modes::load_model_registry;

use super::{
    default_workspace_cwd_for_user, ensure_workspace_dir, resolve_workspace_cwd,
};

/// Build the authenticated + public API router (no static fallback).
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route("/api/defaults", get(defaults))
        .route("/api/workspaces", get(list_workspaces).post(create_workspace))
        .route("/api/workspaces/ensure-default", post(ensure_default_workspace))
        .route(
            "/api/workspaces/{id}",
            patch(rename_workspace).delete(delete_workspace),
        )
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/sessions/{id}",
            get(get_session).patch(rename_session).delete(delete_session),
        )
        .route("/api/models", get(list_models))
        .route("/api/model", post(set_model))
        .route("/api/status", get(status))
        .route("/ws", get(ws::ws_upgrade))
        .with_state(state)
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ErrorBody>) {
    (
        status,
        Json(ErrorBody {
            error: msg.into(),
        }),
    )
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct LoginBody {
    token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    access_token: String,
    user: UserDto,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UserDto {
    id: String,
    username: String,
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<ErrorBody>)> {
    let token = body.token.trim();
    if token.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "token required"));
    }
    let user = authenticate_token(&state, token).map_err(|status| {
        if status == StatusCode::UNAUTHORIZED {
            err(StatusCode::UNAUTHORIZED, "invalid token")
        } else {
            err(status, "login failed")
        }
    })?;
    // First version: login token is the bearer (OpenClaw shared-secret style).
    Ok(Json(LoginResponse {
        access_token: token.to_string(),
        user: UserDto {
            id: user.id,
            username: user.username,
        },
    }))
}

async fn logout() -> Json<Value> {
    // Token-as-bearer: server has nothing to revoke; client clears sessionStorage.
    Json(json!({ "ok": true }))
}

async fn me(user: AuthUser) -> Json<UserDto> {
    Json(UserDto {
        id: user.id,
        username: user.username,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DefaultsDto {
    /// Safe sandbox cwd under `~/.zi/agent/web/workspaces/<userId>` (not the git repo).
    default_workspace_cwd: String,
}

async fn defaults(user: AuthUser) -> Json<DefaultsDto> {
    Json(DefaultsDto {
        default_workspace_cwd: default_workspace_cwd_for_user(&user.id)
            .display()
            .to_string(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDto {
    id: String,
    user_id: String,
    cwd: String,
    name: String,
    updated_at: String,
}

impl From<WorkspaceRow> for WorkspaceDto {
    fn from(w: WorkspaceRow) -> Self {
        Self {
            id: w.id,
            user_id: w.user_id,
            cwd: w.cwd,
            name: w.name,
            updated_at: w.updated_at,
        }
    }
}

async fn list_workspaces(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<WorkspaceDto>>, (StatusCode, Json<ErrorBody>)> {
    let rows = state
        .db
        .list_workspaces(&user.id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(rows.into_iter().map(WorkspaceDto::from).collect()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateWorkspaceBody {
    cwd: String,
    name: Option<String>,
}

async fn create_workspace(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateWorkspaceBody>,
) -> Result<(StatusCode, Json<WorkspaceDto>), (StatusCode, Json<ErrorBody>)> {
    let raw = body.cwd.trim();
    let cwd = if raw.is_empty() {
        default_workspace_cwd_for_user(&user.id)
    } else {
        resolve_workspace_cwd(raw).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?
    };
    ensure_workspace_dir(&cwd).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let cwd_str = cwd.display().to_string();
    let row = state
        .db
        .create_workspace(&user.id, &cwd_str, body.name.as_deref())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(WorkspaceDto::from(row))))
}

/// Move/create Default onto `$ZI_AGENT_DIR/web/workspaces/<userId>` (never a git checkout).
async fn ensure_default_workspace(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<WorkspaceDto>, (StatusCode, Json<ErrorBody>)> {
    let cwd = default_workspace_cwd_for_user(&user.id);
    ensure_workspace_dir(&cwd).map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
    let cwd_str = cwd.display().to_string();
    let row = state
        .db
        .ensure_default_workspace(&user.id, &cwd_str)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(WorkspaceDto::from(row)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameWorkspaceBody {
    name: String,
}

async fn rename_workspace(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<RenameWorkspaceBody>,
) -> Result<Json<WorkspaceDto>, (StatusCode, Json<ErrorBody>)> {
    let row = state
        .db
        .rename_workspace(&user.id, &id, &body.name)
        .map_err(|e| {
            if e.to_string().contains("empty") {
                err(StatusCode::BAD_REQUEST, e.to_string())
            } else {
                err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "workspace not found"))?;
    Ok(Json(WorkspaceDto::from(row)))
}

async fn delete_workspace(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let ok = state
        .db
        .delete_workspace(&user.id, &id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !ok {
        return Err(err(StatusCode::NOT_FOUND, "workspace not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionDto {
    id: String,
    user_id: String,
    workspace_id: String,
    title: String,
    model: String,
    created_at: String,
    updated_at: String,
    revision: i64,
}

impl From<SessionRow> for SessionDto {
    fn from(s: SessionRow) -> Self {
        Self {
            id: s.id,
            user_id: s.user_id,
            workspace_id: s.workspace_id,
            title: s.title,
            model: s.model.unwrap_or_default(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            revision: s.revision,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionDetailDto {
    session: SessionDto,
    messages: Vec<Value>,
    revision: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListSessionsQuery {
    workspace_id: Option<String>,
}

async fn list_sessions(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListSessionsQuery>,
) -> Result<Json<Vec<SessionDto>>, (StatusCode, Json<ErrorBody>)> {
    let rows = state
        .db
        .list_sessions(&user.id, q.workspace_id.as_deref())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(rows.into_iter().map(SessionDto::from).collect()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSessionBody {
    workspace_id: String,
    title: Option<String>,
    model: Option<String>,
}

async fn create_session(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<CreateSessionBody>,
) -> Result<(StatusCode, Json<SessionDto>), (StatusCode, Json<ErrorBody>)> {
    let model = body
        .model
        .or_else(|| state.host_model.clone())
        .or_else(|| crate::settings::load_settings().default_model);
    let row = state
        .db
        .create_session(
            &user.id,
            &body.workspace_id,
            body.title.as_deref(),
            model.as_deref(),
        )
        .map_err(|e| {
            if e.to_string().contains("workspace not found") {
                err(StatusCode::NOT_FOUND, "workspace not found")
            } else {
                err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?;
    Ok((StatusCode::CREATED, Json(SessionDto::from(row))))
}

async fn get_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<SessionDetailDto>, (StatusCode, Json<ErrorBody>)> {
    let session = state
        .db
        .get_session(&user.id, &id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "session not found"))?;
    let revision = session.revision;
    let messages = state
        .db
        .list_messages(&id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        // content_json stores full AgentMessage JSON; skip unreadable rows.
        .filter(|m| !m.content.is_null())
        .map(|m| m.content)
        .collect();
    Ok(Json(SessionDetailDto {
        session: SessionDto::from(session),
        messages,
        revision,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenameSessionBody {
    title: String,
}

async fn rename_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<RenameSessionBody>,
) -> Result<Json<SessionDto>, (StatusCode, Json<ErrorBody>)> {
    let row = state
        .db
        .rename_session(&user.id, &id, &body.title)
        .map_err(|e| {
            if e.to_string().contains("empty") {
                err(StatusCode::BAD_REQUEST, e.to_string())
            } else {
                err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "session not found"))?;
    Ok(Json(SessionDto::from(row)))
}

async fn delete_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorBody>)> {
    let deleted = state
        .db
        .delete_session(&user.id, &id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !deleted {
        return Err(err(StatusCode::NOT_FOUND, "session not found"));
    }
    state.runtimes.remove(&user.id, &id).await;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDto {
    id: String,
    name: String,
    provider: String,
    api: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelsResponse {
    models: Vec<ModelDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<String>,
}

async fn list_models(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<ModelsResponse>, (StatusCode, Json<ErrorBody>)> {
    let registry =
        load_model_registry().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let models = registry
        .models
        .iter()
        .map(|m| ModelDto {
            id: m.id.clone(),
            name: m.name.clone(),
            provider: m.provider.clone(),
            api: m.api.clone(),
        })
        .collect();
    let current = state
        .host_model
        .clone()
        .or_else(|| crate::settings::load_settings().default_model);
    Ok(Json(ModelsResponse { models, current }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetModelBody {
    model: String,
    session_id: Option<String>,
}

async fn set_model(
    State(state): State<AppState>,
    user: AuthUser,
    Json(body): Json<SetModelBody>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorBody>)> {
    let registry =
        load_model_registry().map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if registry.find(&body.model).is_none() {
        return Err(err(
            StatusCode::BAD_REQUEST,
            format!("unknown model id `{}`", body.model),
        ));
    }
    if let Some(session_id) = body.session_id.as_deref() {
        let updated = state
            .db
            .set_session_model(&user.id, session_id, &body.model)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, "session not found"))?;
        state
            .runtimes
            .set_model_if_loaded(&user.id, session_id, &body.model)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(json!({
            "model": updated.model.unwrap_or_else(|| body.model.clone()),
            "revision": updated.revision,
        })));
    }
    if let Err(err_msg) = crate::settings::save_default_model(&body.model) {
        tracing::warn!("failed to persist defaultModel: {err_msg:#}");
    }
    Ok(Json(json!({ "model": body.model })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusQuery {
    session_id: Option<String>,
}

async fn status(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<StatusQuery>,
) -> Result<Json<Value>, (StatusCode, Json<ErrorBody>)> {
    let Some(session_id) = q.session_id.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(Json(json!({ "busy": false })));
    };
    let session = state
        .db
        .get_session(&user.id, session_id)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "session not found"))?;
    let busy = state.runtimes.is_running(&user.id, session_id).await;
    Ok(Json(json!({
        "sessionId": session.id,
        "busy": busy,
        "model": session.model,
        "revision": session.revision,
    })))
}
