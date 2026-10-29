// Terminal setup and frame presentation.

use std::io::stdout;

use crossterm::{
    cursor,
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::component::Component;
use crate::error::TuiError;
use crate::main_screen::MainScreen;

/// Owns terminal mode and differential main-screen updates.
///
/// Mouse capture is on so the wheel can scroll the in-app transcript (alternate
/// screen has no terminal scrollback). Native drag-select is unavailable while
/// captured — use Ctrl+Y / `/copy` for the last reply, or Option+drag on macOS.
pub struct TuiApp {
    main_screen: MainScreen,
    width: u16,
    height: u16,
    active: bool,
}

impl TuiApp {
    /// Enables raw mode, alternate screen, and mouse capture; records terminal size.
    pub fn start() -> Result<Self, TuiError> {
        enable_raw_mode()?;
        let mut out = stdout();
        execute!(
            out,
            EnterAlternateScreen,
            EnableMouseCapture,
            cursor::Hide
        )?;
        let (width, height) = crossterm::terminal::size()?;
        Ok(Self {
            main_screen: MainScreen::new(),
            width,
            height,
            active: true,
        })
    }

    /// Renders a root component and presents the diff.
    pub fn draw<C: Component>(&mut self, component: &C) -> Result<(), TuiError> {
        let lines = component.render(self.width as usize);
        let mut out = stdout();
        self.main_screen
            .present(&mut out, &lines, self.width as usize)?;
        Ok(())
    }

    /// Renders explicit lines (bypasses a component tree).
    pub fn draw_lines(&mut self, lines: &[String]) -> Result<(), TuiError> {
        let mut out = stdout();
        self.main_screen
            .present(&mut out, lines, self.width as usize)?;
        Ok(())
    }

    /// Forces the next draw to clear the alternate screen (avoids ghost rows).
    pub fn invalidate_screen(&mut self) {
        self.main_screen.invalidate();
    }

    /// Polls for the next input event.
    pub fn poll_event(&self) -> Result<Option<event::Event>, TuiError> {
        if event::poll(std::time::Duration::from_millis(50))? {
            Ok(Some(event::read()?))
        } else {
            Ok(None)
        }
    }

    /// Refreshes cached terminal dimensions.
    pub fn refresh_size(&mut self) -> Result<(), TuiError> {
        let (width, height) = crossterm::terminal::size()?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Returns the current terminal width in columns.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Returns the current terminal height in rows.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Full-frame redraw count (for `/tui` stats).
    pub fn full_redraws(&self) -> u64 {
        self.main_screen.full_redraws()
    }

    /// Restores the terminal and clears differential state.
    pub fn stop(mut self) -> Result<(), TuiError> {
        if self.active {
            let mut out = stdout();
            execute!(
                out,
                DisableMouseCapture,
                cursor::Show,
                LeaveAlternateScreen
            )?;
            disable_raw_mode()?;
            self.main_screen.reset();
            self.active = false;
        }
        Ok(())
    }
}

impl Drop for TuiApp {
    fn drop(&mut self) {
        if self.active {
            let _ = (|| -> Result<(), TuiError> {
                let mut out = stdout();
                execute!(
                    out,
                    DisableMouseCapture,
                    cursor::Show,
                    LeaveAlternateScreen
                )?;
                disable_raw_mode()?;
                Ok(())
            })();
            self.active = false;
        }
    }
}
