use std::{
    io::{self, Stdout, stdout},
    panic,
};

use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use thiserror::Error;

pub type AppTerminal = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Error)]
pub enum TerminalError {
    #[error("Failed to configure the terminal: {0}")]
    Io(#[from] io::Error),
}

pub struct TerminalSession {
    terminal: AppTerminal,
    active: bool,
}

impl TerminalSession {
    pub fn enter() -> Result<Self, TerminalError> {
        enable_raw_mode()?;
        let mut output = stdout();
        if let Err(error) = execute!(output, EnterAlternateScreen, Hide, EnableMouseCapture) {
            leave_alternate_screen(&mut output);
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let terminal = match Terminal::new(CrosstermBackend::new(output)) {
            Ok(terminal) => terminal,
            Err(error) => {
                leave_alternate_screen(&mut stdout());
                let _ = disable_raw_mode();
                return Err(error.into());
            }
        };
        Ok(Self {
            terminal,
            active: true,
        })
    }

    pub fn terminal_mut(&mut self) -> &mut AppTerminal {
        &mut self.terminal
    }

    pub fn restore(&mut self) {
        if self.active {
            let _ = self.terminal.show_cursor();
            leave_alternate_screen(self.terminal.backend_mut());
            let _ = disable_raw_mode();
            self.active = false;
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore();
    }
}

fn leave_alternate_screen(output: &mut impl io::Write) {
    let _ = execute!(output, DisableMouseCapture, LeaveAlternateScreen, Show);
}

pub fn install_panic_hook() {
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        leave_alternate_screen(&mut stdout());
        original(info);
    }));
}
