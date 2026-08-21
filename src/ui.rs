use std::{path::PathBuf, time::Duration};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap},
};

use crate::{
    app::{App, ConfigField, JobOutcome, JobRecord, JobState, Screen},
    domain::{EstimateBasis, InputMedia, OutputTarget, RateControlMode, SizeEstimate, file_label},
};

/// Rows the layout owes the panels around the source list, so a long selection grows
/// into spare terminal height instead of squeezing the workspace out of the screen.
const RESERVED_ROWS: u16 = 24;

// Catppuccin Mocha palette.
// See https://catppuccin.com/palette for the reference swatches.
const BG: Color = Color::Rgb(17, 17, 27); // crust  #11111b
const PANEL: Color = Color::Rgb(30, 30, 46); // base   #1e1e2e
const BORDER: Color = Color::Rgb(69, 71, 90); // surface1 #45475a
const TEXT: Color = Color::Rgb(205, 214, 244); // text   #cdd6f4
const MUTED: Color = Color::Rgb(127, 132, 156); // overlay1 #7f849c
const ACCENT: Color = Color::Rgb(148, 226, 213); // teal   #94e2d5
const SECONDARY: Color = Color::Rgb(137, 180, 250); // blue   #89b4fa
const WARNING: Color = Color::Rgb(249, 226, 175); // yellow #f9e2af
const ERROR: Color = Color::Rgb(243, 139, 168); // red    #f38ba8

pub fn render(frame: &mut Frame<'_>, app: &App) {
    frame.render_widget(
        Block::default().style(Style::default().bg(BG)),
        frame.area(),
    );
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(source_panel_height(app, area)),
        Constraint::Length(12),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(frame, app, chunks[0]);
    render_source(frame, app, chunks[1]);
    render_settings(frame, app, chunks[2]);
    render_workspace(frame, app, chunks[3]);
    render_footer(frame, app, chunks[4]);

    if app.help_visible {
        render_help(frame, area);
    }
    if app.cancel_confirmation {
        render_cancel_confirmation(frame, area);
    }
    if let Some((value, field)) = app.numeric_edit_value() {
        render_numeric_edit(frame, area, value, field);
    }
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let ffmpeg_version = short_version(&app.toolchain.ffmpeg_version);
    let title = Line::from(vec![
        Span::styled(
            " FFTUI ",
            Style::default()
                .fg(BG)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  Batch transcoder", Style::default().fg(TEXT)),
        Span::styled(format!("  •  {ffmpeg_version}"), Style::default().fg(MUTED)),
    ]);
    frame.render_widget(
        Paragraph::new(title)
            .block(panel_block("READY"))
            .alignment(Alignment::Left),
        area,
    );
}

/// How tall the source panel wants to be: two rows of borders plus one row per line
/// it would draw, bounded by the height the panels below it are owed.
fn source_panel_height(app: &App, area: Rect) -> u16 {
    let content = match app.draft.inputs.len() {
        0 | 1 => 2,
        // A header row plus one row per file. When they do not all fit, the last row
        // becomes the count of the ones that were left out.
        count => 1 + u16::try_from(count).unwrap_or(u16::MAX),
    };
    let slack = area.height.saturating_sub(RESERVED_ROWS);
    (content + 2).clamp(4, 4 + slack).min(12)
}

fn render_source(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let lines = match app.draft.inputs.as_slice() {
        [] | [_] => single_source_lines(app),
        // The panel's borders take one row each; the rest is content.
        inputs => queue_source_lines(app, inputs, area.height.saturating_sub(2) as usize),
    };
    frame.render_widget(Paragraph::new(lines).block(panel_block("SOURCE")), area);
}

fn single_source_lines<'a>(app: &App) -> Vec<Line<'a>> {
    let path = app
        .draft
        .single_input()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "No input selected — press i".to_owned());
    let mut lines = vec![Line::from(vec![
        Span::styled("File  ", Style::default().fg(MUTED)),
        Span::styled(path, Style::default().fg(TEXT)),
    ])];
    if let Some(media) = app.single_media() {
        lines.push(Line::from(vec![
            Span::styled("Media ", Style::default().fg(MUTED)),
            Span::styled(media_summary(media), Style::default().fg(SECONDARY)),
        ]));
    } else if let Some(input) = app.draft.single_input()
        && let Some(error) = app.probe_error_for(input)
    {
        lines.push(Line::styled(error.to_owned(), Style::default().fg(ERROR)));
    } else {
        lines.push(Line::styled(
            match app.job {
                JobState::Probing => "Reading streams and metadata…",
                _ => "Choose one or more local media files to inspect their streams.",
            },
            Style::default().fg(MUTED),
        ));
    }
    lines
}

