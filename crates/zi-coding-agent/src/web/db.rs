//! SQLite schema and queries for the web control plane (`zi-web.sqlite`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{bail, Context as _, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use zi_ai::Message;

use crate::session_store::resolve_agent_dir;

const SCHEMA_VERSION: i64 = 1;

/// Resolved path helpers for the web database file.
pub struct WebDbPath;

impl WebDbPath {
    /// Default DB path: `$ZI_AGENT_DIR/web/zi-web.sqlite`.
    pub fn default_path() -> PathBuf {
        resolve_agent_dir().join("web").join("zi-web.sqlite")
    }
}

/// Thread-safe handle to the web SQLite database.
pub struct WebDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRow {
    pub id: String,
    pub user_id: String,
    pub cwd: String,
    pub name: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub title: String,
    pub model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: Value,
    pub created_at: String,
    pub seq: i64,
}

impl WebDb {
    /// Open (or create) the database and apply schema v1.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create web db dir {}", parent.display()))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("open sqlite {}", path.display()))?;
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            ",
        )?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().expect("web db poisoned");
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );
            ",
        )?;
        let version: Option<i64> = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
                r.get(0)
            })
            .optional()?;
        match version {
            None => {
                conn.execute_batch(SCHEMA_V1)?;
                conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [SCHEMA_VERSION])?;
            }
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) => bail!("unsupported zi-web schema version {v} (want {SCHEMA_VERSION})"),
        }
        Ok(())
    }

    /// Number of users (for empty-db seed detection).
    pub fn user_count(&self) -> Result<i64> {
        let conn = self.conn.lock().expect("web db poisoned");
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        Ok(n)
    }

    /// Insert a user row; returns the user id.
    pub fn insert_user(&self, username: &str) -> Result<UserRow> {
        let conn = self.conn.lock().expect("web db poisoned");
        let id = Uuid::now_v7().to_string();
        let created_at = iso8601_now();
        conn.execute(
            "INSERT INTO users (id, username, created_at) VALUES (?1, ?2, ?3)",
            params![id, username, created_at],
        )
        .with_context(|| format!("insert user `{username}`"))?;
        Ok(UserRow {
            id,
            username: username.to_string(),
            created_at,
        })
    }

    /// Look up a user by username.
    pub fn find_user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        conn.query_row(
            "SELECT id, username, created_at FROM users WHERE username = ?1",
            [username],
            |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    created_at: r.get(2)?,
                })
            },
        )
        .optional()
        .context("find user by username")
    }

    /// List all users ordered by username.
    pub fn list_users(&self) -> Result<Vec<UserRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        let mut stmt =
            conn.prepare("SELECT id, username, created_at FROM users ORDER BY username")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Replace all tokens for a user with one new hashed token (seed / rotate).
    pub fn upsert_user_token(
        &self,
        user_id: &str,
        token_hash: &str,
        label: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("web db poisoned");
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM user_tokens WHERE user_id = ?1", [user_id])?;
        let created_at = iso8601_now();
        tx.execute(
            "INSERT INTO user_tokens (token_hash, user_id, label, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, user_id, label, created_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Resolve a token hash to a user.
    pub fn find_user_by_token_hash(&self, token_hash: &str) -> Result<Option<UserRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        conn.query_row(
            "SELECT u.id, u.username, u.created_at
             FROM user_tokens t
             JOIN users u ON u.id = t.user_id
             WHERE t.token_hash = ?1",
            [token_hash],
            |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    created_at: r.get(2)?,
                })
            },
        )
        .optional()
        .context("find user by token hash")
    }

    /// List workspaces for a user (newest first).
    pub fn list_workspaces(&self, user_id: &str) -> Result<Vec<WorkspaceRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, user_id, cwd, name, updated_at FROM workspaces
             WHERE user_id = ?1 ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([user_id], |r| {
                Ok(WorkspaceRow {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    cwd: r.get(2)?,
                    name: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Ensure the user's Default workspace lives at `target_cwd` (under ~/.zi).
    ///
    /// Migrates a mis-pointed "Default" (e.g. a git checkout) onto the sandbox path
    /// and demotes other rows that incorrectly used the Default name.
    pub fn ensure_default_workspace(
        &self,
        user_id: &str,
        target_cwd: &str,
    ) -> Result<WorkspaceRow> {
        let updated_at = iso8601_now();
        let conn = self.conn.lock().expect("web db poisoned");

        let by_cwd: Option<WorkspaceRow> = conn
            .query_row(
                "SELECT id, user_id, cwd, name, updated_at FROM workspaces
                 WHERE user_id = ?1 AND cwd = ?2",
                params![user_id, target_cwd],
                |r| {
                    Ok(WorkspaceRow {
                        id: r.get(0)?,
                        user_id: r.get(1)?,
                        cwd: r.get(2)?,
                        name: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                },
            )
            .optional()?;

        if let Some(row) = by_cwd {
            if row.name != "Default" {
                conn.execute(
                    "UPDATE workspaces SET name = 'Default', updated_at = ?1 WHERE id = ?2",
                    params![updated_at, row.id],
                )?;
            }
            // Demote other Default rows so only the sandbox is named Default.
            conn.execute(
                "UPDATE workspaces SET name = 'Legacy', updated_at = ?1
                 WHERE user_id = ?2 AND name = 'Default' AND id != ?3",
                params![updated_at, user_id, row.id],
            )?;
            return Ok(WorkspaceRow {
                id: row.id,
                user_id: row.user_id,
                cwd: row.cwd,
                name: "Default".into(),
                updated_at,
            });
        }

        let by_name: Option<WorkspaceRow> = conn
            .query_row(
                "SELECT id, user_id, cwd, name, updated_at FROM workspaces
                 WHERE user_id = ?1 AND name = 'Default'
                 ORDER BY updated_at DESC LIMIT 1",
                params![user_id],
                |r| {
                    Ok(WorkspaceRow {
                        id: r.get(0)?,
                        user_id: r.get(1)?,
                        cwd: r.get(2)?,
                        name: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                },
            )
            .optional()?;

        if let Some(row) = by_name {
            conn.execute(
                "UPDATE workspaces SET cwd = ?1, name = 'Default', updated_at = ?2 WHERE id = ?3",
                params![target_cwd, updated_at, row.id],
            )?;
            return Ok(WorkspaceRow {
                id: row.id,
                user_id: row.user_id,
                cwd: target_cwd.to_string(),
                name: "Default".into(),
                updated_at,
            });
        }

        drop(conn);
        self.create_workspace(user_id, target_cwd, Some("Default"))
    }

    /// Create a workspace (or return existing for same user+cwd).
    pub fn create_workspace(
        &self,
        user_id: &str,
        cwd: &str,
        name: Option<&str>,
    ) -> Result<WorkspaceRow> {
        let conn = self.conn.lock().expect("web db poisoned");
        if let Some(existing) = conn
            .query_row(
                "SELECT id, user_id, cwd, name, updated_at FROM workspaces
                 WHERE user_id = ?1 AND cwd = ?2",
                params![user_id, cwd],
                |r| {
                    Ok(WorkspaceRow {
                        id: r.get(0)?,
                        user_id: r.get(1)?,
                        cwd: r.get(2)?,
                        name: r.get(3)?,
                        updated_at: r.get(4)?,
                    })
                },
            )
            .optional()?
        {
            return Ok(existing);
        }
        let id = Uuid::now_v7().to_string();
        let updated_at = iso8601_now();
        let display = name
            .map(str::to_string)
            .unwrap_or_else(|| default_workspace_name(cwd));
        conn.execute(
            "INSERT INTO workspaces (id, user_id, cwd, name, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, user_id, cwd, display, updated_at],
        )?;
        Ok(WorkspaceRow {
            id,
            user_id: user_id.to_string(),
            cwd: cwd.to_string(),
            name: display,
            updated_at,
        })
    }

    /// Fetch a workspace owned by the user.
    pub fn get_workspace(&self, user_id: &str, workspace_id: &str) -> Result<Option<WorkspaceRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        conn.query_row(
            "SELECT id, user_id, cwd, name, updated_at FROM workspaces
             WHERE id = ?1 AND user_id = ?2",
            params![workspace_id, user_id],
            |r| {
                Ok(WorkspaceRow {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    cwd: r.get(2)?,
                    name: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            },
        )
        .optional()
        .context("get workspace")
    }

    /// List sessions for a user (newest first), optionally filtered by workspace.
    pub fn list_sessions(
        &self,
        user_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Vec<SessionRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        match workspace_id {
            Some(ws) => {
                let mut stmt = conn.prepare(
                    "SELECT id, user_id, workspace_id, title, model, created_at, updated_at, revision
                     FROM sessions
                     WHERE user_id = ?1 AND workspace_id = ?2
                     ORDER BY updated_at DESC",
                )?;
                let rows = stmt
                    .query_map(params![user_id, ws], map_session_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, user_id, workspace_id, title, model, created_at, updated_at, revision
                     FROM sessions WHERE user_id = ?1 ORDER BY updated_at DESC",
                )?;
                let rows = stmt
                    .query_map([user_id], map_session_row)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            }
        }
    }

    /// Create a chat session under a workspace.
    pub fn create_session(
        &self,
        user_id: &str,
        workspace_id: &str,
        title: Option<&str>,
        model: Option<&str>,
    ) -> Result<SessionRow> {
        let conn = self.conn.lock().expect("web db poisoned");
        let owned: i64 = conn.query_row(
            "SELECT COUNT(*) FROM workspaces WHERE id = ?1 AND user_id = ?2",
            params![workspace_id, user_id],
            |r| r.get(0),
        )?;
        if owned == 0 {
            bail!("workspace not found");
        }
        let id = Uuid::now_v7().to_string();
        let now = iso8601_now();
        let title = title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Chat {}", &id[..8]));
        conn.execute(
            "INSERT INTO sessions
             (id, user_id, workspace_id, title, model, created_at, updated_at, revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![id, user_id, workspace_id, title, model, now, now],
        )?;
        Ok(SessionRow {
            id,
            user_id: user_id.to_string(),
            workspace_id: workspace_id.to_string(),
            title,
            model: model.map(str::to_string),
            created_at: now.clone(),
            updated_at: now,
            revision: 0,
        })
    }

    /// Fetch a session owned by the user.
    pub fn get_session(&self, user_id: &str, session_id: &str) -> Result<Option<SessionRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        conn.query_row(
            "SELECT id, user_id, workspace_id, title, model, created_at, updated_at, revision
             FROM sessions WHERE id = ?1 AND user_id = ?2",
            params![session_id, user_id],
            map_session_row,
        )
        .optional()
        .context("get session")
    }

    /// Delete a session and its messages (must be owned by user).
    pub fn delete_session(&self, user_id: &str, session_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("web db poisoned");
        let tx = conn.unchecked_transaction()?;
        let n = tx.execute(
            "DELETE FROM sessions WHERE id = ?1 AND user_id = ?2",
            params![session_id, user_id],
        )?;
        if n == 0 {
            return Ok(false);
        }
        tx.execute("DELETE FROM messages WHERE session_id = ?1", [session_id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Delete a workspace and cascade its sessions/messages (must be owned by user).
    pub fn delete_workspace(&self, user_id: &str, workspace_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("web db poisoned");
        let tx = conn.unchecked_transaction()?;
        let session_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM sessions WHERE workspace_id = ?1 AND user_id = ?2",
            )?;
            let rows = stmt
                .query_map(params![workspace_id, user_id], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        for sid in &session_ids {
            tx.execute("DELETE FROM messages WHERE session_id = ?1", [sid])?;
        }
        tx.execute(
            "DELETE FROM sessions WHERE workspace_id = ?1 AND user_id = ?2",
            params![workspace_id, user_id],
        )?;
        let n = tx.execute(
            "DELETE FROM workspaces WHERE id = ?1 AND user_id = ?2",
            params![workspace_id, user_id],
        )?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// Rename a workspace owned by the user.
    pub fn rename_workspace(
        &self,
        user_id: &str,
        workspace_id: &str,
        name: &str,
    ) -> Result<Option<WorkspaceRow>> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            bail!("workspace name is empty");
        }
        let conn = self.conn.lock().expect("web db poisoned");
        let now = iso8601_now();
        let n = conn.execute(
            "UPDATE workspaces SET name = ?1, updated_at = ?2
             WHERE id = ?3 AND user_id = ?4",
            params![trimmed, now, workspace_id, user_id],
        )?;
        if n == 0 {
            return Ok(None);
        }
        drop(conn);
        self.get_workspace(user_id, workspace_id)
    }

    /// Rename a session owned by the user.
    pub fn rename_session(
        &self,
        user_id: &str,
        session_id: &str,
        title: &str,
    ) -> Result<Option<SessionRow>> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            bail!("session title is empty");
        }
        let conn = self.conn.lock().expect("web db poisoned");
        let now = iso8601_now();
        let n = conn.execute(
            "UPDATE sessions SET title = ?1, updated_at = ?2
             WHERE id = ?3 AND user_id = ?4",
            params![trimmed, now, session_id, user_id],
        )?;
        if n == 0 {
            return Ok(None);
        }
        drop(conn);
        self.get_session(user_id, session_id)
    }

    /// If the session still has a placeholder title, rename it from the first user prompt.
    pub fn maybe_autotitle_session(
        &self,
        user_id: &str,
        session_id: &str,
        prompt: &str,
    ) -> Result<Option<SessionRow>> {
        let Some(session) = self.get_session(user_id, session_id)? else {
            return Ok(None);
        };
        if !is_placeholder_session_title(&session.title) {
            return Ok(None);
        }
        let next = title_from_prompt(prompt);
        if next.is_empty() || next.eq_ignore_ascii_case(&session.title) {
            return Ok(None);
        }
        self.rename_session(user_id, session_id, &next)
    }

    /// Update session model and bump `updated_at`.
    pub fn set_session_model(
        &self,
        user_id: &str,
        session_id: &str,
        model: &str,
    ) -> Result<Option<SessionRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        let now = iso8601_now();
        let n = conn.execute(
            "UPDATE sessions SET model = ?1, updated_at = ?2
             WHERE id = ?3 AND user_id = ?4",
            params![model, now, session_id, user_id],
        )?;
        if n == 0 {
            return Ok(None);
        }
        drop(conn);
        self.get_session(user_id, session_id)
    }

    /// List messages for a session ordered by seq.
    pub fn list_messages(&self, session_id: &str) -> Result<Vec<MessageRow>> {
        let conn = self.conn.lock().expect("web db poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content_json, created_at, seq
             FROM messages WHERE session_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt
            .query_map([session_id], |r| {
                let content_json: String = r.get(3)?;
                let content: Value = serde_json::from_str(&content_json).unwrap_or(Value::Null);
                Ok(MessageRow {
                    id: r.get(0)?,
                    session_id: r.get(1)?,
                    role: r.get(2)?,
                    content,
                    created_at: r.get(4)?,
                    seq: r.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete all stored messages for a session and bump revision (web `/clear` / `/new`).
    pub fn clear_messages(&self, user_id: &str, session_id: &str) -> Result<i64> {
        let conn = self.conn.lock().expect("web db poisoned");
        let tx = conn.unchecked_transaction()?;
        let owned: i64 = tx.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND user_id = ?2",
            params![session_id, user_id],
            |r| r.get(0),
        )?;
        if owned == 0 {
            bail!("session not found");
        }
        tx.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            [session_id],
        )?;
        let now = iso8601_now();
        tx.execute(
            "UPDATE sessions SET revision = revision + 1, updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(revision)
    }

    /// Append a finalized agent message and bump session revision.
    pub fn append_message(
        &self,
        user_id: &str,
        session_id: &str,
        message: &Message,
    ) -> Result<i64> {
        let role = message_role(message);
        let content_json = serde_json::to_string(message).context("serialize message")?;
        let conn = self.conn.lock().expect("web db poisoned");
        let tx = conn.unchecked_transaction()?;
        let owned: i64 = tx.query_row(
            "SELECT COUNT(*) FROM sessions WHERE id = ?1 AND user_id = ?2",
            params![session_id, user_id],
            |r| r.get(0),
        )?;
        if owned == 0 {
            bail!("session not found");
        }
        let next_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session_id = ?1",
            [session_id],
            |r| r.get(0),
        )?;
        let id = Uuid::now_v7().to_string();
        let created_at = iso8601_now();
        tx.execute(
            "INSERT INTO messages (id, session_id, role, content_json, created_at, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, session_id, role, content_json, created_at, next_seq],
        )?;
        tx.execute(
            "UPDATE sessions SET revision = revision + 1, updated_at = ?1 WHERE id = ?2",
            params![created_at, session_id],
        )?;
        let revision: i64 = tx.query_row(
            "SELECT revision FROM sessions WHERE id = ?1",
            [session_id],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(revision)
    }

    /// Load messages as agent transcript values.
    pub fn load_agent_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let rows = self.list_messages(session_id)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            match serde_json::from_value::<Message>(row.content) {
                Ok(m) => out.push(m),
                Err(err) => tracing::warn!(
                    session_id,
                    seq = row.seq,
                    "skip unreadable message row: {err}"
                ),
            }
        }
        Ok(out)
    }
}

fn map_session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: r.get(0)?,
        user_id: r.get(1)?,
        workspace_id: r.get(2)?,
        title: r.get(3)?,
        model: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        revision: r.get(7)?,
    })
}

fn message_role(message: &Message) -> &'static str {
    match message {
        Message::System(_) => "system",
        Message::User(_) => "user",
        Message::Assistant(_) => "assistant",
        Message::ToolResult(_) => "toolResult",
    }
}

fn default_workspace_name(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .to_string()
}

fn is_placeholder_session_title(title: &str) -> bool {
    let t = title.trim();
    if t.is_empty() {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    lower == "new session"
        || lower == "untitled"
        || lower == "session"
        || (lower.starts_with("session ") && t.len() <= 16)
        || (lower.starts_with("chat ") && t.len() <= 16)
}

fn title_from_prompt(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches('/')
        .trim();
    if line.is_empty() {
        return String::new();
    }
    const MAX: usize = 48;
    let mut out = String::new();
    for (i, ch) in line.chars().enumerate() {
        if i >= MAX {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn iso8601_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    // Compact UTC-ish stamp without chrono dep (good enough for ordering/display).
    format!("{secs}.{millis:03}Z")
}

#[cfg(test)]
mod session_title_tests {
    use super::*;

    #[test]
    fn placeholder_detection() {
        assert!(is_placeholder_session_title("New session"));
        assert!(is_placeholder_session_title("Chat 01a18a33"));
        assert!(!is_placeholder_session_title("分析 pi 架构"));
    }

    #[test]
    fn title_from_prompt_truncates() {
        let long = "a".repeat(80);
        let t = title_from_prompt(&long);
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= 49);
    }
}

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cwd TEXT NOT NULL,
    name TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(user_id, cwd)
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    model TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    seq INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workspaces_user_updated
    ON workspaces(user_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_sessions_user_updated
    ON sessions(user_id, updated_at);
CREATE INDEX IF NOT EXISTS idx_messages_session_seq
    ON messages(session_id, seq);
CREATE INDEX IF NOT EXISTS idx_user_tokens_user
    ON user_tokens(user_id);
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::auth::hash_token;

    #[test]
    fn seed_login_and_isolation() {
        let dir = std::env::temp_dir().join(format!("zi-web-db-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zi-web.sqlite");
        let db = WebDb::open(&path).unwrap();
        assert_eq!(db.user_count().unwrap(), 0);

        let alice = db.insert_user("alice").unwrap();
        let bob = db.insert_user("bob").unwrap();
        db.upsert_user_token(&alice.id, &hash_token("alice-tok"), "login")
            .unwrap();
        db.upsert_user_token(&bob.id, &hash_token("bob-tok"), "login")
            .unwrap();

        let u = db.find_user_by_token_hash(&hash_token("alice-tok")).unwrap().unwrap();
        assert_eq!(u.username, "alice");
        assert!(db.find_user_by_token_hash(&hash_token("nope")).unwrap().is_none());

        let wa = db
            .create_workspace(&alice.id, "/tmp/a", Some("A"))
            .unwrap();
        let _wb = db
            .create_workspace(&bob.id, "/tmp/b", Some("B"))
            .unwrap();
        let sa = db
            .create_session(&alice.id, &wa.id, Some("s1"), None)
            .unwrap();
        assert!(db.get_session(&bob.id, &sa.id).unwrap().is_none());
        assert_eq!(db.list_sessions(&alice.id, None).unwrap().len(), 1);
        assert_eq!(
            db.list_sessions(&alice.id, Some(&wa.id)).unwrap().len(),
            1
        );
        assert!(db.list_sessions(&bob.id, None).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
