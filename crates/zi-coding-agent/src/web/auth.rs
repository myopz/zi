//! Token hashing and Bearer authentication for the web host.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::state::AppState;

/// SHA-256 hex digest of a raw login / bearer token.
pub fn hash_token(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    hex::encode(digest)
}

/// Authenticated user extracted from `Authorization: Bearer …`.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: String,
    pub username: String,
}

/// JSON error body for auth extractor rejections.
#[derive(Serialize)]
pub struct ErrorBody {
    pub error: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (StatusCode, Json<ErrorBody>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorBody {
                        error: "missing Authorization bearer token".into(),
                    }),
                )
            })?;
        let token = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorBody {
                        error: "expected Authorization: Bearer <token>".into(),
                    }),
                )
            })?;
        let user = state
            .db
            .find_user_by_token_hash(&hash_token(token))
            .map_err(|err| {
                tracing::error!("auth db error: {err:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: "auth lookup failed".into(),
                    }),
                )
            })?
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorBody {
                        error: "invalid token".into(),
                    }),
                )
            })?;
        Ok(AuthUser {
            id: user.id,
            username: user.username,
        })
    }
}

/// Resolve a raw token to an [`AuthUser`] (login / WS first-frame auth).
pub fn authenticate_token(state: &AppState, raw: &str) -> Result<AuthUser, StatusCode> {
    let user = state
        .db
        .find_user_by_token_hash(&hash_token(raw))
        .map_err(|err| {
            tracing::error!("auth db error: {err:#}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    Ok(AuthUser {
        id: user.id,
        username: user.username,
    })
}
