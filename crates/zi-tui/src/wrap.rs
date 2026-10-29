// Word-aware text wrapping by visible width.

use crate::width::{truncate_to_width, visible_width};

/// Wraps plain text into lines that fit within `width` columns.
pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        wrap_paragraph(paragraph, width, &mut lines);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_paragraph(paragraph: &str, width: usize, lines: &mut Vec<String>) {
    let mut line = String::new();
    let mut line_width = 0usize;
    for word in paragraph.split_whitespace() {
        let word_width = visible_width(word);
        if word_width > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            flush_long_word(word, width, lines);
            continue;
        }
        let extra = if line.is_empty() { 0 } else { 1 };
        if line_width + extra + word_width > width {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        if !line.is_empty() {
            line.push(' ');
            line_width += 1;
        }
        line.push_str(word);
        line_width += word_width;
    }
    if !line.is_empty() {
        lines.push(line);
    }
}

fn flush_long_word(word: &str, width: usize, lines: &mut Vec<String>) {
    let mut rest = word;
    while !rest.is_empty() {
        let chunk = truncate_to_width(rest, width);
        if chunk.is_empty() {
            lines.push(String::new());
            break;
        }
        let byte_len = chunk.len();
        lines.push(chunk);
        if byte_len >= rest.len() {
            break;
        }
        rest = &rest[byte_len..];
    }
}
