//! Interactive terminal mode via zi-tui.

use std::future::Future;

use tokio::sync::mpsc;
use zi_tui::{
    run_interactive, run_interactive_boxed, InteractiveExit, InteractiveOptions,
    InteractivePromptError, InteractiveSubmitResult, LiveUpdate, PromptFuture, TuiError,
};

/// Async handler invoked when the user submits a line (receives a live-update channel).
pub type PromptCallback =
    Box<dyn FnMut(String, mpsc::UnboundedSender<LiveUpdate>) -> PromptFuture + Send>;

/// Runs interactive mode with a caller-provided async submit handler.
pub async fn run(
    options: InteractiveOptions,
    mut on_submit: PromptCallback,
) -> Result<InteractiveExit, TuiError> {
    run_interactive_boxed(options, move |text, tx| on_submit(text, tx)).await
}

/// Same as [`run`] but accepts any future-returning closure (non-boxed).
#[allow(dead_code)] // public library API for custom InteractiveOptions
pub async fn run_with<F, Fut>(
    options: InteractiveOptions,
    mut on_submit: F,
) -> Result<InteractiveExit, TuiError>
where
    F: FnMut(String, mpsc::UnboundedSender<LiveUpdate>) -> Fut,
    Fut: Future<Output = Result<InteractiveSubmitResult, InteractivePromptError>>
        + Send
        + 'static,
{
    run_interactive(options, move |text, tx| on_submit(text, tx)).await
}

/// Builds a boxed submit callback from an async function.
pub fn boxed_prompt<F, Fut>(handler: F) -> PromptCallback
where
    F: FnMut(String, mpsc::UnboundedSender<LiveUpdate>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<InteractiveSubmitResult, InteractivePromptError>> + Send + 'static,
{
    let mut handler = handler;
    Box::new(move |text, tx| {
        let fut = handler(text, tx);
        Box::pin(fut) as PromptFuture
    })
}