fn queue_source_lines<'a>(app: &App, inputs: &[PathBuf], rows: usize) -> Vec<Line<'a>> {
    let ready = app.probed_count();
    let failed = app.failed_probe_count();
    let mut summary = format!("{} files selected  •  {ready} ready", inputs.len());
    if failed > 0 {
        summary.push_str(&format!("  •  {failed} unreadable"));
    }
    let pending = inputs.len() - ready - failed;
    if pending > 0 {
        summary.push_str(&format!("  •  {pending} reading"));
    }
    let mut lines = vec![Line::from(vec![
        Span::styled("Queue ", Style::default().fg(MUTED)),
        Span::styled(summary, Style::default().fg(TEXT)),
    ])];

    // Keep the last row for the count of files that did not fit.
    let visible = if inputs.len() > rows.saturating_sub(1) {
        rows.saturating_sub(2)
    } else {
        inputs.len()
    };
    for input in inputs.iter().take(visible) {
        let (marker, detail, style) = match (app.media_for(input), app.probe_error_for(input)) {
            (Some(media), _) => ("✓", media_summary(media), Style::default().fg(SECONDARY)),
            (None, Some(error)) => ("✗", error.to_owned(), Style::default().fg(ERROR)),
            (None, None) => ("·", "reading…".to_owned(), Style::default().fg(MUTED)),
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), style),
            Span::styled(
                fixed_width(&file_label(input), 28),
                Style::default().fg(TEXT),
            ),
            Span::styled(detail, style),
        ]));
    }
    if visible < inputs.len() {
        lines.push(Line::styled(
            format!("  … {} more", inputs.len() - visible),
            Style::default().fg(MUTED),
        ));
    }
    lines
}

fn media_summary(media: &InputMedia) -> String {
    let duration = media
        .duration
        .map(format_duration)
        .unwrap_or_else(|| "Unknown".to_owned());
    let audio = media.audio.as_deref().unwrap_or("None");
    format!(
        "{}×{}  •  {}  •  video {}  •  audio {}",
        media.video.width, media.video.height, duration, media.video.codec, audio
    )
}

/// Fits a file name into a fixed column, measured in terminal cells rather than
/// characters so that a name in a wide script does not push the column out of line.
fn fixed_width(value: &str, width: usize) -> String {
    let mut kept = String::new();
    let mut used = value.width();
    if used > width {
        used = 0;
        for character in value.chars() {
            let cell = character.width().unwrap_or(0);
            // Leave the last cell for the ellipsis this trim adds.
            if used + cell > width.saturating_sub(1) {
                break;
            }
            kept.push(character);
            used += cell;
        }
        kept.push('…');
        used += 1;
    } else {
        kept.push_str(value);
    }
    kept.push_str(&" ".repeat(width.saturating_sub(used)));
    kept
}

