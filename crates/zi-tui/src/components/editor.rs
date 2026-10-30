// Multiline editor with history, word edits, slash/@ autocomplete.

use std::path::{Path, PathBuf};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::clipboard;
use crate::component::{Component, InputOutcome};
use crate::width::{truncate_to_width, visible_width};

/// One slash completion candidate (name without `/`, plus short summary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashSuggestion {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionKind {
    Slash,
    Path,
}

/// Line-buffer editor with a prompt prefix; Enter submits, Shift+Enter inserts newline.
pub struct Editor {
    prompt: String,
    buffer: String,
    /// Cursor position as a grapheme index into `buffer` (`0..=grapheme_len`).
    cursor: usize,
    /// Horizontal scroll offset in display columns for the active visual line.
    h_scroll: usize,
    on_submit: Option<Box<dyn FnMut(String) + Send>>,
    slash_suggestions: Vec<SlashSuggestion>,
    /// Selected index into the filtered suggestion list.
    completion_index: Option<usize>,
    completion_kind: Option<CompletionKind>,
    path_suggestions: Vec<String>,
    /// Submitted prompt history (oldest → newest).
    history: Vec<String>,
    /// `None` = editing live buffer; `Some(i)` = browsing `history[i]`.
    history_index: Option<usize>,
    /// Buffer restored when leaving history browse without submitting.
    history_draft: String,
    /// Workspace root for `@` path completion.
    cwd: PathBuf,
    /// Max visible editor body lines (excluding suggestion popup).
    max_visible_lines: usize,
    /// Max rows for the `/` / `@` suggestion popup (window scrolls with selection).
    popup_max_rows: usize,
}

