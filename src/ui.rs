use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap},
};

use crate::{
    app::{App, ConfigField, JobState, Screen},
    domain::{AudioCodec, EstimateBasis, RateControlMode, SizeEstimate},
};

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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(4),
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
        Span::styled("  Single-file transcoder", Style::default().fg(TEXT)),
        Span::styled(format!("  •  {ffmpeg_version}"), Style::default().fg(MUTED)),
    ]);
    frame.render_widget(
        Paragraph::new(title)
            .block(panel_block("READY"))
            .alignment(Alignment::Left),
        area,
    );
}

fn render_source(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut lines = Vec::new();
    let path = app
        .draft
        .input
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "No input selected — press i".to_owned());
    lines.push(Line::from(vec![
        Span::styled("File  ", Style::default().fg(MUTED)),
        Span::styled(path, Style::default().fg(TEXT)),
    ]));
    if let Some(media) = &app.media {
        let duration = media
            .duration
            .map(format_duration)
            .unwrap_or_else(|| "Unknown".to_owned());
        let audio = media
            .audio
            .as_ref()
            .map(|audio| audio.codec.as_str())
            .unwrap_or("None");
        lines.push(Line::from(vec![
            Span::styled("Media ", Style::default().fg(MUTED)),
            Span::styled(
                format!(
                    "{}×{}  •  {}  •  video {}  •  audio {}",
                    media.video.width, media.video.height, duration, media.video.codec, audio
                ),
                Style::default().fg(SECONDARY),
            ),
        ]));
    } else {
        lines.push(Line::styled(
            match app.job {
                JobState::Probing => "Reading streams and metadata…",
                _ => "Choose a local media file to inspect its streams.",
            },
            Style::default().fg(MUTED),
        ));
    }
    frame.render_widget(Paragraph::new(lines).block(panel_block("SOURCE")), area);
}

fn render_settings(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let output = app
        .draft
        .output
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "Not selected".to_owned());
    let rate_value = match app.draft.rate_control_mode {
        RateControlMode::Quality => app.quality_label(),
        RateControlMode::Bitrate => format!("{} kbps", app.draft.video_bitrate_kbps),
    };
    let audio_bitrate_enabled = app.draft.audio_codec != AudioCodec::None
        && app
            .media
            .as_ref()
            .is_some_and(|media| media.audio.is_some());
    let rows = [
        (
            ConfigField::Input,
            "Input",
            "Open file dialog".to_owned(),
            true,
        ),
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
            if app
                .media
                .as_ref()
                .is_some_and(|media| media.audio.is_none())
            {
                "None (source has no audio)".to_owned()
            } else {
                app.draft.audio_codec.to_string()
            },
            app.media.as_ref().is_none_or(|media| media.audio.is_some()),
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
    match app.media {
        // Every path through the estimate multiplies by duration, so without one there
        // is nothing to show but the reason.
        Some(_) => "Unknown (source has no duration)",
        None => "Awaiting source",
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
    if let Some(estimate) = app.size_estimate() {
        text.push_line(Line::default());
        text.push_line(Line::from(vec![
            Span::styled("Estimated output size  ", Style::default().fg(MUTED)),
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
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(2)])
        .split(area);
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
    frame.render_widget(
        Gauge::default()
            .block(panel_block("PROGRESS"))
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
    let (title, color, detail) = match &app.job {
        JobState::Succeeded { output, elapsed } => (
            "CONVERSION COMPLETE",
            ACCENT,
            format!(
                "Output: {}\nElapsed: {}",
                output.display(),
                format_duration(*elapsed)
            ),
        ),
        JobState::Cancelled => (
            "CONVERSION CANCELLED",
            WARNING,
            "The app-owned temporary output was removed.".to_owned(),
        ),
        _ => ("RESULT", TEXT, "The job has finished.".to_owned()),
    };
    frame.render_widget(
        Paragraph::new(detail)
            .style(Style::default().fg(color))
            .block(panel_block(title))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_error(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let error = match &app.job {
        JobState::Failed(error) => error.as_str(),
        _ => "An unexpected error occurred.",
    };
    let mut lines = vec![Line::styled(error, Style::default().fg(ERROR))];
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
            " Tab/↑↓ focus   ←→ adjust   i input   o output   Enter review/edit   ? help   q quit "
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
        Line::raw("i / o               Choose input or output"),
        Line::raw("Enter               Review command or edit bitrate"),
        Line::raw("x                   Cancel a running conversion"),
        Line::raw("q                   Quit (asks before cancelling)"),
        Line::raw("? / Esc             Close this help"),
        Line::default(),
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
        Paragraph::new("Stop FFmpeg and remove the temporary output?\n\n[y/Enter] Cancel job    [n/Esc] Keep running")
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

fn panel_block(title: &str) -> Block<'_> {
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
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
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
        domain::{AudioStreamInfo, InputMedia, RateControlMode, VideoStreamInfo},
        toolchain::Toolchain,
    };

    fn test_app() -> App {
        App::new(Toolchain::test_fixture())
    }

    fn probed_media() -> InputMedia {
        InputMedia {
            path: PathBuf::from("clip.mp4"),
            duration: Some(Duration::from_secs(10)),
            video: VideoStreamInfo {
                codec: "h264".to_owned(),
                width: 1920,
                height: 1080,
                frame_rate: Some(30.0),
                bitrate_kbps: Some(100_000),
            },
            audio: Some(AudioStreamInfo {
                codec: "aac".to_owned(),
                channels: Some(2),
                sample_rate: Some(48_000),
            }),
            format_name: Some("mov,mp4".to_owned()),
            size_bytes: Some(125_000_000),
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
        app.job = JobState::Running {
            pid: 42,
            progress: None,
        };
        let running = render_text(&app, 100, 30);
        assert!(running.contains("Starting FFmpeg"));
        assert!(running.contains("No FFmpeg warnings"));

        app.screen = Screen::Result;
        app.job = JobState::Succeeded {
            output: PathBuf::from("/tmp/output.mp4"),
            elapsed: Duration::from_secs(4),
        };
        assert!(render_text(&app, 100, 30).contains("CONVERSION COMPLETE"));

        app.job = JobState::Cancelled;
        assert!(render_text(&app, 100, 30).contains("CONVERSION CANCELLED"));

        app.screen = Screen::Error;
        app.job = JobState::Failed("Encoder failed.".to_owned());
        let error = render_text(&app, 100, 30);
        assert!(error.contains("ERROR"));
        assert!(error.contains("Encoder failed"));
    }

    #[test]
    fn renders_the_estimated_output_size() {
        let mut app = test_app();
        assert!(
            render_text(&app, 100, 30).contains("Est. size        Awaiting source"),
            "the estimate row should explain why it is empty"
        );

        app.media = Some(probed_media());
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
        app.media = Some(InputMedia {
            duration: None,
            ..probed_media()
        });
        assert!(render_text(&app, 100, 30).contains("Unknown (source has no duration)"));
    }

    #[test]
    fn renders_without_panicking_in_small_terminals() {
        let app = test_app();
        assert!(render_text(&app, 80, 24).contains("FFTUI"));
        assert!(render_text(&app, 60, 20).contains("FFTUI"));
    }
}
