//! Web mode entry: delegates to the axum host in `crate::web`.

use anyhow::Result;

use crate::cli::Cli;

/// Run `--mode web` (HTTP + WebSocket multi-user host).
pub async fn run_web_mode(cli: &Cli) -> Result<i32> {
    crate::web::run_web_mode(cli).await
}
