// Differential main-screen rendering (aligned with pi TuiMainScreen).

use std::io::Write;

const SYNC_BEGIN: &str = "\x1b[?2026h";
const SYNC_END: &str = "\x1b[?2026l";

/// Tracks the last frame and emits minimal cursor moves and line clears.
pub struct MainScreen {
    previous_lines: Vec<String>,
    previous_width: usize,
    hardware_cursor_row: usize,
    /// When set, the next present clears the alternate screen first.
    force_clear: bool,
    /// Count of full-screen redraws (parity with pi `tui.fullRedraws`).
    full_redraws: u64,
}

impl MainScreen {
    /// Creates an empty render state (first draw assumes a clean screen).
    pub fn new() -> Self {
        Self {
            previous_lines: Vec::new(),
            previous_width: 0,
            hardware_cursor_row: 0,
            force_clear: false,
            full_redraws: 0,
        }
    }

    /// Number of full clears / full-frame redraws since start.
    pub fn full_redraws(&self) -> u64 {
        self.full_redraws
    }

    /// Writes `new_lines` to `out`, updating only changed rows when possible.
    pub fn present<W: Write>(
        &mut self,
        out: &mut W,
        new_lines: &[String],
        width: usize,
    ) -> std::io::Result<()> {
        let width_changed = self.previous_width != 0 && self.previous_width != width;
        let height_changed = !self.previous_lines.is_empty()
            && self.previous_lines.len() != new_lines.len();
        let force_clear = self.force_clear;
        self.force_clear = false;

        if force_clear || width_changed || height_changed {
            self.full_render(out, new_lines, true)?;
            self.previous_width = width;
            return Ok(());
        }
        if self.previous_lines.is_empty() {
            // First draw after EnterAlternateScreen: screen is already blank.
            self.full_render(out, new_lines, false)?;
            self.previous_width = width;
            return Ok(());
        }

        let (first, last) = changed_range(&self.previous_lines, new_lines);
        if first.is_none() {
            self.previous_width = width;
            return Ok(());
        }
        let first = first.unwrap();
        let last = last.unwrap().min(new_lines.len().saturating_sub(1));
        let changed = last - first + 1;

        // Slash popup open/close, transcript clears, and viewport scrolls shift
        // many rows while keeping frame height fixed. Line-index diffs then
        // leave ghost prompts / duplicated paragraphs; clear when the shift is
        // large or looks like a scrolled window.
        if first == 0
            || changed * 3 >= new_lines.len().max(1) * 2
            || is_viewport_shift(&self.previous_lines, new_lines)
        {
            self.full_render(out, new_lines, true)?;
            self.previous_width = width;
            return Ok(());
        }

        write!(out, "{SYNC_BEGIN}")?;
        self.move_cursor_to_row(out, first)?;
        for i in first..=last {
            if i > first {
                write!(out, "\r\n")?;
                self.hardware_cursor_row += 1;
            } else {
                write!(out, "\r")?;
            }
            // Clear then rewrite so shorter lines do not leave trailing glyphs.
            write!(out, "\x1b[2K{}", new_lines[i])?;
        }

        // Track the row we actually ended on (not the frame bottom). A wrong
        // baseline makes the next present write into the middle of the screen.
        self.hardware_cursor_row = last;
        write!(out, "{SYNC_END}")?;
        self.previous_lines = new_lines.to_vec();
        self.previous_width = width;
        out.flush()?;
        Ok(())
    }

    /// Forces the next present to clear the screen (avoids ghost rows after layout shifts).
    pub fn invalidate(&mut self) {
        self.force_clear = true;
        self.previous_lines.clear();
        self.previous_width = 0;
        self.hardware_cursor_row = 0;
    }

    fn full_render<W: Write>(
        &mut self,
        out: &mut W,
        new_lines: &[String],
        clear: bool,
    ) -> std::io::Result<()> {
        self.full_redraws = self.full_redraws.saturating_add(1);
        write!(out, "{SYNC_BEGIN}")?;
        if clear {
            write!(out, "\x1b[2J\x1b[H")?;
            self.hardware_cursor_row = 0;
        }
        for (i, line) in new_lines.iter().enumerate() {
            if i > 0 {
                write!(out, "\r\n")?;
            }
            // Clear each row so a shorter frame cannot leave prior-cell residue
            // when the terminal does not wipe on newline.
            write!(out, "\x1b[2K{line}")?;
        }
        write!(out, "{SYNC_END}")?;
        self.previous_lines = new_lines.to_vec();
        self.hardware_cursor_row = new_lines.len().saturating_sub(1);
        out.flush()?;
        Ok(())
    }

    fn move_cursor_to_row<W: Write>(
        &mut self,
        out: &mut W,
        target_row: usize,
    ) -> std::io::Result<()> {
        if target_row > self.hardware_cursor_row {
            let diff = target_row - self.hardware_cursor_row;
            write!(out, "\x1b[{diff}B")?;
        } else if target_row < self.hardware_cursor_row {
            let diff = self.hardware_cursor_row - target_row;
            write!(out, "\x1b[{diff}A")?;
        }
        self.hardware_cursor_row = target_row;
        Ok(())
    }

