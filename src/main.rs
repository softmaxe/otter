use std::{
    env,
    ffi::OsStr,
    io::{self, IsTerminal},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyEventKind};
use fftui::{
    app::{App, UiCommand},
    dialog::{self, CHILD_FLAG, DialogRequest},
    terminal::{TerminalSession, install_panic_hook},
    toolchain::Toolchain,
    ui,
};

fn main() -> Result<()> {
    // The dialog helper mode runs before any terminal check: it is spawned with
    // pipes rather than a tty, and it must never touch the TUI machinery.
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args
        .first()
        .is_some_and(|arg| arg.as_os_str() == OsStr::new(CHILD_FLAG))
    {
        let request = DialogRequest::parse_args(args)?;
        return dialog::run_child(&request).context("File dialog failed");
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("fftui requires an interactive terminal.");
    }

    let toolchain = Toolchain::discover().context("FFmpeg tool discovery failed")?;
    install_panic_hook();
    let mut terminal = TerminalSession::enter()?;
    let mut app = App::new(toolchain);

    loop {
        app.poll_background();
        terminal
            .terminal_mut()
            .draw(|frame| ui::render(frame, &app))?;

        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match app.handle_key(key) {
            UiCommand::None => {}
            UiCommand::OpenInputs { add } => {
                match dialog::prompt(&DialogRequest::OpenInputs) {
                    // An empty selection means the panel was dismissed.
                    Ok(paths) if paths.is_empty() => {}
                    Ok(paths) if add => app.add_inputs(paths),
                    Ok(paths) => app.select_inputs(paths),
                    Err(error) => app.report_error(error.to_string()),
                }
                discard_pending_input()?;
            }
            UiCommand::OpenOutput => {
                match dialog::prompt(&app.output_dialog_request()) {
                    Ok(paths) => {
                        if let Some(path) = paths.into_iter().next() {
                            app.select_output(path);
                        }
                    }
                    Err(error) => app.report_error(error.to_string()),
                }
                discard_pending_input()?;
            }
            UiCommand::Quit => break,
        }
    }

    terminal.restore();
    Ok(())
}

/// Keys pressed while the GUI dialog was in front stay queued on the terminal, so
/// drop them instead of replaying them as settings changes.
fn discard_pending_input() -> Result<()> {
    while event::poll(Duration::ZERO)? {
        let _ = event::read()?;
    }
    Ok(())
}
