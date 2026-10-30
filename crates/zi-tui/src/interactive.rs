// Interactive prompt loop for zi-coding-agent (streaming + scroll + queue).

use std::future::Future;
use std::pin::Pin;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind};
use tokio::sync::mpsc;

use crate::app::TuiApp;
use crate::clipboard;
use crate::component::{Component, InputOutcome};
use crate::components::{Editor, Message, MessageList, MessageRole, SlashSuggestion};
use crate::error::TuiError;

/// Why the interactive session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveExit {
    /// User pressed Ctrl+C while idle (or confirmed quit).
    Aborted,
    /// User ran `/quit` (or alias).
    Quit,
}

/// How the submit handler wants the transcript updated (non-streaming path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractiveSubmitResult {
    /// LLM (or equivalent) reply shown as assistant.
    Assistant(String),
    /// Local slash / status notice shown as system.
    System(String),
    /// Clear the on-screen transcript, optionally seed messages, then show a notice.
    Reset {
        notice: String,
        /// Messages to show after clearing (e.g. `/resume` transcript).
        seed: Vec<(MessageRole, String)>,
    },
    /// End the session cleanly.
    Quit,
}

/// Live patches pushed while a prompt is running (streaming).
#[derive(Debug, Clone)]
pub enum LiveUpdate {
    /// Replace/create the streaming assistant bubble.
    AssistantText(String),
    /// Replace/create the streaming thinking bubble.
    Thinking(String),
    /// Upsert a tool row (keyed by `id` so parallel tools update in place).
    Tool {
        id: String,
        name: String,
        body: String,
    },
    /// System notice line.
    System(String),
    /// Sticky prompt-context widget above the status line (`None` clears).
    Widget(Option<String>),
    /// Refresh status fields.
    Status(StatusInfo),
    /// Agent turn finished successfully.
    Done,
    /// Agent turn finished with error / abort.
    Error(String),
}

/// Footer status line fields.
#[derive(Debug, Clone, Default)]
pub struct StatusInfo {
    pub cwd: String,
    pub model: String,
    pub busy: bool,
    pub queue_len: usize,
    pub extra: String,
}

/// Optional remapped action chords (normalized lowercase, e.g. `ctrl+q`).
#[derive(Clone, Debug, Default)]
pub struct KeybindingOverrides {
    /// Chord that should abort a busy turn (default: Escape).
    pub abort: Option<String>,
    /// Chord that should quit when idle (default: Ctrl+C).
    pub quit: Option<String>,
    /// Chord that copies the last reply (default: Ctrl+Y).
    pub copy: Option<String>,
}

/// Configuration for [`run_interactive`].
#[derive(Clone)]
pub struct InteractiveOptions {
    /// Optional banner shown above the transcript.
    pub banner: Option<String>,
    /// Slash command suggestions for `/` autocomplete.
    pub slash_suggestions: Vec<SlashSuggestion>,
    /// Workspace cwd for `@` path completion and status.
    pub cwd: Option<String>,
    /// Initial model id for the status line.
    pub model: Option<String>,
    /// Invoked on Escape / Ctrl+C while a run is busy.
    pub on_abort: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// Optional remapped key chords from `keybindings.json`.
    pub keybindings: KeybindingOverrides,
    /// Optional shared counter updated after each present with [`TuiApp::full_redraws`].
    pub redraw_counter: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
}

impl Default for InteractiveOptions {
    fn default() -> Self {
        Self {
            banner: None,
            slash_suggestions: Vec::new(),
            cwd: None,
            model: None,
            on_abort: None,
            keybindings: KeybindingOverrides::default(),
            redraw_counter: None,
        }
    }
}

impl std::fmt::Debug for InteractiveOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InteractiveOptions")
            .field("banner", &self.banner)
            .field("keybindings", &self.keybindings)
            .field("slash_suggestions", &self.slash_suggestions)
            .field("cwd", &self.cwd)
            .field("model", &self.model)
            .field("on_abort", &self.on_abort.as_ref().map(|_| "set"))
            .field(
                "redraw_counter",
                &self.redraw_counter.as_ref().map(|_| "set"),
            )
            .finish()
    }
}

