//! Web host: SQLite auth/store, axum REST + WebSocket, agent RuntimeMap.

mod auth;
mod db;
mod routes;
mod seed;
mod state;
mod ws;

pub use db::{WebDb, WebDbPath};
pub use seed::{list_users, seed_default_users_if_empty, seed_user};
pub use state::AppState;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use axum::http::header::{HeaderValue, CACHE_CONTROL};
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::cli::Cli;
use crate::session_store::resolve_agent_dir;

/// Per-user default agent cwd: `$ZI_AGENT_DIR/web/workspaces/<user_id>` (under `~/.zi`, not the repo).
pub fn default_workspace_cwd_for_user(user_id: &str) -> PathBuf {
    resolve_agent_dir()
        .join("web")
        .join("workspaces")
        .join(user_id)
}

/// Expand `~` / `~/…` and return an absolute path (canonicalize when the path exists).
pub fn resolve_workspace_cwd(raw: &str) -> Result<PathBuf> {
    let expanded = expand_user_path(raw.trim());
    if expanded.as_os_str().is_empty() {
        anyhow::bail!("workspace cwd is empty");
    }
    Ok(std::fs::canonicalize(&expanded).unwrap_or(expanded))
}

fn expand_user_path(raw: &str) -> PathBuf {
    if raw == "~" {
        return home_dir().unwrap_or_else(|| PathBuf::from("/"));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Ensure a workspace directory exists on disk before tools run against it.
pub fn ensure_workspace_dir(cwd: &Path) -> Result<()> {
    std::fs::create_dir_all(cwd)
        .with_context(|| format!("create workspace dir {}", cwd.display()))
}

/// Run `zi web …` admin commands (seed / list users).
pub fn run_web_admin(cli: &crate::cli::WebCli) -> Result<i32> {
    let db = WebDb::open(WebDbPath::default_path())?;
    match &cli.command {
        crate::cli::WebCommand::SeedUser { username, token } => {
            let raw = seed_user(&db, username, token.as_deref())?;
            println!("user={username}");
            println!("token={raw}");
            println!("(store the token now; only its sha256 is kept in the database)");
            Ok(0)
        }
        crate::cli::WebCommand::ListUsers => {
            for u in list_users(&db)? {
                println!("{}\t{}", u.id, u.username);
            }
            Ok(0)
        }
    }
}

/// Start the axum web host (`--mode web`).
pub async fn run_web_mode(cli: &Cli) -> Result<i32> {
    let db_path = WebDbPath::default_path();
    let db = WebDb::open(db_path.clone())?;
    if let Some(seeded) = seed_default_users_if_empty(&db)? {
        eprintln!("zi web: empty database; seeded default users (tokens printed once):");
        for (username, token) in &seeded {
            eprintln!("  {username}\ttoken={token}");
        }
    }

    let state = AppState::new(db, cli.model.clone());
    let dist = resolve_web_dist();
    let app = routes::router(state.clone());
    let app = match dist {
        Some(dir) if dir.is_dir() => {
            tracing::info!(path = %dir.display(), "serving zi-web static assets");
            let index = dir.join("index.html");
            // Local/dev: avoid stale SPA shells after `pnpm build` without a hard refresh.
            let static_svc = ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::overriding(
                    CACHE_CONTROL,
                    HeaderValue::from_static("no-cache"),
                ))
                .service(ServeDir::new(&dir).not_found_service(ServeFile::new(index)));
            app.fallback_service(static_svc)
        }
        _ => {
            tracing::warn!("zi-web/dist not found; API/WS only (build frontend to enable UI)");
            app
        }
    };

    let host = cli.host.as_str();
    if host != "127.0.0.1" && host != "localhost" && host != "::1" {
        tracing::warn!(
            host,
            "binding off loopback; web mode is intended for local use without TLS"
        );
    }

    let addr: SocketAddr = format!("{host}:{}", cli.port)
        .parse()
        .with_context(|| format!("invalid listen address {host}:{}", cli.port))?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("bind {addr}"))?;
    let bound = listener.local_addr().unwrap_or(addr);
    let url = format!("http://{bound}/");
    eprintln!("zi web listening on {url}");
    eprintln!("database {}", db_path.display());

    if !cli.no_open {
        open_browser(&url);
    }

    axum::serve(listener, app.into_make_service())
        .await
        .context("web server")?;
    Ok(0)
}

fn resolve_web_dist() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ZI_WEB_DIST") {
        let path = PathBuf::from(p);
        if path.is_dir() {
            return Some(path);
        }
    }
    let candidates = [
        PathBuf::from("zi-web/dist"),
        PathBuf::from("../zi-web/dist"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../zi-web/dist"),
        resolve_agent_dir().join("web/dist"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

fn open_browser(url: &str) {
    let result = {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(url).spawn()
        }
        #[cfg(target_os = "linux")]
        {
            std::process::Command::new("xdg-open").arg(url).spawn()
        }
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "open browser unsupported on this OS",
            ))
        }
    };
    if let Err(err) = result {
        tracing::debug!("could not open browser: {err}");
    }
}
