//! Seed and list web users / login tokens.

use anyhow::{bail, Context as _, Result};
use uuid::Uuid;

use super::auth::hash_token;
use super::db::{UserRow, WebDb};

const DEFAULT_USERS: &[&str] = &["alice", "bob", "admin"];

/// Create a user (if needed) and set a single login token. Returns the raw token.
pub fn seed_user(db: &WebDb, username: &str, token: Option<&str>) -> Result<String> {
    let username = username.trim();
    if username.is_empty() {
        bail!("username must not be empty");
    }
    let user = match db.find_user_by_username(username)? {
        Some(u) => u,
        None => db.insert_user(username)?,
    };
    let raw = match token {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => generate_token(),
    };
    db.upsert_user_token(&user.id, &hash_token(&raw), "login")
        .context("store token hash")?;
    Ok(raw)
}

/// List users in the web database.
pub fn list_users(db: &WebDb) -> Result<Vec<UserRow>> {
    db.list_users()
}

/// When the DB has no users, seed alice/bob/admin and return `(username, raw_token)`.
pub fn seed_default_users_if_empty(db: &WebDb) -> Result<Option<Vec<(String, String)>>> {
    if db.user_count()? > 0 {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(DEFAULT_USERS.len());
    for name in DEFAULT_USERS {
        let token = seed_user(db, name, None)?;
        out.push(((*name).to_string(), token));
    }
    Ok(Some(out))
}

fn generate_token() -> String {
    format!("zi_{}", Uuid::now_v7().as_simple())
}
