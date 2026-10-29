// Component trait for layout and input.

use crossterm::event::Event;

/// Result of feeding a terminal event to a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputOutcome {
    /// No state change requiring a redraw.
    None,
    /// Component state changed; caller should re-render.
    RequestRender,
    /// Editor submitted its buffer.
    Submitted(String),
    /// User requested abort (e.g. Ctrl+C).
    Abort,
}

/// A renderable TUI building block.
pub trait Component {
    /// Renders this component into terminal lines for the given width.
    fn render(&self, width: usize) -> Vec<String>;

    /// Handles a terminal input event when this component has focus.
    fn handle_input(&mut self, event: &Event) -> InputOutcome {
        let _ = event;
        InputOutcome::None
    }
}