    /// Clears render state after leaving raw mode (does not restore the terminal).
    pub fn reset(&mut self) {
        self.previous_lines.clear();
        self.previous_width = 0;
        self.hardware_cursor_row = 0;
        self.force_clear = false;
        // Keep full_redraws across reset so /tui stays meaningful in-session.
    }
}

impl Default for MainScreen {
    fn default() -> Self {
        Self::new()
    }
}

fn changed_range(old: &[String], new: &[String]) -> (Option<usize>, Option<usize>) {
    let max = old.len().max(new.len());
    let mut first: Option<usize> = None;
    let mut last: Option<usize> = None;
    for i in 0..max {
        let o = old.get(i).map(String::as_str).unwrap_or("");
        let n = new.get(i).map(String::as_str).unwrap_or("");
        if o != n {
            if first.is_none() {
                first = Some(i);
            }
            last = Some(i);
        }
    }
    (first, last)
}

/// True when `new` looks like `old` scrolled by a small offset (same rows, new indices).
///
/// Index diffs then rewrite the wrong rows and leave the old paragraph visible —
/// the "duplicate after scroll" glitch.
fn is_viewport_shift(old: &[String], new: &[String]) -> bool {
    if old.len() != new.len() || old.len() < 4 {
        return false;
    }
    // Ignore a short sticky footer (status + editor) when probing the body.
    let body = old.len().saturating_sub(2).max(1);
    let old_body = &old[..body];
    let new_body = &new[..body];
    for shift in 1..=3 {
        if body_shifted_by(old_body, new_body, shift)
            || body_shifted_by(new_body, old_body, shift)
        {
            return true;
        }
    }
    false
}

fn body_shifted_by(prev: &[String], next: &[String], shift: usize) -> bool {
    if shift == 0 || shift >= prev.len() {
        return false;
    }
    let overlap = prev.len() - shift;
    if overlap == 0 {
        return false;
    }
    let mut matches = 0usize;
    for i in 0..overlap {
        if prev[i + shift] == next[i] {
            matches += 1;
        }
    }
    // Strong overlap ⇒ content moved as a block rather than edited in place.
    matches * 4 >= overlap * 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_frame_full_clears() {
        let mut screen = MainScreen::new();
        let mut out = Vec::new();
        screen
            .present(
                &mut out,
                &["a".into(), "b".into(), "c".into()],
                80,
            )
            .unwrap();
        out.clear();
        screen
            .present(&mut out, &["a".into(), "c".into()], 80)
            .unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("\x1b[2J"),
            "expected full clear on height change, got {s:?}"
        );
    }

    #[test]
    fn same_height_rewrites_with_line_clear() {
        let mut screen = MainScreen::new();
        let mut out = Vec::new();
        screen
            .present(&mut out, &["hello world".into()], 80)
            .unwrap();
        out.clear();
        screen.present(&mut out, &["hi".into()], 80).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(s.contains("\x1b[2K"), "expected line clear, got {s:?}");
        assert!(s.contains("hi"), "expected new text, got {s:?}");
    }

    #[test]
    fn large_shift_full_clears_to_avoid_ghosts() {
        let mut screen = MainScreen::new();
        let mut out = Vec::new();
        let tall: Vec<String> = (0..10).map(|i| format!("row-{i}")).collect();
        screen.present(&mut out, &tall, 80).unwrap();
        out.clear();
        let short: Vec<String> = std::iter::once("only".into())
            .chain(std::iter::repeat_with(String::new).take(8))
            .chain(std::iter::once("> ".into()))
            .collect();
        screen.present(&mut out, &short, 80).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("\x1b[2J"),
            "expected full clear on large content shift, got {s:?}"
        );
    }

    #[test]
    fn viewport_scroll_shift_full_clears() {
        let mut screen = MainScreen::new();
        let mut out = Vec::new();
        // Body rows 0..7 plus sticky status/editor footer.
        let frame1: Vec<String> = (0..8)
            .map(|i| format!("row-{i}"))
            .chain([String::from("── status"), String::from("> ")])
            .collect();
        screen.present(&mut out, &frame1, 80).unwrap();
        out.clear();
        // Scroll up by 1: body shows row-1..row-8, footer unchanged.
        let frame2: Vec<String> = (1..9)
            .map(|i| format!("row-{i}"))
            .chain([String::from("── status"), String::from("> ")])
            .collect();
        // Make first body line match so first==0 heuristic alone may not fire
        // if we only changed from index 1 — use a case where row-0 equals by luck.
        // Here row-0 changes (row-0 → row-1); still detect via shift overlap.
        screen.present(&mut out, &frame2, 80).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("\x1b[2J"),
            "expected full clear on viewport scroll shift, got {s:?}"
        );
    }

    #[test]
    fn detects_body_shifted_by_one() {
        let prev: Vec<String> = (0..6).map(|i| format!("row-{i}")).collect();
        let next: Vec<String> = (1..7).map(|i| format!("row-{i}")).collect();
        assert!(body_shifted_by(&prev, &next, 1));
        assert!(is_viewport_shift(
            &[prev.clone(), vec!["status".into(), "> ".into()]].concat(),
            &[next, vec!["status".into(), "> ".into()]].concat()
        ));
    }
}