fn render_settings(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let output = match app.draft.output.as_ref() {
        Some(OutputTarget::File(path)) => path.display().to_string(),
        Some(OutputTarget::Directory(path)) => format!("{}/  (one file each)", path.display()),
        None => "Not selected".to_owned(),
    };
    let rate_value = match app.draft.rate_control_mode {
        RateControlMode::Quality => app.quality_label(),
        RateControlMode::Bitrate => format!("{} kbps", app.draft.video_bitrate_kbps),
    };
    let audio_bitrate_enabled = app.audio_bitrate_enabled();
    let input_value = match app.draft.inputs.len() {
        0 => "Open file dialog".to_owned(),
        1 => "1 file  •  i replace   a add".to_owned(),
        count => format!("{count} files  •  i replace   a add   c clear"),
    };
    let rows = [
        (ConfigField::Input, "Input", input_value, true),
        (ConfigField::Output, "Output", output, true),
        (
            ConfigField::Container,
            "Container",
            app.draft.container.to_string(),
            true,
        ),
        (
            ConfigField::VideoCodec,
            "Video codec",
            app.draft.video_codec.to_string(),
            true,
        ),
        (
            ConfigField::AudioCodec,
            "Audio codec",
            if app.all_sources_silent() {
                "None (no source has audio)".to_owned()
            } else {
                app.draft.audio_codec.to_string()
            },
            !app.all_sources_silent(),
        ),
        (
            ConfigField::Resolution,
            "Resolution",
            app.draft.resolution.to_string(),
            true,
        ),
        (
            ConfigField::RateControl,
            "Rate control",
            app.draft.rate_control_mode.to_string(),
            true,
        ),
        (
            ConfigField::RateValue,
            if app.draft.rate_control_mode == RateControlMode::Quality {
                "Quality"
            } else {
                "Video bitrate"
            },
            rate_value,
            true,
        ),
        (
            ConfigField::AudioBitrate,
            "Audio bitrate",
            if audio_bitrate_enabled {
                format!("{} kbps", app.draft.audio_bitrate_kbps)
            } else {
                "Disabled".to_owned()
            },
            audio_bitrate_enabled,
        ),
    ];
    let mut lines = rows
        .into_iter()
        .map(|(field, label, value, enabled)| setting_line(app, field, label, value, enabled))
        .collect::<Vec<_>>();
    lines.push(estimate_line(app));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("SETTINGS"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// The predicted output size. Read-only, so it sits outside the focus ring and skips
/// the selection marker the editable rows above it use.
fn estimate_line<'a>(app: &App) -> Line<'a> {
    let (value, style) = match app.size_estimate() {
        Some(estimate) => (
            estimate.label(),
            Style::default().fg(estimate_color(estimate)),
        ),
        None => (
            estimate_placeholder(app).to_owned(),
            Style::default().fg(MUTED),
        ),
    };
    Line::from(vec![
        Span::styled(format!("  {:<16}", "Est. size"), Style::default().fg(MUTED)),
        Span::styled(format!(" {value} "), style),
    ])
}

/// Targeted estimates get the same weight as a real setting; heuristic ones are muted
/// toward the advisory palette so a rough number never reads as a measured one.
fn estimate_color(estimate: SizeEstimate) -> Color {
    match estimate.basis {
        EstimateBasis::Targeted => SECONDARY,
        EstimateBasis::Heuristic => MUTED,
    }
}

fn estimate_placeholder(app: &App) -> &'static str {
    // Every path through the estimate multiplies by duration, so without one there is
    // nothing to show but the reason.
    if app.probed_count() > 0 {
        "Unknown (source has no duration)"
    } else {
        "Awaiting source"
    }
}

fn setting_line<'a>(
    app: &App,
    field: ConfigField,
    label: &'a str,
    value: String,
    enabled: bool,
) -> Line<'a> {
    let focused = app.screen == Screen::Configure && app.focus == field;
    let marker = if focused { "›" } else { " " };
    let value_style = if !enabled {
        Style::default().fg(MUTED)
    } else if focused {
        Style::default()
            .fg(BG)
            .bg(ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(TEXT)
    };
    Line::from(vec![
        Span::styled(
            format!("{marker} {label:<16}"),
            Style::default().fg(if focused { ACCENT } else { MUTED }),
        ),
        Span::styled(format!(" {value} "), value_style),
    ])
}

fn render_workspace(frame: &mut Frame<'_>, app: &App, area: Rect) {
    match app.screen {
        Screen::Configure => render_configure_status(frame, app, area),
        Screen::Confirm => render_confirmation(frame, app, area),
        Screen::Running => render_progress(frame, app, area),
        Screen::Result => render_result(frame, app, area),
        Screen::Error => render_error(frame, app, area),
    }
}

