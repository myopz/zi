// Multi-line plain text with wrapping.

use crate::component::Component;
use crate::wrap::wrap_text;

/// Displays wrapped plain text with optional horizontal padding.
pub struct Text {
    text: String,
    padding_x: usize,
}

impl Text {
    /// Creates text with the given content and left/right padding in columns.
    pub fn new(text: impl Into<String>, padding_x: usize) -> Self {
        Self {
            text: text.into(),
            padding_x,
        }
    }

    /// Replaces the displayed text and invalidates any render cache.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        if self.text.trim().is_empty() {
            return Vec::new();
        }
        let padding_x = self.padding_x.min(width.saturating_sub(1) / 2);
        let content_width = width.saturating_sub(padding_x * 2).max(1);
        let pad = " ".repeat(padding_x);
        wrap_text(&self.text, content_width)
            .into_iter()
            .map(|line| format!("{pad}{line}"))
            .collect()
    }
}