impl Editor {
    /// Creates an editor with the given prompt prefix (e.g. `"> "`).
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            buffer: String::new(),
            cursor: 0,
            h_scroll: 0,
            on_submit: None,
            slash_suggestions: Vec::new(),
            completion_index: None,
            completion_kind: None,
            path_suggestions: Vec::new(),
            history: Vec::new(),
            history_index: None,
            history_draft: String::new(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            max_visible_lines: 8,
            popup_max_rows: 14,
        }
    }

    /// Sets the workspace directory used for `@` path completion.
    pub fn set_cwd(&mut self, cwd: impl Into<PathBuf>) {
        self.cwd = cwd.into();
    }

    /// Registers slash command suggestions for `/` autocomplete.
    pub fn set_slash_suggestions(&mut self, suggestions: Vec<SlashSuggestion>) {
        self.slash_suggestions = suggestions;
        self.sync_completion();
    }

    /// Caps how many suggestion rows are painted (selection still scrolls within the full list).
    pub fn set_popup_max_rows(&mut self, rows: usize) {
        self.popup_max_rows = rows.clamp(4, 40);
    }

    /// Whether a slash or path completion popup is open.
    pub fn completion_open(&self) -> bool {
        self.completion_kind.is_some()
    }

    /// Registers a callback invoked when the user submits the buffer.
    pub fn on_submit<F>(&mut self, handler: F)
    where
        F: FnMut(String) + Send + 'static,
    {
        self.on_submit = Some(Box::new(handler));
    }

    /// Clears the edit buffer after a successful submit (keeps history).
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.h_scroll = 0;
        self.completion_index = None;
        self.completion_kind = None;
        self.path_suggestions.clear();
        self.history_index = None;
        self.history_draft.clear();
    }

    /// Replaces the buffer contents and places the cursor at the end.
    pub fn set_buffer(&mut self, text: impl Into<String>) {
        self.buffer = text.into();
        self.cursor = self.grapheme_len();
        self.h_scroll = 0;
        self.history_index = None;
        self.sync_completion();
    }

    /// Returns the current buffer text.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Records a submitted prompt in history (dedupes consecutive duplicates).
    pub fn push_history(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_string());
        }
        self.history_index = None;
        self.history_draft.clear();
    }

    fn prompt_width(&self) -> usize {
        visible_width(&self.prompt)
    }

    fn grapheme_len(&self) -> usize {
        self.buffer.graphemes(true).count()
    }

    fn clamp_cursor(&mut self) {
        let len = self.grapheme_len();
        if self.cursor > len {
            self.cursor = len;
        }
    }

    fn byte_index_at_cursor(&self) -> usize {
        self.buffer
            .grapheme_indices(true)
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.buffer.len())
    }

    fn byte_index_at(&self, grapheme: usize) -> usize {
        self.buffer
            .grapheme_indices(true)
            .nth(grapheme)
            .map(|(i, _)| i)
            .unwrap_or(self.buffer.len())
    }

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.grapheme_len() {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn move_home(&mut self) -> bool {
        let line_start = self.current_line_start();
        if self.cursor == line_start {
            return false;
        }
        self.cursor = line_start;
        true
    }

    fn move_end(&mut self) -> bool {
        let line_end = self.current_line_end();
        if self.cursor == line_end {
            return false;
        }
        self.cursor = line_end;
        true
    }

    fn current_line_start(&self) -> usize {
        let before = &self.buffer[..self.byte_index_at_cursor()];
        let nl = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        self.buffer[..nl].graphemes(true).count()
    }

    fn current_line_end(&self) -> usize {
        let from = self.byte_index_at_cursor();
        let rest = &self.buffer[from..];
        let rel = rest.find('\n').unwrap_or(rest.len());
        self.cursor + self.buffer[from..from + rel].graphemes(true).count()
    }

    fn cursor_line_col(&self) -> (usize, usize) {
        let mut line = 0usize;
        let mut col = 0usize;
        for (i, g) in self.buffer.graphemes(true).enumerate() {
            if i == self.cursor {
                break;
            }
            if g == "\n" {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn move_line(&mut self, delta: isize) -> bool {
        let (line, col) = self.cursor_line_col();
        let lines: Vec<&str> = self.buffer.split('\n').collect();
        if lines.is_empty() {
            return false;
        }
        let target = line as isize + delta;
        if target < 0 || target as usize >= lines.len() {
            return false;
        }
        let target = target as usize;
        let mut idx = 0usize;
        for (li, l) in lines.iter().enumerate() {
            if li == target {
                let gcount = l.graphemes(true).count();
                self.cursor = idx + col.min(gcount);
                return true;
            }
            idx += l.graphemes(true).count() + 1; // + newline
        }
        false
    }

    fn insert_str(&mut self, text: &str) {
        let idx = self.byte_index_at_cursor();
        self.buffer.insert_str(idx, text);
        self.cursor += text.graphemes(true).count();
        self.clamp_cursor();
        self.leave_history_browse();
        self.sync_completion();
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let end = self.byte_index_at_cursor();
        self.cursor -= 1;
        let start = self.byte_index_at_cursor();
        self.buffer.replace_range(start..end, "");
        self.leave_history_browse();
        self.sync_completion();
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.cursor >= self.grapheme_len() {
            return false;
        }
        let start = self.byte_index_at_cursor();
        let end = self.byte_index_at(self.cursor + 1);
        self.buffer.replace_range(start..end, "");
        self.leave_history_browse();
        self.sync_completion();
        true
    }

    fn delete_word_backward(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let end = self.cursor;
        let mut i = end;
        // Skip trailing whitespace.
        while i > 0 {
            let g = self.grapheme_at(i - 1);
            if g.chars().all(char::is_whitespace) {
                i -= 1;
            } else {
                break;
            }
        }
        while i > 0 {
            let g = self.grapheme_at(i - 1);
            if g.chars().all(char::is_whitespace) {
                break;
            }
            i -= 1;
        }
        if i == end {
            return false;
        }
        let start_b = self.byte_index_at(i);
        let end_b = self.byte_index_at(end);
        self.buffer.replace_range(start_b..end_b, "");
        self.cursor = i;
        self.leave_history_browse();
        self.sync_completion();
        true
    }

    fn kill_to_line_end(&mut self) -> bool {
        let end = self.current_line_end();
        if self.cursor >= end {
            // At EOL: delete the newline if present.
            if self.cursor < self.grapheme_len() && self.grapheme_at(self.cursor) == "\n" {
                return self.delete_forward();
            }
            return false;
        }
        let start_b = self.byte_index_at_cursor();
        let end_b = self.byte_index_at(end);
        self.buffer.replace_range(start_b..end_b, "");
        self.leave_history_browse();
        self.sync_completion();
        true
    }

    fn kill_to_line_start(&mut self) -> bool {
        let start = self.current_line_start();
        if self.cursor <= start {
            return false;
        }
        let start_b = self.byte_index_at(start);
        let end_b = self.byte_index_at_cursor();
        self.buffer.replace_range(start_b..end_b, "");
        self.cursor = start;
        self.leave_history_browse();
        self.sync_completion();
        true
    }

    fn grapheme_at(&self, index: usize) -> &str {
        self.buffer
            .graphemes(true)
            .nth(index)
            .unwrap_or("")
    }

    fn leave_history_browse(&mut self) {
        self.history_index = None;
        self.history_draft.clear();
    }

    fn history_prev(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        match self.history_index {
            None => {
                self.history_draft = self.buffer.clone();
                let i = self.history.len() - 1;
                self.history_index = Some(i);
                self.buffer = self.history[i].clone();
            }
            Some(0) => return false,
            Some(i) => {
                let i = i - 1;
                self.history_index = Some(i);
                self.buffer = self.history[i].clone();
            }
        }
        self.cursor = self.grapheme_len();
        self.sync_completion();
        true
    }

    fn history_next(&mut self) -> bool {
        let Some(i) = self.history_index else {
            return false;
        };
        if i + 1 >= self.history.len() {
            self.history_index = None;
            self.buffer = std::mem::take(&mut self.history_draft);
            self.cursor = self.grapheme_len();
            self.sync_completion();
            return true;
        }
        let i = i + 1;
        self.history_index = Some(i);
        self.buffer = self.history[i].clone();
        self.cursor = self.grapheme_len();
        self.sync_completion();
        true
    }

    /// Active slash filter: `Some(prefix)` when buffer is `/…` with no whitespace yet.
    fn slash_filter(&self) -> Option<&str> {
        let trimmed = self.buffer.trim_end();
        let rest = trimmed.strip_prefix('/')?;
        if rest.contains(char::is_whitespace) {
            return None;
        }
        Some(rest)
    }

    fn path_token_before_cursor(&self) -> Option<(usize, String)> {
        let before = &self.buffer[..self.byte_index_at_cursor()];
        let at = before.rfind('@')?;
        let token = &before[at + 1..];
        if token.chars().any(|c| c.is_whitespace()) {
            return None;
        }
        let start_g = before[..at].graphemes(true).count();
        Some((start_g, token.to_string()))
    }

    fn filtered_slash(&self) -> Vec<&SlashSuggestion> {
        let Some(prefix) = self.slash_filter() else {
            return Vec::new();
        };
        let prefix_lower = prefix.to_ascii_lowercase();
        self.slash_suggestions
            .iter()
            .filter(|s| s.name.to_ascii_lowercase().starts_with(&prefix_lower))
            .collect()
    }

    fn sync_completion(&mut self) {
        if self.slash_filter().is_some() {
            let n = self.filtered_slash().len();
            self.path_suggestions.clear();
            self.completion_kind = if n > 0 {
                Some(CompletionKind::Slash)
            } else {
                None
            };
            self.completion_index = if n == 0 {
                None
            } else {
                Some(self.completion_index.unwrap_or(0).min(n - 1))
            };
            return;
        }
        if let Some((_, token)) = self.path_token_before_cursor() {
            self.path_suggestions = list_path_completions(&self.cwd, &token);
            let n = self.path_suggestions.len();
            self.completion_kind = if n > 0 {
                Some(CompletionKind::Path)
            } else {
                None
            };
            self.completion_index = if n == 0 {
                None
            } else {
                Some(self.completion_index.unwrap_or(0).min(n - 1))
            };
            return;
        }
        self.completion_kind = None;
        self.completion_index = None;
        self.path_suggestions.clear();
    }

    fn apply_completion(&mut self) -> bool {
        match self.completion_kind {
            Some(CompletionKind::Slash) => {
                let filtered: Vec<SlashSuggestion> =
                    self.filtered_slash().into_iter().cloned().collect();
                if filtered.is_empty() {
                    return false;
                }
                let idx = self.completion_index.unwrap_or(0).min(filtered.len() - 1);
                self.buffer = format!("/{} ", filtered[idx].name);
                self.cursor = self.grapheme_len();
                self.completion_index = None;
                self.completion_kind = None;
                true
            }
            Some(CompletionKind::Path) => {
                let Some((at_g, _)) = self.path_token_before_cursor() else {
                    return false;
                };
                if self.path_suggestions.is_empty() {
                    return false;
                }
                let idx = self
                    .completion_index
                    .unwrap_or(0)
                    .min(self.path_suggestions.len() - 1);
                let path = self.path_suggestions[idx].clone();
                let start_b = self.byte_index_at(at_g);
                let end_b = self.byte_index_at_cursor();
                let insert = format!("@{path}");
                self.buffer.replace_range(start_b..end_b, &insert);
                self.cursor = at_g + insert.graphemes(true).count();
                self.completion_index = None;
                self.completion_kind = None;
                self.path_suggestions.clear();
                true
            }
            None => false,
        }
    }

    fn ensure_h_scroll(&mut self, field_width: usize) {
        let (_line, col) = self.cursor_line_col();
        // Approximate: treat graphemes as 1 column for scroll bookkeeping.
        if col < self.h_scroll {
            self.h_scroll = col;
        } else if col >= self.h_scroll + field_width {
            self.h_scroll = col + 1 - field_width;
        }
    }

    fn render_editor_body(&self, width: usize) -> Vec<String> {
        let prompt_w = self.prompt_width();
        let field_width = width.saturating_sub(prompt_w).max(1);
        let logical: Vec<&str> = if self.buffer.is_empty() {
            vec![""]
        } else {
            self.buffer.split('\n').collect()
        };
        let (cursor_line, cursor_col) = self.cursor_line_col();
        let total = logical.len();
        let max_vis = self.max_visible_lines.max(1);
        let v_scroll = cursor_line.saturating_sub(max_vis - 1);
        let mut out = Vec::new();
        for (li, line) in logical.iter().enumerate().skip(v_scroll).take(max_vis) {
            let prefix = if li == v_scroll {
                self.prompt.as_str()
            } else {
                ""
            };
            let pad = if li == v_scroll {
                String::new()
            } else {
                " ".repeat(prompt_w)
            };
            let rendered = if li == cursor_line {
                render_line_with_cursor_scroll(line, cursor_col, self.h_scroll, field_width)
            } else {
                let skipped = skip_cols(line, self.h_scroll);
                truncate_to_width(skipped, field_width)
            };
            out.push(format!("{prefix}{pad}{rendered}"));
            let _ = total;
        }
        out
    }
}

fn skip_cols<'a>(line: &'a str, cols: usize) -> &'a str {
    if cols == 0 {
        return line;
    }
    let mut used = 0usize;
    for (i, g) in line.grapheme_indices(true) {
        if used >= cols {
            return &line[i..];
        }
        used += visible_width(g).max(1);
    }
    ""
}

fn render_line_with_cursor_scroll(
    line: &str,
    cursor_col: usize,
    h_scroll: usize,
    field_width: usize,
) -> String {
    let graphemes: Vec<&str> = line.graphemes(true).collect();
    let mut used = 0usize;
    let mut out = String::new();
    let mut placed = false;
    let start = h_scroll.min(graphemes.len());
    for (i, g) in graphemes.iter().enumerate().skip(start) {
        let w = visible_width(g).max(1);
        if used + w > field_width {
            break;
        }
        if i == cursor_col {
            out.push_str("\x1b[7m");
            out.push_str(g);
            out.push_str("\x1b[27m");
            placed = true;
        } else {
            out.push_str(g);
        }
        used += w;
    }
    if !placed && cursor_col >= graphemes.len() && used < field_width {
        out.push_str("\x1b[7m \x1b[27m");
    }
    out
}

fn list_path_completions(cwd: &Path, token: &str) -> Vec<String> {
    let (dir_rel, file_prefix) = match token.rsplit_once('/') {
        Some((dir, file)) => (dir, file),
        None => ("", token),
    };
    let base = if dir_rel.is_empty() {
        cwd.to_path_buf()
    } else {
        cwd.join(dir_rel)
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let prefix_lower = file_prefix.to_ascii_lowercase();
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !file_prefix.starts_with('.') {
                return None;
            }
            if !name.to_ascii_lowercase().starts_with(&prefix_lower) {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let rel = if dir_rel.is_empty() {
                name
            } else {
                format!("{dir_rel}/{name}")
            };
            Some(if is_dir { format!("{rel}/") } else { rel })
        })
        .collect();
    out.sort();
    out.truncate(40);
    out
}

/// Visible slice of a completion list that keeps `selected` on-screen.
fn completion_window(len: usize, selected: usize, max_show: usize) -> (usize, usize) {
    if len == 0 {
        return (0, 0);
    }
    let max_show = max_show.clamp(1, len);
    let selected = selected.min(len - 1);
    let mut start = selected.saturating_sub(max_show.saturating_sub(1));
    if start + max_show > len {
        start = len - max_show;
    }
    (start, max_show)
}

fn completion_mark(index: usize, selected: usize, more_above: bool, more_below: bool) -> &'static str {
    if index == selected {
        ">"
    } else if more_above {
        "↑"
    } else if more_below {
        "↓"
    } else {
        " "
    }
}