fn render_configure_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let validation = app.current_validation_error();
    let (title, style, message) = if let Some(error) = validation {
        ("NEEDS ATTENTION", Style::default().fg(WARNING), error)
    } else {
        (
            "STATUS",
            Style::default().fg(ACCENT),
            app.status_message
                .clone()
                .unwrap_or_else(|| "Ready.".to_owned()),
        )
    };
    let body = vec![
        Line::styled(message, style),
        Line::default(),
        Line::styled(
            "Press Enter to build and review the safe FFmpeg command before running it.",
            Style::default().fg(MUTED),
        ),
    ];
    frame.render_widget(
        Paragraph::new(body)
            .block(panel_block(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_confirmation(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let preview = app
        .command_preview
        .as_deref()
        .unwrap_or("Command preview unavailable.");
    let mut text = Text::from(vec![
        Line::styled(
            "Review the exact program and arguments. Output is written to an app-owned temporary file first.",
            Style::default().fg(WARNING),
        ),
        Line::default(),
        Line::styled(preview, Style::default().fg(TEXT)),
    ]);
    if app.queue.len() > 1 {
        text.push_line(Line::default());
        text.push_line(Line::styled(
            format!(
                "The same settings run for all {} files, one after another:",
                app.queue.len()
            ),
            Style::default().fg(MUTED),
        ));
        for record in app.queue.iter().take(6) {
            text.push_line(Line::styled(
                format!(
                    "  {} → {}",
                    file_label(&record.input),
                    file_label(&record.output)
                ),
                Style::default().fg(SECONDARY),
            ));
        }
        if app.queue.len() > 6 {
            text.push_line(Line::styled(
                format!("  … {} more", app.queue.len() - 6),
                Style::default().fg(MUTED),
            ));
        }
    }
    if let Some(estimate) = app.size_estimate() {
        text.push_line(Line::default());
        text.push_line(Line::from(vec![
            Span::styled(
                if app.queue.len() > 1 {
                    "Estimated total output size  "
                } else {
                    "Estimated output size  "
                },
                Style::default().fg(MUTED),
            ),
            Span::styled(
                estimate.label(),
                Style::default().fg(estimate_color(estimate)),
            ),
        ]));
    }
    frame.render_widget(
        Paragraph::new(text)
            .block(panel_block("CONFIRM COMMAND"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_progress(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let sections = Layout::vertical([Constraint::Length(3), Constraint::Min(2)]).split(area);
    let (ratio, label) = match &app.job {
        JobState::Running {
            progress: Some(progress),
            ..
        } => {
            let percent = progress.percent.unwrap_or(0.0);
            let speed = progress.speed.as_deref().unwrap_or("—");
            (
                percent / 100.0,
                format!(
                    "{:>5.1}%  •  {} processed  •  {speed}",
                    percent,
                    format_duration(progress.processed)
                ),
            )
        }
        JobState::Cancelling => (0.0, "Cancelling FFmpeg safely…".to_owned()),
        _ => (0.0, "Starting FFmpeg…".to_owned()),
    };
    let title = match (&app.job, app.queue.len()) {
        (_, 0 | 1) => "PROGRESS".to_owned(),
        (JobState::Running { index, .. }, total) => format!(
            "PROGRESS — FILE {} OF {total}: {}",
            index + 1,
            app.queue
                .get(*index)
                .map(|record| file_label(&record.input))
                .unwrap_or_default()
                .to_uppercase()
        ),
        (_, total) => format!("PROGRESS — {} OF {total} DONE", app.succeeded_count()),
    };
    frame.render_widget(
        Gauge::default()
            .block(panel_block(&title))
            .gauge_style(Style::default().fg(ACCENT).bg(PANEL))
            .ratio(ratio.clamp(0.0, 1.0))
            .label(label),
        sections[0],
    );
    let warnings = if app.stderr_tail.is_empty() {
        Text::from(Line::styled(
            "No FFmpeg warnings.",
            Style::default().fg(MUTED),
        ))
    } else {
        Text::from(
            app.stderr_tail
                .iter()
                .rev()
                .take(sections[1].height.saturating_sub(2) as usize)
                .rev()
                .map(|line| Line::styled(line.clone(), Style::default().fg(WARNING)))
                .collect::<Vec<_>>(),
        )
    };
    frame.render_widget(
        Paragraph::new(warnings)
            .block(panel_block("FFMPEG MESSAGES"))
            .wrap(Wrap { trim: false }),
        sections[1],
    );
}

fn render_result(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let (elapsed, cancelled) = match &app.job {
        JobState::Finished { elapsed, cancelled } => (*elapsed, *cancelled),
        _ => (Duration::ZERO, false),
    };
    // One file keeps the plain-language result it has always had; a queue reports a
    // count and then names every file, because a summary alone hides which failed.
    let (title, color, mut lines) = match app.queue.as_slice() {
        // The file's own outcome decides, not how the run ended: a cancellation that
        // arrived after the last frame was written still produced the file.
        [record] => match &record.outcome {
            JobOutcome::Succeeded { elapsed } => (
                "CONVERSION COMPLETE",
                ACCENT,
                vec![
                    Line::raw(format!("Output: {}", record.output.display())),
                    Line::raw(format!("Elapsed: {}", format_duration(*elapsed))),
                ],
            ),
            JobOutcome::Cancelled | JobOutcome::Skipped => (
                "CONVERSION CANCELLED",
                WARNING,
                vec![Line::raw("The app-owned temporary output was removed.")],
            ),
            _ => ("RESULT", TEXT, vec![Line::raw("The job has finished.")]),
        },
        records => {
            let converted = app.succeeded_count();
            let headline = if cancelled {
                format!(
                    "Queue cancelled: {converted} of {} files converted in {}.",
                    records.len(),
                    format_duration(elapsed)
                )
            } else {
                format!(
                    "{converted} of {} files converted in {}.",
                    records.len(),
                    format_duration(elapsed)
                )
            };
            let color = if converted == records.len() {
                ACCENT
            } else {
                WARNING
            };
            (
                if cancelled {
                    "QUEUE CANCELLED"
                } else {
                    "QUEUE COMPLETE"
                },
                color,
                vec![Line::styled(headline, Style::default().fg(color))],
            )
        }
    };
    if app.queue.len() > 1 {
        lines.push(Line::default());
        lines.extend(app.queue.iter().map(outcome_line));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().fg(color))
            .block(panel_block(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// One row per queued file, so a partly failed run says exactly what happened to what.
fn outcome_line<'a>(record: &JobRecord) -> Line<'a> {
    let (marker, detail, color) = match &record.outcome {
        JobOutcome::Succeeded { elapsed } => (
            "✓",
            format!(
                "{}  ({})",
                record.output.display(),
                format_duration(*elapsed)
            ),
            ACCENT,
        ),
        JobOutcome::Failed(error) => ("✗", error.clone(), ERROR),
        JobOutcome::Cancelled => ("■", "cancelled".to_owned(), WARNING),
        JobOutcome::Skipped => ("·", "not started".to_owned(), MUTED),
        JobOutcome::Pending | JobOutcome::Running => ("·", "running".to_owned(), MUTED),
    };
    Line::from(vec![
        Span::styled(format!("{marker} "), Style::default().fg(color)),
        Span::styled(
            fixed_width(&file_label(&record.input), 28),
            Style::default().fg(TEXT),
        ),
        Span::styled(detail, Style::default().fg(color)),
    ])
}

fn render_error(frame: &mut Frame<'_>, app: &App, area: Rect) {
    // One file reports its own failure; a queue says how many failed and then names
    // them, because each can fail for a different reason.
    let mut lines = match app.queue.as_slice() {
        [record] => vec![Line::styled(
            match &record.outcome {
                JobOutcome::Failed(error) => error.clone(),
                _ => "The conversion failed.".to_owned(),
            },
            Style::default().fg(ERROR),
        )],
        [] => vec![Line::styled(
            "An unexpected error occurred.",
            Style::default().fg(ERROR),
        )],
        records => {
            let mut lines = vec![Line::styled(
                format!("All {} files failed to convert.", records.len()),
                Style::default().fg(ERROR),
            )];
            lines.extend(records.iter().map(outcome_line));
            lines
        }
    };
    if !app.stderr_tail.is_empty() {
        lines.push(Line::default());
        lines.push(Line::styled(
            "Recent FFmpeg output:",
            Style::default().fg(MUTED),
        ));
        lines.extend(
            app.stderr_tail
                .iter()
                .rev()
                .take(6)
                .rev()
                .map(|line| Line::styled(line.clone(), Style::default().fg(WARNING))),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("ERROR"))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let help = match app.screen {
        Screen::Configure => {
            " Tab/↑↓ focus   ←→ adjust   i inputs   a add   o output   Enter review   ? help   q quit "
        }
        Screen::Confirm => " Enter/y start   Esc/n back   q quit ",
        Screen::Running => " x cancel   q/Esc cancel menu ",
        Screen::Result | Screen::Error => " Enter back to settings   ? help   q quit ",
    };
    frame.render_widget(
        Paragraph::new(help)
            .style(Style::default().fg(MUTED).bg(PANEL))
            .alignment(Alignment::Center),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(72, 70, area);
    frame.render_widget(Clear, popup);
    let text = vec![
        Line::styled(
            "Keyboard",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::default(),
        Line::raw("Tab / Shift-Tab     Move between settings"),
        Line::raw("Arrow keys / hjkl   Change the selected value"),
        Line::raw("i                   Choose input files (select several at once)"),
        Line::raw("a / c               Add more files / clear the selection"),
        Line::raw("o                   Choose the output file or folder"),
        Line::raw("r                   Read the selected files again"),
        Line::raw("Enter               Review command or edit bitrate"),
        Line::raw("x                   Cancel the run (stops the whole queue)"),
        Line::raw("q                   Quit (asks before cancelling)"),
        Line::raw("? / Esc             Close this help"),
        Line::default(),
        Line::styled(
            "Several files share one set of settings and convert one after another.",
            Style::default().fg(MUTED),
        ),
        Line::styled(
            "Bitrates are entered in kbps. Video: 100–200000. Audio: 32–512.",
            Style::default().fg(WARNING),
        ),
    ];
    frame.render_widget(
        Paragraph::new(text)
            .block(panel_block("HELP"))
            .style(Style::default().bg(BG).fg(TEXT))
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_cancel_confirmation(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(56, 24, area);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new("Stop FFmpeg and abandon the rest of the queue?\n\n[y/Enter] Cancel job    [n/Esc] Keep running")
            .block(panel_block("CANCEL CONVERSION"))
            .style(Style::default().bg(BG).fg(WARNING))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_numeric_edit(frame: &mut Frame<'_>, area: Rect, value: &str, field: ConfigField) {
    let popup = centered_rect(50, 24, area);
    frame.render_widget(Clear, popup);
    let (title, range) = if field == ConfigField::RateValue {
        ("EDIT VIDEO BITRATE", "Valid range: 100–200000 kbps")
    } else {
        ("EDIT AUDIO BITRATE", "Valid range: 32–512 kbps")
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(format!("> {value}_ kbps"), Style::default().fg(ACCENT)),
            Line::default(),
            Line::styled(range, Style::default().fg(MUTED)),
            Line::styled("Enter save  •  Esc cancel", Style::default().fg(MUTED)),
        ])
        .block(panel_block(title))
        .style(Style::default().bg(BG).fg(TEXT))
        .alignment(Alignment::Center),
        popup,
    );
}

fn panel_block<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(SECONDARY).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(PANEL).fg(TEXT))
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let [area] = Layout::vertical([Constraint::Percentage(percent_y)])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn short_version(version: &str) -> String {
    version
        .split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        app::{JobOutcome, JobRecord},
        domain::{InputMedia, RateControlMode, VideoStreamInfo},
        toolchain::Toolchain,
    };

    fn test_app() -> App {
        App::new(Toolchain::test_fixture())
    }

    /// Puts `paths` in the selection with the given probe results already in.
    fn with_inputs(app: &mut App, sources: &[(&str, Option<InputMedia>)]) {
        app.draft.inputs = sources
            .iter()
            .map(|(path, _)| PathBuf::from(path))
            .collect();
        app.draft.output = Some(match app.draft.inputs.as_slice() {
            [input] => OutputTarget::File(input.with_extension("transcoded.mp4")),
            _ => OutputTarget::Directory(PathBuf::from("/exports")),
        });
        for (path, media) in sources {
            app.probes.insert(
                PathBuf::from(path),
                media
                    .clone()
                    .ok_or_else(|| "The selected file does not contain a video stream.".to_owned()),
            );
        }
    }

    fn probed_media() -> InputMedia {
        InputMedia {
            duration: Some(Duration::from_secs(10)),
            video: VideoStreamInfo {
                codec: "h264".to_owned(),
                width: 1920,
                height: 1080,
                frame_rate: Some(30.0),
                bitrate_kbps: Some(100_000),
            },
            audio: Some("aac".to_owned()),
            bitrate_kbps: Some(100_000),
        }
    }

    fn render_text(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal should be created");
        terminal
            .draw(|frame| render(frame, app))
            .expect("UI should render");
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn file_name_column_is_measured_in_terminal_cells() {
        assert_eq!(fixed_width("clip.mov", 12), "clip.mov    ");
        assert_eq!(fixed_width("a-very-long-name.mov", 12), "a-very-long…");
        // Each of these characters takes two cells, so the trim stops at five of them
        // plus the ellipsis and pads the cell that is left over.
        assert_eq!(fixed_width("影片影片影片影片.mov", 12), "影片影片影… ");
        assert_eq!(fixed_width("影片.mov", 12), "影片.mov    ");
    }

    #[test]
    fn formats_short_and_long_durations() {
        assert_eq!(format_duration(Duration::from_secs(65)), "01:05");
        assert_eq!(format_duration(Duration::from_secs(3_665)), "01:01:05");
    }

    #[test]
    fn renders_configure_and_help_in_english() {
        let mut app = test_app();
        let configured = render_text(&app, 100, 30);
        assert!(configured.contains("FFTUI"));
        assert!(configured.contains("No input selected"));
        assert!(configured.contains("Rate control"));
        assert!(configured.contains("Balanced (CRF 23)"));

        app.help_visible = true;
        let help = render_text(&app, 100, 30);
        assert!(help.contains("Keyboard"));
        assert!(help.contains("Change the selected value"));
        assert!(help.contains("Video: 100–200000"));
    }

    #[test]
    fn renders_target_bitrate_and_numeric_editor() {
        let mut app = test_app();
        app.draft.rate_control_mode = RateControlMode::Bitrate;
        app.focus = ConfigField::RateValue;

        let configured = render_text(&app, 100, 30);
        assert!(configured.contains("Target bitrate"));
        assert!(configured.contains("Video bitrate"));
        assert!(configured.contains("5000 kbps"));

        app.handle_key(KeyEvent::from(KeyCode::Enter));
        let editing = render_text(&app, 100, 30);
        assert!(editing.contains("EDIT VIDEO BITRATE"));
        assert!(editing.contains("Valid range: 100–200000 kbps"));
    }

    #[test]
    fn renders_confirmation_running_results_and_error() {
        let mut app = test_app();
        app.screen = Screen::Confirm;
        app.command_preview = Some("'/opt/homebrew/bin/ffmpeg' '-i' 'input file.mp4'".to_owned());
        assert!(render_text(&app, 100, 30).contains("CONFIRM COMMAND"));

        app.screen = Screen::Running;
        app.queue = vec![record("clip.mp4", "/tmp/output.mp4", JobOutcome::Running)];
        app.job = JobState::Running {
            index: 0,
            pid: 42,
            progress: None,
        };
        let running = render_text(&app, 100, 30);
        assert!(running.contains("Starting FFmpeg"));
        assert!(running.contains("No FFmpeg warnings"));

        app.screen = Screen::Result;
        app.queue = vec![record(
            "clip.mp4",
            "/tmp/output.mp4",
            JobOutcome::Succeeded {
                elapsed: Duration::from_secs(4),
            },
        )];
        app.job = JobState::Finished {
            elapsed: Duration::from_secs(4),
            cancelled: false,
        };
        let complete = render_text(&app, 100, 30);
        assert!(complete.contains("CONVERSION COMPLETE"));
        assert!(complete.contains("/tmp/output.mp4"));

        app.queue = vec![record("clip.mp4", "/tmp/output.mp4", JobOutcome::Cancelled)];
        app.job = JobState::Finished {
            elapsed: Duration::from_secs(1),
            cancelled: true,
        };
        assert!(render_text(&app, 100, 30).contains("CONVERSION CANCELLED"));

        app.screen = Screen::Error;
        app.queue = vec![record(
            "clip.mp4",
            "/tmp/output.mp4",
            JobOutcome::Failed("Encoder failed.".to_owned()),
        )];
        app.job = JobState::Finished {
            elapsed: Duration::from_secs(1),
            cancelled: false,
        };
        let error = render_text(&app, 100, 30);
        assert!(error.contains("ERROR"));
        assert!(error.contains("Encoder failed"));
    }

    fn record(input: &str, output: &str, outcome: JobOutcome) -> JobRecord {
        JobRecord {
            input: PathBuf::from(input),
            output: PathBuf::from(output),
            outcome,
        }
    }

    /// A queue has to say what happened to each file: a count alone hides which one
    /// failed, and that is the only thing worth reading on this screen.
    #[test]
    fn renders_a_queue_from_selection_to_per_file_outcome() {
        let mut app = test_app();
        with_inputs(
            &mut app,
            &[
                ("/media/a.mov", Some(probed_media())),
                ("/media/b.mov", None),
            ],
        );

        let selected = render_text(&app, 100, 32);
        assert!(selected.contains("2 files selected"), "{selected}");
        assert!(selected.contains("1 ready"), "{selected}");
        assert!(selected.contains("1 unreadable"), "{selected}");
        assert!(selected.contains("a.mov"), "{selected}");
        // The output row must say the queue writes one file per input.
        assert!(selected.contains("one file each"), "{selected}");
        // An unreadable file blocks the run instead of being dropped from it.
        assert!(selected.contains("NEEDS ATTENTION"), "{selected}");

        // The confirmation must name every file the queue would touch, not only the
        // one whose command is previewed.
        app.screen = Screen::Confirm;
        app.command_preview = Some("'/opt/homebrew/bin/ffmpeg' '-i' '/media/a.mov'".to_owned());
        app.queue = vec![
            record(
                "/media/a.mov",
                "/exports/a.transcoded.mp4",
                JobOutcome::Pending,
            ),
            record(
                "/media/b.mov",
                "/exports/b.transcoded.mp4",
                JobOutcome::Pending,
            ),
        ];
        let confirm = render_text(&app, 100, 32);
        assert!(confirm.contains("CONFIRM COMMAND"), "{confirm}");
        assert!(
            confirm.contains("The same settings run for all 2 files"),
            "{confirm}"
        );
        assert!(confirm.contains("b.mov → b.transcoded.mp4"), "{confirm}");
        assert!(confirm.contains("Estimated total output size"), "{confirm}");

        // Progress names the file being worked on and its place in the queue.
        app.screen = Screen::Running;
        app.job = JobState::Running {
            index: 1,
            pid: 7,
            progress: None,
        };
        let running = render_text(&app, 100, 32);
        assert!(running.contains("FILE 2 OF 2: B.MOV"), "{running}");

        app.screen = Screen::Result;
        app.queue = vec![
            record(
                "/media/a.mov",
                "/exports/a.transcoded.mp4",
                JobOutcome::Succeeded {
                    elapsed: Duration::from_secs(3),
                },
            ),
            record(
                "/media/b.mov",
                "/exports/b.transcoded.mp4",
                JobOutcome::Failed("FFmpeg failed with exit code 1.".to_owned()),
            ),
        ];
        app.job = JobState::Finished {
            elapsed: Duration::from_secs(9),
            cancelled: false,
        };

        let result = render_text(&app, 100, 32);
        assert!(result.contains("QUEUE COMPLETE"), "{result}");
        assert!(result.contains("1 of 2 files converted"), "{result}");
        assert!(result.contains("exit code 1"), "{result}");
    }

    #[test]
    fn renders_the_estimated_output_size() {
        let mut app = test_app();
        assert!(
            render_text(&app, 100, 30).contains("Est. size        Awaiting source"),
            "the estimate row should explain why it is empty"
        );

        with_inputs(&mut app, &[("clip.mp4", Some(probed_media()))]);
        app.draft.rate_control_mode = RateControlMode::Bitrate;
        app.draft.video_bitrate_kbps = 5_000;
        app.draft.audio_bitrate_kbps = 192;
        let targeted = render_text(&app, 100, 30);
        assert!(targeted.contains("Est. size"));
        assert!(targeted.contains("~6.6 MB"), "{targeted}");
        assert!(!targeted.contains("(rough)"));

        app.draft.rate_control_mode = RateControlMode::Quality;
        let heuristic = render_text(&app, 100, 30);
        assert!(heuristic.contains("(rough)"), "{heuristic}");

        // A source whose duration ffprobe could not read must say so, not guess.
        with_inputs(
            &mut app,
            &[(
                "clip.mp4",
                Some(InputMedia {
                    duration: None,
                    ..probed_media()
                }),
            )],
        );
        assert!(render_text(&app, 100, 30).contains("Unknown (source has no duration)"));
    }

    #[test]
    fn renders_without_panicking_in_small_terminals() {
        let mut app = test_app();
        assert!(render_text(&app, 80, 24).contains("FFTUI"));
        assert!(render_text(&app, 60, 20).contains("FFTUI"));

        // A queue must not push the workspace off a terminal that has no spare rows.
        let sources: Vec<_> = (0..40)
            .map(|index| (format!("/media/clip{index}.mov"), Some(probed_media())))
            .collect();
        let borrowed: Vec<_> = sources
            .iter()
            .map(|(path, media)| (path.as_str(), media.clone()))
            .collect();
        with_inputs(&mut app, &borrowed);
        assert!(render_text(&app, 80, 24).contains("40 files selected"));
        assert!(render_text(&app, 60, 20).contains("FFTUI"));
        assert!(render_text(&app, 120, 50).contains("more"));
    }
}
