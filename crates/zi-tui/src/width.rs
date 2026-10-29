// Visible width helpers (plain text, no ANSI).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Returns the display width of `text` in terminal columns.
pub fn visible_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Truncates `text` to at most `max_width` columns, never splitting a grapheme cluster.
pub fn truncate_to_width(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let mut used = 0usize;
    let mut out = String::new();
    for g in text.graphemes(true) {
        let w = UnicodeWidthStr::width(g);
        if used + w > max_width {
            break;
        }
        used += w;
        out.push_str(g);
    }
    out
}