fn sync_redraw_counter(options: &InteractiveOptions, app: &crate::app::TuiApp) {
    if let Some(counter) = &options.redraw_counter {
        counter.store(
            app.full_redraws(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

fn present(
    app: &mut crate::app::TuiApp,
    root: &mut InteractiveRoot,
    options: &InteractiveOptions,
    width: usize,
) -> Result<(), TuiError> {
    app.draw_lines(&root.frame(width, root.term_height))?;
    sync_redraw_counter(options, app);
    Ok(())
}

/// Error from the async prompt handler.
#[derive(Debug, thiserror::Error)]
pub enum InteractivePromptError {
    #[error("{0}")]
    Message(String),
}

struct InteractiveRoot {
    messages: MessageList,
    editor: Editor,
    banner: Option<String>,
    scroll_from_bottom: usize,
    term_height: usize,
    status: StatusInfo,
    /// Prompts waiting while the agent is busy.
    queue: Vec<String>,
    /// Streaming assistant text currently shown (for Ctrl+Y).
    last_assistant: String,
    /// Sticky context widget (prompt-url etc.), rendered above the status line.
    widget: Option<String>,
}

impl InteractiveRoot {
    fn new(options: &InteractiveOptions) -> Self {
        let mut editor = Editor::new("> ");
        editor.set_slash_suggestions(options.slash_suggestions.clone());
        if let Some(cwd) = &options.cwd {
            editor.set_cwd(cwd);
        }
        Self {
            messages: MessageList::new(),
            editor,
            banner: options.banner.clone(),
            scroll_from_bottom: 0,
            term_height: 24,
            status: StatusInfo {
                cwd: options.cwd.clone().unwrap_or_default(),
                model: options.model.clone().unwrap_or_default(),
                busy: false,
                queue_len: 0,
                extra: String::new(),
            },
            queue: Vec::new(),
            last_assistant: String::new(),
            widget: None,
        }
    }

    fn body_lines(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some(banner) = &self.banner {
            lines.extend(MessageList::render_banner(banner, width));
            lines.push(String::new());
        }
        lines.extend(self.messages.render(width));
        lines
    }

    fn status_line(&self, width: usize) -> String {
        let busy = if self.status.busy { "busy" } else { "idle" };
        let queue = if self.status.queue_len > 0 {
            format!(" · queue {}", self.status.queue_len)
        } else {
            String::new()
        };
        let scroll = if self.scroll_from_bottom > 0 {
            format!(" · ↑{}", self.scroll_from_bottom)
        } else {
            String::new()
        };
        let tools = if self.messages.expand_tools {
            "tools:on"
        } else {
            "tools:off"
        };
        let think = if self.messages.show_thinking {
            "think:on"
        } else {
            "think:off"
        };
        let extra = if self.status.extra.is_empty() {
            String::new()
        } else {
            format!(" · {}", self.status.extra)
        };
        let raw = format!(
            "── {} · {} · {busy}{queue}{scroll} · {tools} · {think}{extra} ",
            self.status.cwd, self.status.model
        );
        crate::width::truncate_to_width(&raw, width)
    }

    fn widget_lines(&self, width: usize) -> Vec<String> {
        let Some(widget) = &self.widget else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for (i, raw) in widget.lines().take(4).enumerate() {
            let prefix = if i == 0 { "╭ " } else { "│ " };
            lines.push(crate::width::truncate_to_width(
                &format!("{prefix}{raw}"),
                width,
            ));
        }
        if !lines.is_empty() {
            lines.push(crate::width::truncate_to_width("╰─", width));
        }
        lines
    }

    fn frame(&mut self, width: usize, height: usize) -> Vec<String> {
        let height = height.max(3);
        let (body_h, widget_lines, editor_lines) = self.layout_parts(width, height);
        let body = self.body_lines(width);
        let mut out = visible_body_window(&body, body_h, self.scroll_from_bottom);
        while out.len() < body_h {
            out.push(String::new());
        }
        out.truncate(body_h);
        out.extend(widget_lines);
        out.push(self.status_line(width));
        out.extend(editor_lines);
        debug_assert_eq!(out.len(), height);
        out
    }

    /// Shared body/footer geometry so scroll math matches what `frame` paints.
    fn layout_parts(&mut self, width: usize, height: usize) -> (usize, Vec<String>, Vec<String>) {
        let height = height.max(3);
        let widget_lines = self.widget_lines(width);
        // Leave room for status + ~2 editor body lines; rest can be the `/` popup.
        let popup_budget = height
            .saturating_sub(2 + widget_lines.len() + 2)
            .clamp(6, 24);
        self.editor.set_popup_max_rows(popup_budget);
        let mut editor_lines = self.editor.render(width);
        if editor_lines.is_empty() {
            editor_lines.push("> \x1b[7m \x1b[27m".into());
        }
        // Pin widget + status + editor at the bottom; trim tall autocomplete from the top.
        let max_editor = height
            .saturating_sub(2 + widget_lines.len())
            .max(1);
        if editor_lines.len() > max_editor {
            let skip = editor_lines.len() - max_editor;
            editor_lines = editor_lines[skip..].to_vec();
        }
        let footer_h = 1 + editor_lines.len() + widget_lines.len();
        let body_h = height.saturating_sub(footer_h).max(1);
        (body_h, widget_lines, editor_lines)
    }

    fn body_height(&mut self, width: usize) -> usize {
        self.layout_parts(width, self.term_height.max(3)).0
    }

    fn max_scroll_from_bottom(&mut self, width: usize) -> usize {
        let body_h = self.body_height(width);
        let body_len = self.body_lines(width).len();
        body_len.saturating_sub(body_h)
    }

    fn clamp_scroll(&mut self, width: usize) {
        let max = self.max_scroll_from_bottom(width);
        if self.scroll_from_bottom > max {
            self.scroll_from_bottom = max;
        }
    }

    fn scroll_by(&mut self, delta: isize, width: usize) -> bool {
        let max = self.max_scroll_from_bottom(width) as isize;
        let next = (self.scroll_from_bottom as isize + delta).clamp(0, max) as usize;
        if next == self.scroll_from_bottom {
            return false;
        }
        self.scroll_from_bottom = next;
        true
    }

    fn scroll_to_bottom(&mut self) -> bool {
        if self.scroll_from_bottom == 0 {
            return false;
        }
        self.scroll_from_bottom = 0;
        true
    }

    fn scroll_to_top(&mut self, width: usize) -> bool {
        let max = self.max_scroll_from_bottom(width);
        if self.scroll_from_bottom == max {
            return false;
        }
        self.scroll_from_bottom = max;
        true
    }

    fn page_step(&mut self, width: usize) -> usize {
        self.body_height(width).saturating_sub(1).max(1)
    }

    fn apply_live(&mut self, update: LiveUpdate) {
        match update {
            LiveUpdate::AssistantText(text) => {
                self.last_assistant = text.clone();
                self.messages
                    .upsert_last(MessageRole::Assistant, text);
                if self.scroll_from_bottom == 0 {
                    // stay pinned
                }
            }
            LiveUpdate::Thinking(text) => {
                self.messages.upsert_last(MessageRole::Thinking, text);
            }
            LiveUpdate::Tool { id, name, body } => {
                self.messages.upsert_tool(&id, &name, body);
            }
            LiveUpdate::System(text) => {
                self.messages
                    .push(Message::new(MessageRole::System, text));
            }
            LiveUpdate::Widget(widget) => {
                self.widget = widget;
            }
            LiveUpdate::Status(info) => {
                self.status = info;
            }
            LiveUpdate::Done | LiveUpdate::Error(_) => {
                self.status.busy = false;
            }
        }
    }

    fn copy_last_assistant(&self) -> String {
        let text = if !self.last_assistant.is_empty() {
            self.last_assistant.as_str()
        } else {
            self.messages
                .messages()
                .iter()
                .rev()
                .find(|m| m.role == MessageRole::Assistant)
                .map(|m| m.content.as_str())
                .unwrap_or("")
        };
        if text.is_empty() {
            return "Nothing to copy (no assistant reply yet).".into();
        }
        match clipboard::write_text(text) {
            Ok(()) => "Copied last reply to clipboard.".into(),
            Err(err) => format!("clipboard error: {err}"),
        }
    }

    fn handle_scroll_input(&mut self, event: &Event, width: usize) -> Option<InputOutcome> {
        const WHEEL_LINES: isize = 3;
        match event {
            Event::Mouse(mouse) => {
                let delta = match mouse.kind {
                    MouseEventKind::ScrollUp => WHEEL_LINES,
                    MouseEventKind::ScrollDown => -WHEEL_LINES,
                    _ => return Some(InputOutcome::None),
                };
                if self.scroll_by(delta, width) {
                    Some(InputOutcome::RequestRender)
                } else {
                    Some(InputOutcome::None)
                }
            }
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    return None;
                }
                let page = self.page_step(width) as isize;
                match key.code {
                    KeyCode::PageUp => Some(if self.scroll_by(page, width) {
                        InputOutcome::RequestRender
                    } else {
                        InputOutcome::None
                    }),
                    KeyCode::PageDown => Some(if self.scroll_by(-page, width) {
                        InputOutcome::RequestRender
                    } else {
                        InputOutcome::None
                    }),
                    KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(if self.scroll_to_top(width) {
                            InputOutcome::RequestRender
                        } else {
                            InputOutcome::None
                        })
                    }
                    KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(if self.scroll_to_bottom() {
                            InputOutcome::RequestRender
                        } else {
                            InputOutcome::None
                        })
                    }
                    KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(if self.scroll_by(1, width) {
                            InputOutcome::RequestRender
                        } else {
                            InputOutcome::None
                        })
                    }
                    KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        Some(if self.scroll_by(-1, width) {
                            InputOutcome::RequestRender
                        } else {
                            InputOutcome::None
                        })
                    }
                    KeyCode::Char('o') | KeyCode::Char('O')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        self.messages.toggle_tools();
                        Some(InputOutcome::RequestRender)
                    }
                    KeyCode::Char('t') | KeyCode::Char('T')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        self.messages.toggle_thinking();
                        Some(InputOutcome::RequestRender)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

fn visible_body_window(body: &[String], body_h: usize, scroll_from_bottom: usize) -> Vec<String> {
    if body_h == 0 {
        return Vec::new();
    }
    let max_scroll = body.len().saturating_sub(body_h);
    let scroll = scroll_from_bottom.min(max_scroll);
    let end = body.len().saturating_sub(scroll);
    let start = end.saturating_sub(body_h);
    body.get(start..end).unwrap_or(&[]).to_vec()
}

/// Runs interactive UI with optional streaming via `live_tx` inside the submit future.
pub async fn run_interactive<F, Fut, E>(
    options: InteractiveOptions,
    mut on_submit: F,
) -> Result<InteractiveExit, TuiError>
where
    F: FnMut(String, mpsc::UnboundedSender<LiveUpdate>) -> Fut,
    Fut: Future<Output = Result<InteractiveSubmitResult, E>> + Send + 'static,
    E: std::fmt::Display + Send + 'static,
{
    let mut app = TuiApp::start()?;
    let mut root = InteractiveRoot::new(&options);
    app.refresh_size()?;
    root.term_height = app.height() as usize;
    let mut width = app.width() as usize;
    let mut last_size = (width, root.term_height);
    present(&mut app, &mut root, &options, width)?;

    let (live_tx, mut live_rx) = mpsc::unbounded_channel::<LiveUpdate>();
    let mut pending: Option<tokio::task::JoinHandle<Result<InteractiveSubmitResult, String>>> =
        None;

    loop {
        app.refresh_size()?;
        root.term_height = app.height() as usize;
        width = app.width() as usize;
        root.clamp_scroll(width);
        let size = (width, root.term_height);
        if size != last_size {
            last_size = size;
            present(&mut app, &mut root, &options, width)?;
        }

        // Drain live updates without blocking.
        while let Ok(update) = live_rx.try_recv() {
            let done = matches!(update, LiveUpdate::Done | LiveUpdate::Error(_));
            let err_text = if let LiveUpdate::Error(ref e) = update {
                Some(e.clone())
            } else {
                None
            };
            root.apply_live(update);
            if let Some(err) = err_text {
                root.messages
                    .push(Message::new(MessageRole::System, format!("error: {err}")));
            }
            if done {
                root.status.busy = false;
                root.status.queue_len = root.queue.len();
            }
            root.scroll_from_bottom = 0;
            present(&mut app, &mut root, &options, width)?;
        }

        // Harvest finished submit task.
        if let Some(handle) = pending.as_ref() {
            if handle.is_finished() {
                let handle = pending.take().unwrap();
                match handle.await {
                    Ok(Ok(InteractiveSubmitResult::Quit)) => {
                        app.stop()?;
                        return Ok(InteractiveExit::Quit);
                    }
                    Ok(Ok(InteractiveSubmitResult::Reset { notice, seed })) => {
                        root.messages.clear();
                        root.last_assistant.clear();
                        root.editor.clear();
                        root.scroll_from_bottom = 0;
                        for (role, content) in seed {
                            if role == MessageRole::Assistant {
                                root.last_assistant = content.clone();
                            }
                            root.messages.push(Message::new(role, content));
                        }
                        root.messages
                            .push(Message::new(MessageRole::System, notice));
                        root.status.busy = false;
                        // Transcript clear shifts every row; drop diff state so
                        // we do not leave a ghost prompt above the real editor.
                        app.invalidate_screen();
                    }
                    Ok(Ok(InteractiveSubmitResult::System(notice))) => {
                        root.messages
                            .push(Message::new(MessageRole::System, notice));
                        root.status.busy = false;
                    }
                    Ok(Ok(InteractiveSubmitResult::Assistant(text))) => {
                        if !text.is_empty() {
                            root.last_assistant = text.clone();
                            // Streaming may already have painted this; system notices
                            // (e.g. TPS) may follow — update this turn's bubble only.
                            root.messages
                                .upsert_turn_role(MessageRole::Assistant, text);
                        }
                        root.status.busy = false;
                    }
                    Ok(Err(err)) => {
                        root.messages.push(Message::new(
                            MessageRole::System,
                            format!("error: {err}"),
                        ));
                        root.status.busy = false;
                    }
                    Err(join_err) => {
                        root.messages.push(Message::new(
                            MessageRole::System,
                            format!("error: {join_err}"),
                        ));
                        root.status.busy = false;
                    }
                }
                root.status.queue_len = root.queue.len();
                present(&mut app, &mut root, &options, width)?;

                // Drain one queued prompt.
                if !root.status.busy {
                    if let Some(next) = root.queue.first().cloned() {
                        root.queue.remove(0);
                        root.status.queue_len = root.queue.len();
                        start_prompt(
                            &mut root,
                            &mut app,
                            &mut pending,
                            &live_tx,
                            &mut on_submit,
                            next,
                            width,
                            &options,
                        )?;
                    }
                }
            }
        }

        if let Some(event) = app.poll_event()? {
            // Remappable abort / copy chords (defaults: Esc, Ctrl+Y).
            if let Event::Key(key) = &event {
                if key.kind != KeyEventKind::Release {
                    let chord = key_chord(key);
                    let abort_chord = options
                        .keybindings
                        .abort
                        .as_deref()
                        .unwrap_or("escape");
                    let copy_chord = options.keybindings.copy.as_deref().unwrap_or("ctrl+y");
                    if chord == abort_chord {
                        if root.status.busy {
                            if let Some(abort) = &options.on_abort {
                                abort();
                            }
                            live_tx
                                .send(LiveUpdate::System("abort requested…".into()))
                                .ok();
                            present(&mut app, &mut root, &options, width)?;
                            continue;
                        }
                    }
                    if chord == copy_chord {
                        let notice = root.copy_last_assistant();
                        root.messages
                            .push(Message::new(MessageRole::System, notice));
                        present(&mut app, &mut root, &options, width)?;
                        continue;
                    }
                }
            }

            if let Some(outcome) = root.handle_scroll_input(&event, width) {
                if outcome == InputOutcome::RequestRender {
                    // Viewport scroll shifts nearly every body row. Index-based
                    // diffs then leave ghost lines that look like duplicates.
                    app.invalidate_screen();
                    present(&mut app, &mut root, &options, width)?;
                }
                continue;
            }

            match root.editor.handle_input(&event) {
                InputOutcome::Abort => {
                    if root.status.busy {
                        if let Some(abort) = &options.on_abort {
                            abort();
                        }
                        continue;
                    }
                    app.stop()?;
                    return Ok(InteractiveExit::Aborted);
                }
                InputOutcome::Submitted(text) => {
                    if root.status.busy {
                        root.queue.push(text);
                        root.status.queue_len = root.queue.len();
                        root.messages.push(Message::new(
                            MessageRole::System,
                            format!("queued ({} waiting)", root.queue.len()),
                        ));
                        root.editor.clear();
                        present(&mut app, &mut root, &options, width)?;
                        continue;
                    }
                    root.messages
                        .push(Message::new(MessageRole::User, text.clone()));
                    root.scroll_from_bottom = 0;
                    root.editor.clear();
                    present(&mut app, &mut root, &options, width)?;
                    start_prompt(
                        &mut root,
                        &mut app,
                        &mut pending,
                        &live_tx,
                        &mut on_submit,
                        text,
                        width,
                        &options,
                    )?;
                }
                InputOutcome::RequestRender => {
                    present(&mut app, &mut root, &options, width)?;
                }
                InputOutcome::None => {}
            }
        }

        // Tiny yield so spawned tasks progress.
        tokio::task::yield_now().await;
    }
}

fn start_prompt<F, Fut, E>(
    root: &mut InteractiveRoot,
    app: &mut TuiApp,
    pending: &mut Option<tokio::task::JoinHandle<Result<InteractiveSubmitResult, String>>>,
    live_tx: &mpsc::UnboundedSender<LiveUpdate>,
    on_submit: &mut F,
    text: String,
    width: usize,
    options: &InteractiveOptions,
) -> Result<(), TuiError>
where
    F: FnMut(String, mpsc::UnboundedSender<LiveUpdate>) -> Fut,
    Fut: Future<Output = Result<InteractiveSubmitResult, E>> + Send + 'static,
    E: std::fmt::Display,
{
    root.status.busy = true;
    root.status.queue_len = root.queue.len();
    let _ = present(app, root, options, width);
    let tx = live_tx.clone();
    let fut = on_submit(text, tx);
    *pending = Some(tokio::spawn(async move {
        fut.await.map_err(|e| e.to_string())
    }));
    Ok(())
}

/// Type-erased submit callback for coding-agent wiring.
pub type PromptFuture = Pin<
    Box<
        dyn Future<Output = Result<InteractiveSubmitResult, InteractivePromptError>> + Send,
    >,
>;

/// Runs interactive mode with a boxed async submit handler (live channel provided).
pub async fn run_interactive_boxed(
    options: InteractiveOptions,
    mut on_submit: impl FnMut(String, mpsc::UnboundedSender<LiveUpdate>) -> PromptFuture + Send,
) -> Result<InteractiveExit, TuiError> {
    run_interactive(options, move |text, tx| on_submit(text, tx)).await
}

fn key_chord(key: &crossterm::event::KeyEvent) -> String {
    use crossterm::event::KeyCode;
    let mut out = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("alt+");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) && !matches!(key.code, KeyCode::Char(_)) {
        out.push_str("shift+");
    }
    match key.code {
        KeyCode::Esc => out.push_str("escape"),
        KeyCode::Enter => out.push_str("enter"),
        KeyCode::Tab => out.push_str("tab"),
        KeyCode::Backspace => out.push_str("backspace"),
        KeyCode::Char(c) => out.push(c.to_ascii_lowercase()),
        other => out.push_str(&format!("{other:?}").to_ascii_lowercase()),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_window_follows_bottom_by_default() {
        let body: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let win = visible_body_window(&body, 4, 0);
        assert_eq!(win, vec!["6", "7", "8", "9"]);
    }

    #[test]
    fn visible_window_scrolls_up() {
        let body: Vec<String> = (0..10).map(|i| i.to_string()).collect();
        let win = visible_body_window(&body, 4, 3);
        assert_eq!(win, vec!["3", "4", "5", "6"]);
    }

    #[test]
    fn frame_includes_status_and_editor() {
        let mut root = InteractiveRoot::new(&InteractiveOptions {
            cwd: Some("/tmp".into()),
            model: Some("m".into()),
            ..Default::default()
        });
        root.messages
            .push(Message::new(MessageRole::Assistant, "hi"));
        root.term_height = 8;
        let frame = root.frame(60, 8);
        assert_eq!(frame.len(), 8);
        assert!(frame.iter().any(|l| l.contains("idle")));
        assert!(frame.last().unwrap().starts_with("> "));
    }
}