impl Component for Editor {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        match self.completion_kind {
            Some(CompletionKind::Slash) => {
                let filtered = self.filtered_slash();
                let selected = self.completion_index.unwrap_or(0);
                let (start, count) =
                    completion_window(filtered.len(), selected, self.popup_max_rows);
                let end = start + count;
                let more_above = start > 0;
                let more_below = end < filtered.len();
                for (i, item) in filtered.iter().enumerate().skip(start).take(count) {
                    let mark = completion_mark(i, selected, i == start && more_above, i + 1 == end && more_below);
                    let row = format!(" {mark} /{}  {}", item.name, item.summary);
                    lines.push(truncate_to_width(&row, width));
                }
            }
            Some(CompletionKind::Path) => {
                let selected = self.completion_index.unwrap_or(0);
                let (start, count) = completion_window(
                    self.path_suggestions.len(),
                    selected,
                    self.popup_max_rows,
                );
                let end = start + count;
                let more_above = start > 0;
                let more_below = end < self.path_suggestions.len();
                for (i, item) in self
                    .path_suggestions
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(count)
                {
                    let mark = completion_mark(i, selected, i == start && more_above, i + 1 == end && more_below);
                    let row = format!(" {mark} @{item}");
                    lines.push(truncate_to_width(&row, width));
                }
            }
            None => {}
        }
        // Mutating h_scroll in render is awkward; approximate without store via clone pattern.
        // Callers that need accurate scroll should call ensure before render via handle_input.
        lines.extend(self.render_editor_body(width));
        lines
    }

    fn handle_input(&mut self, event: &Event) -> InputOutcome {
        let Event::Key(key) = event else {
            return InputOutcome::None;
        };
        if key.kind == KeyEventKind::Release {
            return InputOutcome::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return InputOutcome::Abort;
        }

        let completing = self.completion_kind.is_some();
        let field_width = 80usize.saturating_sub(self.prompt_width()).max(1);

        let outcome = match key.code {
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_str("\n");
                InputOutcome::RequestRender
            }
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.insert_str("\n");
                InputOutcome::RequestRender
            }
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.buffer);
                self.cursor = 0;
                self.h_scroll = 0;
                self.completion_index = None;
                self.completion_kind = None;
                self.path_suggestions.clear();
                if text.trim().is_empty() {
                    self.buffer = text;
                    return InputOutcome::None;
                }
                self.push_history(&text);
                self.history_index = None;
                self.history_draft.clear();
                if let Some(handler) = self.on_submit.as_mut() {
                    handler(text.clone());
                }
                InputOutcome::Submitted(text)
            }
            KeyCode::Tab if completing => {
                if self.apply_completion() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Up if completing => {
                let n = match self.completion_kind {
                    Some(CompletionKind::Slash) => self.filtered_slash().len(),
                    Some(CompletionKind::Path) => self.path_suggestions.len(),
                    None => 0,
                };
                if n > 0 {
                    let cur = self.completion_index.unwrap_or(0);
                    self.completion_index = Some(if cur == 0 { n - 1 } else { cur - 1 });
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Down if completing => {
                let n = match self.completion_kind {
                    Some(CompletionKind::Slash) => self.filtered_slash().len(),
                    Some(CompletionKind::Path) => self.path_suggestions.len(),
                    None => 0,
                };
                if n > 0 {
                    let cur = self.completion_index.unwrap_or(0);
                    self.completion_index = Some((cur + 1) % n);
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Up => {
                let (line, _) = self.cursor_line_col();
                if line > 0 {
                    if self.move_line(-1) {
                        InputOutcome::RequestRender
                    } else {
                        InputOutcome::None
                    }
                } else if self.history_prev() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Down => {
                let (line, _) = self.cursor_line_col();
                let line_count = self.buffer.split('\n').count().max(1);
                if line + 1 < line_count {
                    if self.move_line(1) {
                        InputOutcome::RequestRender
                    } else {
                        InputOutcome::None
                    }
                } else if self.history_next() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Esc if completing => {
                self.completion_index = None;
                self.completion_kind = None;
                self.path_suggestions.clear();
                InputOutcome::RequestRender
            }
            KeyCode::Left => {
                if self.move_left() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Right => {
                if self.move_right() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Home => {
                if self.move_home() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::End => {
                if self.move_end() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.move_home() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.move_end() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.delete_word_backward() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Backspace if key.modifiers.contains(KeyModifiers::ALT) => {
                if self.delete_word_backward() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.kill_to_line_start() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.kill_to_line_end() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Backspace => {
                if self.backspace() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Delete => {
                if self.delete_forward() {
                    InputOutcome::RequestRender
                } else {
                    InputOutcome::None
                }
            }
            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                match clipboard::read_text() {
                    Ok(Some(paste)) => {
                        // Preserve newlines on paste for multiline editing.
                        self.insert_str(&paste.replace('\r', ""));
                        InputOutcome::RequestRender
                    }
                    Ok(None) => InputOutcome::None,
                    Err(_) => InputOutcome::None,
                }
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let mut tmp = [0u8; 4];
                self.insert_str(ch.encode_utf8(&mut tmp));
                InputOutcome::RequestRender
            }
            _ => InputOutcome::None,
        };
        if matches!(outcome, InputOutcome::RequestRender) {
            self.ensure_h_scroll(field_width);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_slash_prefix() {
        let mut ed = Editor::new("> ");
        ed.set_slash_suggestions(vec![
            SlashSuggestion {
                name: "model".into(),
                summary: "switch".into(),
            },
            SlashSuggestion {
                name: "quit".into(),
                summary: "exit".into(),
            },
        ]);
        ed.buffer = "/mo".into();
        ed.cursor = ed.grapheme_len();
        ed.sync_completion();
        let filtered = ed.filtered_slash();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "model");
    }

    #[test]
    fn tab_completes_name() {
        let mut ed = Editor::new("> ");
        ed.set_slash_suggestions(vec![SlashSuggestion {
            name: "hotkeys".into(),
            summary: "help".into(),
        }]);
        ed.buffer = "/hot".into();
        ed.cursor = ed.grapheme_len();
        ed.sync_completion();
        ed.completion_index = Some(0);
        assert!(ed.apply_completion());
        assert_eq!(ed.buffer, "/hotkeys ");
        assert_eq!(ed.cursor, ed.grapheme_len());
    }

    #[test]
    fn insert_and_edit_in_middle() {
        let mut ed = Editor::new("> ");
        ed.insert_str("hello");
        assert_eq!(ed.buffer, "hello");
        assert_eq!(ed.cursor, 5);
        assert!(ed.move_left());
        assert!(ed.move_left());
        ed.insert_str("X");
        assert_eq!(ed.buffer, "helXlo");
        assert_eq!(ed.cursor, 4);
        assert!(ed.backspace());
        assert_eq!(ed.buffer, "hello");
        assert_eq!(ed.cursor, 3);
        assert!(ed.delete_forward());
        assert_eq!(ed.buffer, "helo");
        assert_eq!(ed.cursor, 3);
    }

    #[test]
    fn home_end_motion() {
        let mut ed = Editor::new("> ");
        ed.insert_str("abcd");
        assert!(ed.move_home());
        assert_eq!(ed.cursor, 0);
        assert!(ed.move_end());
        assert_eq!(ed.cursor, 4);
    }

    #[test]
    fn multiline_and_history() {
        let mut ed = Editor::new("> ");
        ed.insert_str("first");
        ed.push_history("first");
        ed.clear();
        ed.insert_str("second");
        ed.push_history("second");
        ed.clear();
        assert!(ed.history_prev());
        assert_eq!(ed.buffer, "second");
        assert!(ed.history_prev());
        assert_eq!(ed.buffer, "first");
        ed.insert_str("\nline2");
        assert!(ed.buffer.contains('\n'));
        assert!(ed.delete_word_backward());
    }

    #[test]
    fn kill_line_keys() {
        let mut ed = Editor::new("> ");
        ed.insert_str("hello world");
        ed.cursor = 6;
        assert!(ed.kill_to_line_end());
        assert_eq!(ed.buffer, "hello ");
        assert!(ed.kill_to_line_start());
        assert_eq!(ed.buffer, "");
    }

    #[test]
    fn completion_window_keeps_selection_visible() {
        assert_eq!(completion_window(0, 0, 6), (0, 0));
        assert_eq!(completion_window(3, 1, 6), (0, 3));
        assert_eq!(completion_window(10, 0, 6), (0, 6));
        assert_eq!(completion_window(10, 9, 6), (4, 6));
        assert_eq!(completion_window(10, 5, 6), (0, 6));
    }

    #[test]
    fn slash_popup_respects_max_rows_and_scrolls() {
        let mut ed = Editor::new("> ");
        ed.set_popup_max_rows(4);
        ed.set_slash_suggestions(
            (0..12)
                .map(|i| SlashSuggestion {
                    name: format!("cmd{i}"),
                    summary: "s".into(),
                })
                .collect(),
        );
        ed.buffer = "/".into();
        ed.cursor = 1;
        ed.sync_completion();
        ed.completion_index = Some(0);
        let lines = ed.render(80);
        let slash_only: Vec<_> = lines.iter().filter(|l| l.contains("/cmd")).collect();
        assert_eq!(slash_only.len(), 4);
        assert!(slash_only[0].contains("/cmd0"));

        ed.completion_index = Some(11);
        let lines = ed.render(80);
        let slash_only: Vec<_> = lines.iter().filter(|l| l.contains("/cmd")).collect();
        assert_eq!(slash_only.len(), 4);
        assert!(slash_only.last().unwrap().contains("/cmd11"));
        assert!(slash_only.iter().any(|l| l.contains('↑')));
    }
}
