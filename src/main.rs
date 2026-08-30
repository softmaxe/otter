use std::{
    io::{self, IsTerminal},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyEventKind};
use otter::{
    app::{App, UiCommand},
    terminal::{TerminalSession, install_panic_hook},
    toolchain::Toolchain,
    ui,
};

fn main() -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("otter requires an interactive terminal.");
    }

    let toolchain = Toolchain::discover().context("FFmpeg tool discovery failed")?;
    install_panic_hook();
    let mut terminal = TerminalSession::enter()?;
    let mut app = App::new(toolchain);

    loop {
        app.poll_background();
        let area = terminal
            .terminal_mut()
            .draw(|frame| ui::render(frame, &app))?
            .area;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let command = match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
            Event::Mouse(mouse) => ui::handle_mouse(&mut app, mouse, area),
            _ => continue,
        };

        match command {
            UiCommand::None => {}
            UiCommand::OpenInputs { add } => app.open_inputs_picker(add),
            UiCommand::OpenOutput => app.open_output_picker(),
            UiCommand::Quit => break,
        }
    }

    terminal.restore();
    Ok(())
}
