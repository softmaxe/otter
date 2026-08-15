use std::{
    env,
    ffi::OsStr,
    io::{self, IsTerminal},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyEventKind};
use ffmpeg_tui::{
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
        bail!("FFmpeg TUI requires an interactive terminal.");
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
            UiCommand::OpenInput => {
                match dialog::prompt(&DialogRequest::OpenInput) {
                    Ok(Some(path)) => app.select_input(path),
                    Ok(None) => {}
                    Err(error) => app.report_error(error.to_string()),
                }
                discard_pending_input()?;
            }
            UiCommand::OpenOutput => {
                let output = app.draft.output.as_ref();
                let request = DialogRequest::SaveOutput {
                    directory: output.and_then(|path| path.parent()).map(ToOwned::to_owned),
                    file_name: output
                        .and_then(|path| path.file_name())
                        .and_then(OsStr::to_str)
                        .map(ToOwned::to_owned),
                };
                match dialog::prompt(&request) {
                    Ok(Some(path)) => app.select_output(path),
                    Ok(None) => {}
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
