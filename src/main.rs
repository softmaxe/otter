use std::{
    io::{self, IsTerminal},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyEventKind};
use ffmpeg_tui::{
    app::{App, UiCommand},
    terminal::{TerminalSession, install_panic_hook},
    toolchain::Toolchain,
    ui,
};
use rfd::FileDialog;

fn main() -> Result<()> {
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
                terminal.suspend_for_dialog()?;
                let selected = FileDialog::new()
                    .set_title("Choose input media")
                    .add_filter(
                        "Media files",
                        &["mp4", "mkv", "webm", "mov", "avi", "m4v", "ts"],
                    )
                    .pick_file();
                terminal.resume_after_dialog()?;
                if let Some(path) = selected {
                    app.select_input(path);
                }
            }
            UiCommand::OpenOutput => {
                terminal.suspend_for_dialog()?;
                let mut dialog = FileDialog::new().set_title("Choose output file");
                if let Some(output) = app.draft.output.as_ref() {
                    if let Some(parent) = output.parent() {
                        dialog = dialog.set_directory(parent);
                    }
                    if let Some(name) = output.file_name().and_then(|name| name.to_str()) {
                        dialog = dialog.set_file_name(name);
                    }
                }
                let selected = dialog.save_file();
                terminal.resume_after_dialog()?;
                if let Some(path) = selected {
                    app.select_output(path);
                }
            }
            UiCommand::Quit => break,
        }
    }

    terminal.restore();
    Ok(())
}
