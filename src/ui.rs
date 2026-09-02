use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Flex, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Clear, List, ListItem, Padding, Paragraph, Wrap},
};

use crate::{
    app::{
        App, ConfigField, HoverTarget, JobOutcome, JobRecord, JobState, NavigationButton, Screen,
        UiCommand,
    },
    domain::{EstimateBasis, RateControlMode, SizeEstimate, file_label},
    picker::{Picker, PickerAction, PickerMode, Row},
    theme,
};

// These local aliases keep the drawing code compact while the palette and its
// semantic roles live in the shared theme module.
const BG: Color = theme::BACKGROUND;
const PANEL: Color = theme::PANEL;
const SURFACE: Color = theme::SURFACE;
const SELECTION: Color = theme::SELECTION_BACKGROUND;
const BORDER: Color = theme::BORDER;
const TEXT: Color = theme::FOREGROUND;
const MUTED: Color = theme::MUTED;
const ACCENT: Color = theme::FOCUS;
const SECONDARY: Color = theme::KEY;
const LAVENDER: Color = theme::HEADING;
const WARNING: Color = theme::WORKING;
const ERROR: Color = theme::ERROR;

/// Modal cards, the picker size beaver uses, with a full-width fallback.
const PICKER_WIDTH: u16 = 62;
const PICKER_HEIGHT: u16 = 18;

const AIR_MINIMUM: u16 = 26;
const CARD_WIDTH: u16 = 62;
const WIDE_CARD_WIDTH: u16 = 100;
const DOT_STRIDE: usize = 12;

/// The steps of the workflow, in the order the screens visit them.
const STEPS: [&str; 5] = ["Folders", "Settings", "Review", "Progress", "Done"];

struct UiLayout {
    header: Rect,
    stepper: Rect,
    card: Rect,
    status: Rect,
}

pub fn render(frame: &mut Frame<'_>, app: &App) {
    frame.render_widget(Block::default().style(theme::base()), frame.area());
    if let Some(picker) = &app.picker {
        render_picker(frame, app, picker, frame.area());
        return;
    }
    let area = frame.area();
    let layout = ui_layout(app, area);
    render_header(frame, app, layout.header);
    render_stepper(frame, layout.stepper, app);
    match app.screen {
        Screen::Folders => render_folders(frame, app, layout.card),
        Screen::Settings => render_settings(frame, app, layout.card),
        Screen::Confirm => render_confirm(frame, app, layout.card),
        Screen::Running => render_running(frame, app, layout.card),
        Screen::Result => render_outcome(frame, app, layout.card, "RESULT", theme::SUCCESS),
        Screen::Error => render_outcome(frame, app, layout.card, "ERROR", ERROR),
    }
    render_status(frame, app, layout.status);

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

fn ui_layout(app: &App, area: Rect) -> UiLayout {
    let air = u16::from(area.height >= AIR_MINIMUM);
    let [header, _, stepper, _, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(air),
        Constraint::Length(2),
        Constraint::Length(air),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(area);
    let _ = app;
    UiLayout {
        header,
        stepper,
        card: body.inner(Margin::new(1, 0)),
        status,
    }
}

/// The card follows Beaver's fixed setup width and wider review width.
fn card_rect(area: Rect, screen: Screen) -> Rect {
    let wide = matches!(
        screen,
        Screen::Confirm | Screen::Running | Screen::Result | Screen::Error
    );
    let width = if wide { WIDE_CARD_WIDTH } else { CARD_WIDTH }.min(area.width);
    let wanted_height = if wide {
        area.height
    } else if matches!(screen, Screen::Folders) {
        10
    } else {
        13
    };
    let height = wanted_height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn card_block<'a>(step: Option<usize>, title: &'a str) -> Block<'a> {
    let heading = match step {
        Some(step) => format!(" {step} · {title} "),
        None => format!(" {title} "),
    };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::FOCUS))
        .title_top(Span::styled(heading, theme::heading()))
        .padding(Padding::new(2, 2, 1, 1))
        .style(Style::default().bg(theme::SURFACE).fg(theme::FOREGROUND))
}

/// The step a screen sits in: a number for the card and a dot for the stepper.
fn screen_step(screen: Screen) -> usize {
    match screen {
        Screen::Folders => 0,
        Screen::Settings => 1,
        Screen::Confirm => 2,
        Screen::Running => 3,
        Screen::Result | Screen::Error => 4,
    }
}

// ------------------------------------------------------------------- chrome

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    frame.render_widget(Block::new().style(Style::default().bg(theme::PANEL)), area);
    let ffmpeg_version = short_version(&app.toolchain.ffmpeg_version);
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled("otter", theme::heading().bg(theme::PANEL)),
        Span::styled("  batch transcoder", theme::faint().bg(theme::PANEL)),
        Span::styled(
            format!("  ·  {ffmpeg_version}"),
            theme::faint().bg(theme::PANEL),
        ),
    ]);
    frame.render_widget(Paragraph::new(title), area);
    draw_chips(
        frame,
        area,
        &[('?', "help"), ('q', "quit")],
        |key| app.hover == Some(HoverTarget::HeaderChip(key)),
        PANEL,
    );
}

/// Quiet chips on the right of a bar. The key takes its own block so a shortcut
/// is never only a letter, and a click answers exactly the key it stands for.
fn draw_chips(
    frame: &mut Frame<'_>,
    area: Rect,
    chips: &[(char, &str)],
    is_hovered: impl Fn(char) -> bool,
    background: Color,
) {
    for (rect, (key, label)) in chip_rects(area, chips) {
        frame.render_widget(
            Line::from(chip_spans(key, label, is_hovered(key), background)),
            rect,
        );
    }
}

fn chip_rects<'a>(area: Rect, chips: &'a [(char, &str)]) -> Vec<(Rect, (char, &'a str))> {
    let widths: Vec<u16> = chips
        .iter()
        .map(|(key, label)| (key.width().unwrap_or(0) + 1 + label.width() + 4) as u16)
        .collect();
    let total: u16 = widths.iter().sum::<u16>() + chips.len().saturating_sub(1) as u16;
    if total >= area.width {
        return Vec::new();
    }
    let mut x = area.right() - total;
    let mut rects = Vec::new();
    for ((key, label), width) in chips.iter().zip(widths) {
        rects.push((Rect::new(x, area.y, width, 1), (*key, *label)));
        x += width + 1;
    }
    rects
}

fn chip_spans(key: char, label: &str, hovered: bool, background: Color) -> Vec<Span<'static>> {
    let fill = if hovered {
        theme::hovered_fill(background)
    } else {
        background
    };
    vec![
        Span::styled(format!(" {key} "), theme::key().bg(fill)),
        Span::styled(format!("{label} "), theme::muted().bg(fill)),
    ]
}

/// The bar of dots, modelled on beaver's: every label is centred on its own dot,
/// a finished step turns green, and the current one wears its weight.
fn render_stepper(frame: &mut Frame<'_>, area: Rect, app: &App) {
    if area.height < 2 {
        return;
    }
    let current = screen_step(app.screen);
    let lead = STEPS[0].width().saturating_sub(1) / 2;

    let mut dots: Vec<Span> = Vec::new();
    let mut labels: Vec<Span> = Vec::new();
    let mut used = 0usize;
    for (index, label) in STEPS.iter().enumerate() {
        let (dot, dot_fg, label_fg) = match index.cmp(&current) {
            std::cmp::Ordering::Less => ("●", theme::SUCCESS, theme::MUTED),
            std::cmp::Ordering::Equal => ("●", theme::FOCUS, theme::FOCUS),
            std::cmp::Ordering::Greater => ("○", theme::FAINT, theme::FAINT),
        };
        let dot_style = Style::default().fg(dot_fg);
        let label_style = Style::default()
            .fg(label_fg)
            .add_modifier(if index == current {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        dots.push(Span::styled(dot, dot_style));
        if index + 1 < STEPS.len() {
            dots.push(Span::styled(
                "─".repeat(DOT_STRIDE - 1),
                Style::default().fg(if index < current {
                    theme::SUCCESS
                } else {
                    theme::FAINT
                }),
            ));
        }

        let width = label.width();
        let dot_column = index * DOT_STRIDE + lead;
        let begin = dot_column
            .saturating_sub(width.saturating_sub(1) / 2)
            .max(used);
        labels.push(Span::raw(" ".repeat(begin - used)));
        labels.push(Span::styled((*label).to_owned(), label_style));
        used = begin + width;
    }

    let width = (lead + (STEPS.len() - 1) * DOT_STRIDE + 1).max(used) as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    frame.render_widget(
        Paragraph::new(Line::from(dots)),
        Rect::new(
            (x + lead as u16).min(area.right().saturating_sub(1)),
            area.y,
            width.saturating_sub(lead as u16),
            1,
        ),
    );
    frame.render_widget(
        Paragraph::new(Line::from(labels)),
        Rect::new(x, area.y + 1, width, 1),
    );
}

/// The message line: status on the left, live hints on the right, same as
/// beaver's footer and the only always-open channel the app has.
fn render_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    frame.render_widget(Block::new().style(Style::default().bg(SURFACE)), area);
    let preflight = preflight_error(app);
    let colour = status_colour(app, preflight.is_some());
    let message = preflight
        .or_else(|| app.status_message.clone())
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!(" {message}"),
            Style::default().fg(colour).bg(SURFACE),
        )])),
        area,
    );
    draw_chips(
        frame,
        area,
        &hints(app),
        |key| app.hover == Some(HoverTarget::StatusChip(key)),
        SURFACE,
    );
}

/// A validation problem outweighs status copy: the bar must say what blocks a
/// run rather than what is false when it does.
fn status_colour(app: &App, has_preflight_error: bool) -> Color {
    if has_preflight_error {
        return WARNING;
    }
    match app.screen {
        Screen::Error => ERROR,
        Screen::Result => theme::SUCCESS,
        _ => MUTED,
    }
}

/// Validation only owns the footer before a run starts. Once FFmpeg has begun,
/// the output it creates must not be mistaken for a new preflight conflict.
fn preflight_error(app: &App) -> Option<String> {
    matches!(
        app.screen,
        Screen::Folders | Screen::Settings | Screen::Confirm
    )
    .then(|| app.current_validation_error())
    .flatten()
}

/// The keys the status bar names on the right for this screen.
fn hints(app: &App) -> Vec<(char, &'static str)> {
    match app.screen {
        Screen::Folders => {
            let mut hints = vec![('i', "input"), ('o', "output")];
            if app.button_focus == Some(NavigationButton::Advance) {
                hints.push(('↵', "next"));
            }
            hints
        }
        Screen::Settings => match app.button_focus {
            Some(NavigationButton::Advance) => vec![('↵', "review"), ('←', "back")],
            Some(NavigationButton::Back) => vec![('←', "back")],
            None => vec![('r', "reprobe")],
        },
        Screen::Confirm => vec![('↵', "start"), ('←', "back")],
        Screen::Running => vec![('x', "cancel")],
        Screen::Result | Screen::Error => vec![('↵', "folders"), ('?', "help"), ('q', "quit")],
    }
}

// ------------------------------------------------------------------- mouse

pub fn handle_mouse(app: &mut App, event: MouseEvent, area: Rect) -> UiCommand {
    app.hover = hover_target(app, event.column, event.row, area);
    if event.kind == MouseEventKind::Moved {
        return UiCommand::None;
    }
    let command = if app.picker.is_some() {
        handle_picker_mouse(app, event, area)
    } else if app.help_visible {
        match event.kind {
            MouseEventKind::Down(_) => app.handle_key(KeyEvent::from(KeyCode::Esc)),
            _ => UiCommand::None,
        }
    } else if app.cancel_confirmation || app.numeric_edit_value().is_some() {
        handle_popup_mouse(app, event, area)
    } else {
        let chip = (event.kind == MouseEventKind::Down(MouseButton::Left))
            .then(|| chip_hit(app, event, area))
            .flatten();
        if let Some(key) = chip {
            chip_action(app, key)
        } else {
            match app.screen {
                Screen::Folders => handle_folders_mouse(app, event, area),
                Screen::Settings => handle_settings_mouse(app, event, area),
                Screen::Confirm | Screen::Running | Screen::Result | Screen::Error => {
                    handle_card_mouse(app, event, area)
                }
            }
        }
    };
    app.hover = hover_target(app, event.column, event.row, area);
    command
}

/// Finds only controls that the current screen actually draws. Moved events
/// call this without entering any of the action handlers below, which keeps
/// hover state free of side effects.
fn hover_target(app: &App, column: u16, row: u16, area: Rect) -> Option<HoverTarget> {
    if app.help_visible || app.cancel_confirmation || app.numeric_edit_value().is_some() {
        return None;
    }
    if let Some(picker) = app.picker.as_ref() {
        return picker_hover_target(picker, column, row, area);
    }

    let layout = ui_layout(app, area);
    if let Some((_, (key, _))) = chip_rects(layout.header, &[('?', "help"), ('q', "quit")])
        .into_iter()
        .find(|(rect, _)| contains(*rect, column, row))
    {
        return Some(HoverTarget::HeaderChip(key));
    }
    if let Some((_, (key, _))) = chip_rects(layout.status, &hints(app))
        .into_iter()
        .find(|(rect, _)| contains(*rect, column, row))
    {
        return Some(HoverTarget::StatusChip(key));
    }

    match app.screen {
        Screen::Folders => {
            let folders = folders_layout(app, layout.card);
            if contains(folders.input, column, row) {
                Some(HoverTarget::InputRow)
            } else if contains(folders.output, column, row) {
                Some(HoverTarget::OutputRow)
            } else {
                card_hover_target(app, column, row, layout.card)
            }
        }
        Screen::Settings => {
            let settings = settings_layout(app, layout.card);
            if let Some((index, _rect)) = settings
                .setting_row
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    (
                        index,
                        Rect::new(settings.content.x, *row, settings.content.width, 1),
                    )
                })
                .find(|(_, rect)| contains(*rect, column, row))
            {
                ConfigField::SETTINGS
                    .get(index)
                    .copied()
                    .map(HoverTarget::Setting)
            } else {
                card_hover_target(app, column, row, layout.card)
            }
        }
        Screen::Confirm | Screen::Running | Screen::Result | Screen::Error => {
            card_hover_target(app, column, row, layout.card)
        }
    }
}

fn card_hover_target(app: &App, column: u16, row: u16, area: Rect) -> Option<HoverTarget> {
    card_buttons(app, area)
        .into_iter()
        .find(|(rect, _)| contains(*rect, column, row))
        .map(|(_, code)| match code {
            KeyCode::Esc => HoverTarget::CardButton(NavigationButton::Back),
            KeyCode::Enter | KeyCode::Char('x') => {
                HoverTarget::CardButton(NavigationButton::Advance)
            }
            _ => HoverTarget::CardButton(NavigationButton::Advance),
        })
}

fn picker_hover_target(picker: &Picker, column: u16, row: u16, area: Rect) -> Option<HoverTarget> {
    let layout = picker_layout(picker, area);
    if picker.error.is_none() && contains(layout.list, column, row) {
        let offset = usize::from(row.saturating_sub(layout.list.y));
        let index = picker
            .window(layout.list.height as usize)
            .saturating_add(offset);
        if index < picker.rows.len() {
            return Some(HoverTarget::PickerRow(index));
        }
    }
    if picker.mode == PickerMode::OutputFile && contains(layout.name, column, row) {
        return Some(HoverTarget::PickerName);
    }
    picker_button_rects(picker, layout.buttons)
        .into_iter()
        .find(|(rect, _)| contains(*rect, column, row))
        .map(|(_, button)| match button {
            PickerButton::Cancel => HoverTarget::PickerCancel,
            PickerButton::Parent => HoverTarget::PickerParent,
            PickerButton::Primary => HoverTarget::PickerPrimary,
        })
}

/// A click on a chip answers exactly the key it stands for; `↵` on the
/// configure screen means what it always means on others too.
fn chip_action(app: &mut App, key: char) -> UiCommand {
    if key == '↵' {
        return app.handle_key(KeyEvent::from(KeyCode::Enter));
    }
    if key == '←' {
        return app.handle_key(KeyEvent::from(KeyCode::Esc));
    }
    app.handle_key(KeyEvent::from(KeyCode::Char(key)))
}

fn chip_hit(app: &App, event: MouseEvent, area: Rect) -> Option<char> {
    let layout = ui_layout(app, area);
    for (rect, (key, _)) in chip_rects(layout.header, &[('?', "help"), ('q', "quit")]) {
        if contains(rect, event.column, event.row) {
            return Some(key);
        }
    }
    let hints = hints(app);
    for (rect, (key, _)) in chip_rects(layout.status, &hints) {
        if contains(rect, event.column, event.row) {
            return Some(key);
        }
    }
    None
}

/// The bottom row of a card: the way back on the left, the way forward on the
/// right, exactly where beaver puts them. A click runs the branch its key runs.
fn card_buttons(app: &App, area: Rect) -> Vec<(Rect, KeyCode)> {
    let card = card_rect(area, app.screen);
    let row = card.bottom().saturating_sub(2);
    let mut buttons = Vec::new();
    if matches!(app.screen, Screen::Settings | Screen::Confirm) {
        let label = "← Back";
        let width = label.width() as u16 + 7;
        buttons.push((Rect::new(card.x + 2, row, width, 1), KeyCode::Esc));
    }
    let forward = match app.screen {
        Screen::Folders => Some(("Next →", KeyCode::Enter)),
        Screen::Settings => Some(("Review →", KeyCode::Enter)),
        Screen::Confirm => Some(("Start →", KeyCode::Enter)),
        Screen::Running => Some(("x Cancel", KeyCode::Char('x'))),
        Screen::Result | Screen::Error => Some(("Done", KeyCode::Enter)),
    };
    if let Some((label, code)) = forward {
        let width = label.width() as u16 + 7;
        buttons.push((
            Rect::new(card.right().saturating_sub(width + 2), row, width, 1),
            code,
        ));
    }
    buttons
}

fn render_card_buttons(frame: &mut Frame<'_>, app: &App, card: Rect) {
    for (rect, code) in card_buttons(app, card) {
        let focused = match code {
            KeyCode::Esc => app.button_focus == Some(NavigationButton::Back),
            KeyCode::Enter => app.button_focus == Some(NavigationButton::Advance),
            _ => false,
        };
        let (label, primary) = if code == KeyCode::Enter {
            (
                match app.screen {
                    Screen::Folders => "Next →",
                    Screen::Settings => "Review →",
                    Screen::Confirm => "Start →",
                    _ => "Done",
                },
                true,
            )
        } else {
            (
                match code {
                    KeyCode::Esc => "← Back",
                    KeyCode::Char('x') => "x Cancel",
                    _ => "",
                },
                focused,
            )
        };
        if let Some(key) = button_key(code) {
            let target = if code == KeyCode::Esc {
                HoverTarget::CardButton(NavigationButton::Back)
            } else {
                HoverTarget::CardButton(NavigationButton::Advance)
            };
            draw_button(frame, rect, label, key, primary, app.hover == Some(target));
        }
    }
}

fn button_key(code: KeyCode) -> Option<char> {
    match code {
        KeyCode::Enter => Some('↵'),
        KeyCode::Esc => Some('←'),
        KeyCode::Char('x') => Some('x'),
        _ => None,
    }
}

/// One button of the card's bottom row: `label (key)` in a filled block.
fn draw_button(
    frame: &mut Frame<'_>,
    rect: Rect,
    label: &str,
    key: char,
    primary: bool,
    hovered: bool,
) {
    let normal_bg = if primary { theme::FOCUS } else { SURFACE };
    let bg = if hovered {
        theme::hovered_fill(normal_bg)
    } else {
        normal_bg
    };
    let fg = if primary { BG } else { TEXT };
    let key_fg = if primary { BG } else { MUTED };
    frame.render_widget(
        Line::from(vec![
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(label.to_owned(), Style::default().fg(fg).bg(bg)),
            Span::styled(format!(" ({key})"), Style::default().fg(key_fg).bg(bg)),
            Span::styled("  ", Style::default().bg(bg)),
        ]),
        rect,
    );
}

fn handle_card_mouse(app: &mut App, event: MouseEvent, area: Rect) -> UiCommand {
    let card = card_rect(ui_layout(app, area).card, app.screen);
    if contains(scrolling_card_viewport(app, card), event.column, event.row) {
        match (app.screen, event.kind) {
            (Screen::Confirm, MouseEventKind::ScrollUp) => app.scroll_review_up(3),
            (Screen::Confirm, MouseEventKind::ScrollDown) => {
                app.review_scroll = app.review_scroll.saturating_add(3);
            }
            (Screen::Running, MouseEventKind::ScrollUp) => app.scroll_progress_up(3),
            (Screen::Running, MouseEventKind::ScrollDown) => app.scroll_progress_down(3),
            _ => {}
        }
        return UiCommand::None;
    }
    if event.kind != MouseEventKind::Down(MouseButton::Left) {
        return UiCommand::None;
    }
    let card_area = ui_layout(app, area).card;
    for (rect, code) in card_buttons(app, card_area) {
        if contains(rect, event.column, event.row) {
            if app.screen == Screen::Folders {
                app.button_focus = Some(NavigationButton::Advance);
                app.handle_key(KeyEvent::from(KeyCode::Enter));
                return UiCommand::None;
            }
            if app.screen == Screen::Settings && code == KeyCode::Enter {
                app.button_focus = Some(NavigationButton::Advance);
                app.prepare_confirmation();
                return UiCommand::None;
            }
            return app.handle_key(KeyEvent::from(code));
        }
    }
    UiCommand::None
}

fn scrolling_card_viewport(app: &App, card: Rect) -> Rect {
    let content = card_content_rect(card);
    let button_row = card_buttons(app, card)
        .first()
        .map(|(button, _)| button.y)
        .unwrap_or(content.bottom());
    Rect::new(
        content.x,
        content.y,
        content.width,
        button_row.saturating_sub(content.y + 1),
    )
}

// ------------------------------------------------------------------- folders

struct FoldersLayout {
    input: Rect,
    output: Rect,
}

fn folders_layout(app: &App, area: Rect) -> FoldersLayout {
    let card = card_rect(area, app.screen);
    let content = card_content_rect(card);
    let input_row = content.y + 1;
    let input = Rect::new(content.x, input_row, content.width, 1);
    let output = Rect::new(content.x, input_row + 3, content.width, 1);
    FoldersLayout { input, output }
}

fn card_content_rect(card: Rect) -> Rect {
    Block::bordered()
        .padding(Padding::new(2, 2, 1, 1))
        .inner(card)
}

fn render_folders(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let card = card_rect(area, app.screen);
    let input = app
        .draft
        .inputs
        .as_slice()
        .first()
        .map(|path| {
            if app.draft.inputs.len() == 1 {
                path.display().to_string()
            } else {
                format!("{} video files selected", app.draft.inputs.len())
            }
        })
        .unwrap_or_else(|| "Select one or more video files".to_owned());
    let output = app
        .draft
        .output
        .as_ref()
        .filter(|target| target.is_directory())
        .map(|target| target.path().display().to_string())
        .unwrap_or_else(|| "Select an output folder".to_owned());
    let width = card.width.saturating_sub(6) as usize;
    let lines = vec![
        heading_line("Input video file(s)"),
        folder_line(app, ConfigField::Input, &input, width),
        Line::default(),
        heading_line("Output folder"),
        folder_line(app, ConfigField::Output, &output, width),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(card_block(Some(1), "Folders")),
        card,
    );
    render_card_buttons(frame, app, card);
}

fn heading_line(label: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(label.to_owned(), theme::faint()),
    ])
}

fn folder_line(app: &App, field: ConfigField, value: &str, width: usize) -> Line<'static> {
    let focused = app.screen == Screen::Folders && app.button_focus.is_none() && app.focus == field;
    let hovered = matches!(
        (field, app.hover),
        (ConfigField::Input, Some(HoverTarget::InputRow))
            | (ConfigField::Output, Some(HoverTarget::OutputRow))
    );
    let fill = if hovered {
        theme::hovered_fill(theme::SELECTION_BACKGROUND)
    } else if focused {
        theme::SELECTION_BACKGROUND
    } else {
        theme::SURFACE
    };
    let marker = if hovered {
        "› "
    } else if focused {
        "▸ "
    } else {
        "  "
    };
    let label = capped(value, width.saturating_sub(2));
    let mut spans = vec![
        Span::styled(
            marker,
            Style::default()
                .fg(if hovered || focused {
                    theme::FOCUS
                } else {
                    theme::MUTED
                })
                .bg(fill),
        ),
        Span::styled(label, Style::default().fg(theme::FOREGROUND).bg(fill)),
    ];
    let used = spans.iter().map(|span| span.content.width()).sum::<usize>();
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(used)),
        Style::default().bg(fill),
    ));
    Line::from(spans)
}

fn handle_folders_mouse(app: &mut App, event: MouseEvent, area: Rect) -> UiCommand {
    let card_area = ui_layout(app, area).card;
    if event.kind == MouseEventKind::Down(MouseButton::Left) {
        for (rect, code) in card_buttons(app, card_area) {
            if contains(rect, event.column, event.row) {
                if code == KeyCode::Enter {
                    app.button_focus = Some(NavigationButton::Advance);
                    app.handle_key(KeyEvent::from(KeyCode::Enter));
                }
                return UiCommand::None;
            }
        }
    }
    let layout = folders_layout(app, card_area);
    if contains(layout.input, event.column, event.row)
        && event.kind == MouseEventKind::Down(MouseButton::Left)
    {
        app.focus = ConfigField::Input;
        app.button_focus = None;
        return UiCommand::OpenInputs { add: false };
    }
    if contains(layout.output, event.column, event.row)
        && event.kind == MouseEventKind::Down(MouseButton::Left)
    {
        app.focus = ConfigField::Output;
        app.button_focus = None;
        return UiCommand::OpenOutput;
    }
    UiCommand::None
}

// ------------------------------------------------------------------- settings

struct SettingsLayout {
    content: Rect,
    setting_row: Vec<u16>,
}

fn settings_layout(app: &App, area: Rect) -> SettingsLayout {
    let card = card_rect(area, app.screen);
    let content = card_content_rect(card);
    let start = card.y + 2;
    SettingsLayout {
        content,
        setting_row: (0..ConfigField::SETTINGS.len())
            .map(|offset| start + offset as u16)
            .collect(),
    }
}

fn render_settings(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let card = card_rect(area, app.screen);
    let rate_value = match app.draft.rate_control_mode {
        RateControlMode::Quality => app.quality_label(),
        RateControlMode::Bitrate => format!("{} kbps", app.draft.video_bitrate_kbps),
    };
    let audio_enabled = app.audio_bitrate_enabled();
    let rows = [
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
            if audio_enabled {
                format!("{} kbps", app.draft.audio_bitrate_kbps)
            } else {
                "Disabled".to_owned()
            },
            audio_enabled,
        ),
    ];
    let mut lines = rows
        .into_iter()
        .map(|(field, label, value, enabled)| {
            setting_line(
                app,
                field,
                label,
                value,
                enabled,
                card.width.saturating_sub(6) as usize,
            )
        })
        .collect::<Vec<_>>();
    lines.push(estimate_line(app));
    frame.render_widget(
        Paragraph::new(lines).block(card_block(Some(2), "Settings")),
        card,
    );
    render_card_buttons(frame, app, card);
}

fn handle_settings_mouse(app: &mut App, event: MouseEvent, area: Rect) -> UiCommand {
    let card_area = ui_layout(app, area).card;
    for (rect, code) in card_buttons(app, card_area) {
        if event.kind == MouseEventKind::Down(MouseButton::Left)
            && contains(rect, event.column, event.row)
        {
            if code == KeyCode::Enter {
                app.button_focus = Some(NavigationButton::Advance);
                app.handle_key(KeyEvent::from(KeyCode::Enter));
            } else {
                app.handle_key(KeyEvent::from(code));
            }
            return UiCommand::None;
        }
    }
    let layout = settings_layout(app, card_area);
    if let Some(field) = layout
        .setting_row
        .iter()
        .enumerate()
        .find(|(_, row)| {
            contains(
                Rect::new(layout.content.x, **row, layout.content.width, 1),
                event.column,
                event.row,
            )
        })
        .and_then(|(index, _)| ConfigField::SETTINGS.get(index).copied())
    {
        return handle_setting_mouse(app, field, event);
    }
    UiCommand::None
}

fn handle_setting_mouse(app: &mut App, field: ConfigField, event: MouseEvent) -> UiCommand {
    app.focus = field;
    app.button_focus = None;
    if (field == ConfigField::AudioCodec && app.all_sources_silent())
        || (field == ConfigField::AudioBitrate && !app.audio_bitrate_enabled())
    {
        return UiCommand::None;
    }
    match event.kind {
        MouseEventKind::Down(MouseButton::Right) | MouseEventKind::ScrollUp => {
            app.handle_key(KeyEvent::from(KeyCode::Left))
        }
        MouseEventKind::ScrollDown => app.handle_key(KeyEvent::from(KeyCode::Right)),
        MouseEventKind::Down(MouseButton::Left) => activate_setting(app, field),
        _ => UiCommand::None,
    }
}

fn activate_setting(app: &mut App, field: ConfigField) -> UiCommand {
    match field {
        ConfigField::RateValue if app.draft.rate_control_mode == RateControlMode::Bitrate => {
            app.handle_key(KeyEvent::from(KeyCode::Enter))
        }
        ConfigField::AudioBitrate if app.audio_bitrate_enabled() => {
            app.handle_key(KeyEvent::from(KeyCode::Enter))
        }
        _ => app.handle_key(KeyEvent::from(KeyCode::Right)),
    }
}

/// One setting line of the configure card. A focused line takes the selection
/// fill for its whole width, like a beaver control row.
fn setting_line<'a>(
    app: &App,
    field: ConfigField,
    label: &'a str,
    value: String,
    enabled: bool,
    width: usize,
) -> Line<'a> {
    let focused =
        app.screen == Screen::Settings && app.button_focus.is_none() && app.focus == field;
    let hovered = app.hover == Some(HoverTarget::Setting(field));
    let fill = if hovered {
        theme::hovered_fill(theme::SELECTION_BACKGROUND)
    } else if focused {
        theme::SELECTION_BACKGROUND
    } else {
        theme::SURFACE
    };
    let marker = if hovered {
        "› "
    } else if focused {
        "▸ "
    } else {
        "  "
    };
    let value_style = if !enabled {
        Style::default().fg(theme::MUTED).bg(fill)
    } else {
        Style::default().fg(theme::FOREGROUND).bg(fill)
    };
    let mut spans = vec![
        Span::styled(" ", Style::default().bg(fill)),
        Span::styled(
            marker,
            Style::default()
                .fg(if focused { ACCENT } else { MUTED })
                .bg(fill),
        ),
        Span::styled(
            format!("{label:<16}"),
            Style::default()
                .fg(if focused { ACCENT } else { MUTED })
                .bg(fill),
        ),
        Span::styled(format!(" {value} "), value_style),
    ];
    let used = spans.iter().map(|span| span.content.width()).sum::<usize>();
    spans.push(Span::styled(
        " ".repeat(width.saturating_sub(used)),
        Style::default().bg(fill),
    ));
    Line::from(spans)
}

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
        Span::styled(
            format!("   {:<16}", "Est. size"),
            Style::default().fg(MUTED),
        ),
        Span::styled(format!(" {value} "), style),
    ])
}

/// Targeted estimates get the same weight as a real setting; heuristic ones are
/// muted toward the advisory palette so a rough number never reads as a
/// measured one.
fn estimate_color(estimate: SizeEstimate) -> Color {
    match estimate.basis {
        EstimateBasis::Targeted => SECONDARY,
        EstimateBasis::Heuristic => MUTED,
    }
}

fn estimate_placeholder(app: &App) -> &'static str {
    // Every path through the estimate multiplies by duration, so without one
    // there is nothing to show but the reason.
    if app.probed_count() > 0 {
        "Unknown (source has no duration)"
    } else {
        "Awaiting source"
    }
}

// ---------------------------------------------------------------- review

fn render_confirm(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let card = card_rect(area, app.screen);
    let viewport = scrolling_card_viewport(app, card);
    let preview = app
        .command_preview
        .as_deref()
        .unwrap_or("Command preview unavailable.");
    let mut text = Text::from(vec![
        Line::styled(
            "Output is written to an app-owned temporary file first.",
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
        for record in &app.queue {
            text.push_line(Line::styled(
                format!(
                    "  {} → {}",
                    file_label(&record.input),
                    file_label(&record.output)
                ),
                Style::default().fg(SECONDARY),
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
    frame.render_widget(card_block(Some(3), "Review"), card);
    let wrapped = wrap_lines(text.lines, viewport.width);
    let max_scroll = wrapped.len().saturating_sub(viewport.height as usize) as u16;
    let paragraph = Paragraph::new(wrapped);
    frame.render_widget(
        paragraph.scroll((app.review_scroll.min(max_scroll), 0)),
        viewport,
    );
    render_card_buttons(frame, app, card);
}

// ---------------------------------------------------------------- progress

fn render_running(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let card = card_rect(area, app.screen);
    let viewport = scrolling_card_viewport(app, card);
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
    let counter = match (&app.job, app.queue.len()) {
        (_, 0 | 1) => None,
        (JobState::Running { index, .. }, total) => Some(format!(
            "FILE {} OF {total}: {}",
            index + 1,
            app.queue
                .get(*index)
                .map(|record| file_label(&record.input))
                .unwrap_or_default()
                .to_uppercase()
        )),
        (_, total) => Some(format!("{} OF {total} DONE", app.succeeded_count())),
    };

    let bar_width = (card.width as usize).saturating_sub(6);
    let filled = (ratio.clamp(0.0, 1.0) * bar_width as f64).round() as usize;
    let mut lines = vec![Line::from(Span::styled(
        label,
        Style::default().fg(if matches!(app.job, JobState::Cancelling) {
            WARNING
        } else {
            ACCENT
        }),
    ))];
    if let Some(counter) = counter {
        lines.push(Line::styled(counter, Style::default().fg(MUTED)));
    }
    lines.push(Line::from(vec![
        Span::styled("█".repeat(filled), Style::default().fg(ACCENT)),
        Span::styled(
            "░".repeat(bar_width.saturating_sub(filled)),
            Style::default().fg(BORDER),
        ),
    ]));
    lines.push(Line::default());
    lines.push(Line::styled("FFmpeg messages", Style::default().fg(MUTED)));
    if app.stderr_tail.is_empty() {
        lines.push(Line::styled(
            "No FFmpeg warnings.",
            Style::default().fg(MUTED),
        ));
    } else {
        lines.extend(
            app.stderr_tail
                .iter()
                .map(|line| Line::styled(line.as_str(), Style::default().fg(WARNING))),
        );
    }

    frame.render_widget(card_block(Some(4), "Progress"), card);
    let wrapped = wrap_lines(lines, viewport.width);
    let max_scroll = wrapped.len().saturating_sub(viewport.height as usize) as u16;
    let paragraph = Paragraph::new(wrapped);
    let scroll = if app.progress_follow {
        max_scroll
    } else if app.progress_scroll_from_top {
        app.progress_scroll.min(max_scroll)
    } else {
        max_scroll.saturating_sub(app.progress_scroll)
    };
    frame.render_widget(paragraph.scroll((scroll, 0)), viewport);
    render_card_buttons(frame, app, card);
}

fn wrap_lines(lines: Vec<Line<'_>>, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut wrapped = Vec::new();
    for line in lines {
        if line.spans.is_empty() {
            wrapped.push(Line::default());
            continue;
        }
        let mut output = Vec::new();
        let mut used: usize = 0;
        for span in line.spans {
            let mut chunk = String::new();
            for character in span.content.chars() {
                let character_width = character.width().unwrap_or(0);
                if used > 0 && used.saturating_add(character_width) > width {
                    if !chunk.is_empty() {
                        output.push(Span::styled(std::mem::take(&mut chunk), span.style));
                    }
                    wrapped.push(Line::from(std::mem::take(&mut output)));
                    used = 0;
                }
                chunk.push(character);
                used = used.saturating_add(character_width);
            }
            if !chunk.is_empty() {
                output.push(Span::styled(chunk, span.style));
            }
        }
        wrapped.push(Line::from(output));
    }
    wrapped
}

// ---------------------------------------------------------------- results

fn render_outcome(frame: &mut Frame<'_>, app: &App, area: Rect, title: &str, colour: Color) {
    let card = card_rect(area, app.screen);
    let (elapsed, cancelled) = match &app.job {
        JobState::Finished { elapsed, cancelled } => (*elapsed, *cancelled),
        _ => (Duration::ZERO, false),
    };
    let mut lines: Vec<Line> = vec![];
    match app.queue.as_slice() {
        [record] => match &record.outcome {
            JobOutcome::Succeeded { elapsed } => {
                lines.push(Line::styled(
                    "CONVERSION COMPLETE",
                    Style::default().fg(colour).add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::styled(
                    format!("Output: {}", record.output.display()),
                    Style::default().fg(colour),
                ));
                lines.push(Line::styled(
                    format!("Elapsed: {}", format_duration(*elapsed)),
                    Style::default().fg(MUTED),
                ));
            }
            JobOutcome::Cancelled | JobOutcome::Skipped => {
                lines.push(Line::styled(
                    "CONVERSION CANCELLED",
                    Style::default().fg(WARNING),
                ));
                lines.push(Line::styled(
                    "The app-owned temporary output was removed.",
                    Style::default().fg(MUTED),
                ));
            }
            JobOutcome::Failed(error) => {
                lines.push(Line::styled(
                    error.clone(),
                    Style::default().fg(ERROR).add_modifier(Modifier::BOLD),
                ));
                lines.push(Line::styled(
                    "Nothing was written to the destination.",
                    Style::default().fg(MUTED),
                ));
            }
            _ => lines.push(Line::styled(
                "The job has finished.",
                Style::default().fg(colour),
            )),
        },
        records => {
            let converted = app.succeeded_count();
            let title = if cancelled {
                "QUEUE CANCELLED"
            } else {
                "QUEUE COMPLETE"
            };
            let title_colour = if cancelled {
                WARNING
            } else if converted == records.len() {
                colour
            } else {
                WARNING
            };
            lines.push(Line::styled(
                title,
                Style::default()
                    .fg(title_colour)
                    .add_modifier(Modifier::BOLD),
            ));
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
            let headline_colour = if converted == records.len() {
                colour
            } else {
                WARNING
            };
            lines.push(Line::styled(headline, Style::default().fg(headline_colour)));
        }
    };
    if app.queue.len() > 1 {
        lines.push(Line::default());
        let available = (card.height as usize).saturating_sub(6);
        lines.extend(app.queue.iter().take(available).map(outcome_line));
    }
    if title == "ERROR" && !app.stderr_tail.is_empty() {
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
            .style(Style::default().fg(colour))
            .block(card_block(Some(5), "Done"))
            .wrap(Wrap { trim: false }),
        card,
    );
    render_card_buttons(frame, app, card);
}

/// One row per queued file, so a partly failed run says exactly what happened
/// to what.
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
            Style::default().fg(color),
        ),
        Span::styled(detail, Style::default().fg(color)),
    ])
}

// ------------------------------------------------------------------- picker

/// The picker card regions, shared between rendering and the mouse handler.
struct PickerLayout {
    card: Rect,
    path: Rect,
    name: Rect,
    list: Rect,
    buttons: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerButton {
    Cancel,
    Parent,
    Primary,
}

fn picker_layout(picker: &Picker, area: Rect) -> PickerLayout {
    let width = PICKER_WIDTH.min(area.width);
    let height = PICKER_HEIGHT.min(area.height.saturating_sub(1));
    let card = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height + 1) / 2,
        width,
        height,
    );
    let inner = Block::bordered()
        .padding(Padding::horizontal(1))
        .inner(card);
    let name_rows = if picker.mode == PickerMode::OutputFile {
        2
    } else {
        0
    };
    let [path, name, list, buttons] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(name_rows),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    PickerLayout {
        card,
        path,
        name,
        list,
        buttons,
    }
}

/// The picker screen: the modal card on the floor, and the same status bar
/// under it so its hints stay on the same line.
fn render_picker(frame: &mut Frame<'_>, app: &App, picker: &Picker, area: Rect) {
    let layout = picker_layout(picker, area);

    let title = match picker.mode {
        PickerMode::InputFolder => "INPUT FOLDER",
        PickerMode::InputFiles => "INPUT VIDEO FILES",
        PickerMode::OutputFile => "SAVE OUTPUT",
        PickerMode::OutputFolder => "CHOOSE FOLDER",
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(LAVENDER))
        .title_top(Span::styled(
            format!(" {title} "),
            Style::default().fg(LAVENDER).add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(PANEL).fg(TEXT));
    frame.render_widget(block, layout.card);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            trimmed_path(&picker.dir, layout.path.width),
            Style::default().fg(TEXT),
        ))),
        layout.path,
    );

    if picker.mode == PickerMode::OutputFile {
        render_picker_name(frame, app, picker, layout.name);
    }

    if let Some(error) = picker.error.as_deref() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                error.to_owned(),
                Style::default().fg(WARNING),
            )))
            .wrap(Wrap { trim: false }),
            layout.list,
        );
    } else {
        render_picker_list(frame, app, picker, layout.list);
    }

    render_picker_buttons(frame, app, picker, layout.buttons);

    let status = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
    frame.render_widget(Block::new().style(Style::default().bg(SURFACE)), status);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            picker.footer_help().to_owned(),
            Style::default().fg(MUTED).bg(SURFACE),
        ))),
        status,
    );
}

/// The file name field of the save card: a label line and an editable line that
/// takes the selection fill while it is focused, like a beaver path field.
fn render_picker_name(frame: &mut Frame<'_>, app: &App, picker: &Picker, area: Rect) {
    let [heading, field] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  Name".to_owned(),
            Style::default().fg(MUTED),
        ))),
        heading,
    );
    let fill = if app.hover == Some(HoverTarget::PickerName) {
        theme::HOVER
    } else if picker.editing_name {
        SELECTION
    } else {
        PANEL
    };
    let value = format!(
        "  {}{}",
        capped(&picker.filename, (field.width as usize).saturating_sub(3)),
        if picker.editing_name { "_" } else { "" }
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            value,
            Style::default().fg(TEXT).bg(fill),
        ))),
        field,
    );
}

fn render_picker_list(frame: &mut Frame<'_>, app: &App, picker: &Picker, list: Rect) {
    if list.height == 0 {
        return;
    }
    let name_width = (list.width as usize).saturating_sub(13).max(8);
    // The visible slice is rendered row by row so pointer hover can be painted
    // independently of the keyboard cursor.
    let window = picker.window(list.height as usize);
    let rows = picker.rows.get(window..).unwrap_or_default();
    for (offset, row) in rows.iter().take(list.height as usize).enumerate() {
        let index = window + offset;
        let focused = index == picker.cursor;
        let hovered = app.hover == Some(HoverTarget::PickerRow(index));
        let fill = if hovered {
            theme::HOVER
        } else if focused {
            SELECTION
        } else {
            PANEL
        };
        let item = picker_list_item(picker, row, name_width, focused, hovered).style(
            Style::default().fg(TEXT).bg(fill).add_modifier(if hovered {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
        );
        frame.render_widget(
            List::new(vec![item]),
            Rect::new(list.x, list.y + offset as u16, list.width, 1),
        );
    }
}

/// One listing row: directories with a slash, files with their size in a fixed
/// right-aligned column, and a tick in front of every marked file.
fn picker_list_item<'a>(
    picker: &Picker,
    row: &'a Row,
    name_width: usize,
    focused: bool,
    hovered: bool,
) -> ListItem<'a> {
    let marker = if hovered {
        "› "
    } else if focused {
        "▸ "
    } else {
        "  "
    };
    match row {
        Row::Parent => ListItem::new(Line::from(vec![Span::styled(
            format!("{marker}.."),
            Style::default().fg(if hovered { TEXT } else { MUTED }),
        )])),
        Row::Entry(entry) => {
            let input = picker.mode == PickerMode::InputFiles;
            let marked = picker
                .selected
                .iter()
                .any(|path| path.as_os_str() == entry.path.as_os_str());
            let prefix = if entry.is_dir {
                ("", SECONDARY)
            } else if marked {
                ("[✓] ", ACCENT)
            } else if input {
                ("[ ] ", MUTED)
            } else {
                ("  ", MUTED)
            };
            let mut name = entry.name.clone();
            if entry.is_dir {
                name.push('/');
            }
            let name_style = if entry.hidden {
                Style::default().fg(MUTED)
            } else if entry.is_dir {
                Style::default().fg(SECONDARY)
            } else {
                Style::default().fg(TEXT)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    marker,
                    Style::default().fg(if hovered { TEXT } else { MUTED }),
                ),
                Span::styled(prefix.0, Style::default().fg(prefix.1)),
                Span::styled(capped(&name, name_width), name_style),
                Span::styled(
                    format!(
                        "{:>10}",
                        if entry.is_dir {
                            String::new()
                        } else {
                            human_size(entry.size)
                        }
                    ),
                    Style::default().fg(MUTED),
                ),
            ]))
        }
    }
}

/// The buttons of the picker card: cancel and parent on the left, the mode's
/// primary action on the right, exactly where beaver puts them.
fn render_picker_buttons(frame: &mut Frame<'_>, app: &App, picker: &Picker, area: Rect) {
    for (rect, button) in picker_button_rects(picker, area) {
        let (label, key, primary, enabled) = match button {
            PickerButton::Cancel => ("Cancel".to_owned(), "esc", false, true),
            PickerButton::Parent => ("Parent".to_owned(), "←", false, true),
            PickerButton::Primary => {
                let label = picker.primary_label();
                let enabled = picker.primary_ready();
                draw_picker_button(
                    frame,
                    rect,
                    &label,
                    "s",
                    true,
                    enabled,
                    app.hover == Some(HoverTarget::PickerPrimary),
                );
                continue;
            }
        };
        let hovered = match button {
            PickerButton::Cancel => app.hover == Some(HoverTarget::PickerCancel),
            PickerButton::Parent => app.hover == Some(HoverTarget::PickerParent),
            PickerButton::Primary => false,
        };
        draw_picker_button(frame, rect, &label, key, primary, enabled, hovered);
    }
}

fn picker_button_rects(picker: &Picker, area: Rect) -> Vec<(Rect, PickerButton)> {
    let mut rects = Vec::new();
    let cancel_width = picker_button_width("Cancel", "esc");
    rects.push((
        Rect::new(area.x, area.y, cancel_width, 1),
        PickerButton::Cancel,
    ));
    let mut left = area.x + cancel_width;
    if picker.dir.parent().is_some() {
        let parent_width = picker_button_width("Parent", "←");
        rects.push((
            Rect::new(left, area.y, parent_width, 1),
            PickerButton::Parent,
        ));
        left += parent_width;
    }
    let label = picker.primary_label();
    let width = picker_button_width(&label, "s");
    let x = area.right().saturating_sub(width);
    if x >= left {
        rects.push((Rect::new(x, area.y, width, 1), PickerButton::Primary));
    }
    rects
}

fn picker_button_width(label: &str, key: &str) -> u16 {
    label.width() as u16 + key.width() as u16 + 7
}

fn draw_picker_button(
    frame: &mut Frame<'_>,
    rect: Rect,
    label: &str,
    key: &str,
    primary: bool,
    enabled: bool,
    hovered: bool,
) {
    let (fg, bg) = match (primary, enabled) {
        (true, true) => (
            BG,
            if hovered {
                theme::hovered_fill(ACCENT)
            } else {
                ACCENT
            },
        ),
        (true, false) => (
            MUTED,
            if hovered {
                theme::hovered_fill(SURFACE)
            } else {
                SURFACE
            },
        ),
        (false, _) => (
            TEXT,
            if hovered {
                theme::hovered_fill(SURFACE)
            } else {
                SURFACE
            },
        ),
    };
    let key_fg = if primary && enabled { BG } else { MUTED };
    frame.render_widget(
        Line::from(vec![
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(label.to_owned(), Style::default().fg(fg).bg(bg)),
            Span::styled(format!(" ({key})"), Style::default().fg(key_fg).bg(bg)),
            Span::styled("  ", Style::default().bg(bg)),
        ]),
        rect,
    );
}

fn handle_picker_mouse(app: &mut App, event: MouseEvent, area: Rect) -> UiCommand {
    let picker = app.picker.as_ref().expect("a picker is open");
    let layout = picker_layout(picker, area);

    if event.kind == MouseEventKind::Down(MouseButton::Left) {
        if contains(layout.name, event.column, event.row) && picker.mode == PickerMode::OutputFile {
            if let Some(picker) = app.picker.as_mut() {
                picker.focus_name();
            }
            return UiCommand::None;
        }
        if contains(layout.buttons, event.column, event.row) {
            return handle_picker_button_click(app, event, &layout);
        }
        if contains(layout.list, event.column, event.row) {
            if let Some(action @ (PickerAction::Done(_) | PickerAction::Cancel)) = app
                .picker
                .as_mut()
                .map(|picker| picker.handle_mouse(event, layout.list))
            {
                app.close_picker(action);
            }
            return UiCommand::None;
        }
    } else if matches!(
        event.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    ) && contains(layout.list, event.column, event.row)
    {
        app.picker
            .as_mut()
            .map(|picker| picker.handle_mouse(event, layout.list));
        if let Some(picker) = app.picker.as_ref() {
            app.hover = picker_hover_target(picker, event.column, event.row, area);
        }
    }
    UiCommand::None
}

/// A click in the button row. The buttons are measured exactly as they are
/// drawn, left to right, so the lead one answers the click.
fn handle_picker_button_click(
    app: &mut App,
    event: MouseEvent,
    layout: &PickerLayout,
) -> UiCommand {
    let picker = app.picker.as_ref().expect("a picker is open");
    let Some((_, button)) = picker_button_rects(picker, layout.buttons)
        .into_iter()
        .find(|(rect, _)| contains(*rect, event.column, event.row))
    else {
        return UiCommand::None;
    };
    match button {
        PickerButton::Cancel => {
            let _ = app.handle_key(KeyEvent::from(KeyCode::Esc));
        }
        PickerButton::Parent => {
            app.picker
                .as_mut()
                .map(|picker| picker.handle_key(KeyEvent::from(KeyCode::Left)));
        }
        PickerButton::Primary => {
            let action = app
                .picker
                .as_mut()
                .map(|picker| picker.handle_key(KeyEvent::from(KeyCode::Char('s'))))
                .unwrap_or(PickerAction::None);
            app.close_picker(action);
        }
    }
    UiCommand::None
}

// ------------------------------------------------------------------- helpers

fn contains(area: Rect, column: u16, row: u16) -> bool {
    area.contains(Position::new(column, row))
}

fn capped(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    let mut kept = String::new();
    let mut used = 0;
    for character in value.chars() {
        let cells = character.width().unwrap_or(0);
        if used + cells + 1 > width {
            break;
        }
        kept.push(character);
        used += cells;
    }
    format!("{kept}…")
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

/// A path trimmed from the left so the folder being visited always stays
/// visible, e.g. "…/Users/demo/movies/holiday" instead of anything the width
/// can only hold from the left.
fn trimmed_path(path: &std::path::Path, width: u16) -> String {
    let text = path.display().to_string();
    let keep = width as usize;
    if text.width() <= keep {
        return text;
    }
    let mut tail: Vec<char> = text.chars().rev().take(keep.saturating_sub(2)).collect();
    tail.reverse();
    format!("…{}", tail.into_iter().collect::<String>())
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    let popup = centered_rect(72, 80, area);
    frame.render_widget(Clear, popup);
    let text = vec![
        Line::styled(
            "Keyboard",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::default(),
        Line::raw("Tab / Shift-Tab     Move between folders, settings, and buttons"),
        Line::raw("Arrow keys / hjkl   Change the selected value"),
        Line::raw("i / a / r            Pick, add, or replace video files"),
        Line::raw("o                   Choose the output folder"),
        Line::raw("r                   Read the selected files again on Settings"),
        Line::raw("Enter               Continue, review, or edit bitrate"),
        Line::raw("Review / Progress   Up/Down, PageUp/PageDown, Home/End scroll"),
        Line::raw("x                   Cancel the run (stops the whole queue)"),
        Line::raw("q                   Quit (asks before cancelling)"),
        Line::raw("? / Esc             Close this help"),
        Line::styled(
            "Mouse",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Line::raw("Left click          Activate a setting or a card button"),
        Line::raw("Wheel / right click Change settings; wheel scrolls their content"),
        Line::raw("File row click      Open the input or output picker"),
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
            .block(card_block(None, "HELP"))
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
            .block(card_block(None, "CANCEL CONVERSION"))
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
        .block(card_block(None, title))
        .style(Style::default().bg(BG).fg(TEXT))
        .alignment(Alignment::Center),
        popup,
    );
}

fn handle_popup_mouse(app: &mut App, event: MouseEvent, area: Rect) -> UiCommand {
    let code = match event.kind {
        MouseEventKind::Down(MouseButton::Right) => Some(KeyCode::Esc),
        MouseEventKind::Down(MouseButton::Left) if app.cancel_confirmation => {
            let popup = centered_rect(56, 24, area);
            let content = bordered_inner(popup);
            let line = "[y/Enter] Cancel job    [n/Esc] Keep running";
            if centered_text_hit(
                content,
                content.y + 2,
                line,
                "[y/Enter] Cancel job",
                event.column,
                event.row,
            ) {
                Some(KeyCode::Enter)
            } else if centered_text_hit(
                content,
                content.y + 2,
                line,
                "[n/Esc] Keep running",
                event.column,
                event.row,
            ) {
                Some(KeyCode::Esc)
            } else {
                None
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let popup = centered_rect(50, 24, area);
            let content = bordered_inner(popup);
            let line = "Enter save  •  Esc cancel";
            if centered_text_hit(
                content,
                content.y + 3,
                line,
                "Enter save",
                event.column,
                event.row,
            ) {
                Some(KeyCode::Enter)
            } else if centered_text_hit(
                content,
                content.y + 3,
                line,
                "Esc cancel",
                event.column,
                event.row,
            ) {
                Some(KeyCode::Esc)
            } else {
                None
            }
        }
        _ => None,
    };
    code.map_or(UiCommand::None, |code| app.handle_key(KeyEvent::from(code)))
}

fn centered_text_hit(
    area: Rect,
    line_row: u16,
    line: &str,
    label: &str,
    column: u16,
    row: u16,
) -> bool {
    let Some(byte_offset) = line.find(label) else {
        return false;
    };
    let mut rendered_width: u16 = 0;
    for character in line.chars() {
        let width = u16::try_from(character.width().unwrap_or(0)).unwrap_or(u16::MAX);
        if rendered_width.saturating_add(width) > area.width {
            break;
        }
        rendered_width += width;
    }
    let prefix_width = u16::try_from(line[..byte_offset].width()).unwrap_or(u16::MAX);
    let label_width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    if prefix_width.saturating_add(label_width) > rendered_width {
        return false;
    }
    let start = area.x + area.width / 2 - rendered_width / 2;
    let label_start = start + prefix_width;
    let label_end = label_start.saturating_add(label_width);
    row == line_row && column >= label_start && column < label_end
}

fn bordered_inner(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
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
    use std::{fs, path::PathBuf};

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::{
        app::{JobOutcome, JobRecord},
        domain::{InputMedia, OutputTarget, RateControlMode, VideoStreamInfo},
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
        app.input_folder = app
            .draft
            .inputs
            .first()
            .and_then(|input| input.parent())
            .map(PathBuf::from);
        app.draft.output = Some(OutputTarget::Directory(PathBuf::from("/exports")));
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

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn fixed_width_measures_terminal_cells() {
        assert_eq!(fixed_width("clip.mov", 12), "clip.mov    ");
        assert_eq!(fixed_width("a-very-long-name.mov", 12), "a-very-long…");
        // Each of these characters takes two cells, so the trim stops at five of
        // them plus the ellipsis, then pads the cell that is left over.
        assert_eq!(fixed_width("影片影片影片影片.mov", 12), "影片影片影… ");
        assert_eq!(fixed_width("影片.mov", 12), "影片.mov    ");
    }

    #[test]
    fn formats_short_and_long_durations() {
        assert_eq!(format_duration(Duration::from_secs(65)), "01:05");
        assert_eq!(format_duration(Duration::from_secs(3_665)), "01:01:05");
    }

    #[test]
    fn renders_the_stepper_and_the_folders_card() {
        let app = test_app();
        let rendered = render_text(&app, 100, 30);
        assert!(rendered.contains("otter"), "{rendered}");
        assert!(rendered.contains("Folders"), "{rendered}");
        assert!(rendered.contains("Settings"), "{rendered}");
        assert!(rendered.contains("●"), "{rendered}");
        assert!(rendered.contains("1 · Folders"), "{rendered}");
        assert!(rendered.contains("Input video file(s)"), "{rendered}");
        assert!(rendered.contains("Output folder"), "{rendered}");
        assert!(!rendered.contains("Rate control"), "{rendered}");
        assert!(rendered.contains("i input"), "{rendered}");
    }

    #[test]
    fn renders_help_in_english() {
        let mut app = test_app();
        app.help_visible = true;
        let help = render_text(&app, 100, 30);
        assert!(help.contains("Keyboard"));
        assert!(help.contains("Change the selected value"));
        assert!(help.contains("PageUp/PageDown"), "{help}");
        assert!(help.contains("wheel scrolls their content"), "{help}");
        assert!(help.contains("Bitrates are entered"));
    }

    #[test]
    fn mouse_and_keyboard_share_the_settings_actions() {
        let mut app = test_app();
        let area = Rect::new(0, 0, 100, 30);
        app.screen = Screen::Settings;
        let layout = settings_layout(&app, ui_layout(&app, area).card);
        let container_row = layout.setting_row[0];

        let initial_container = app.draft.container;
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), 40, container_row),
            area,
        );
        assert_eq!(app.focus, ConfigField::Container);
        assert_ne!(app.draft.container, initial_container);

        // Folder rows remain available on the first screen.
        app.screen = Screen::Folders;
        let folder_layout = folders_layout(&app, ui_layout(&app, area).card);
        let replace = handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                folder_layout.input.x + 1,
                folder_layout.input.y,
            ),
            area,
        );
        assert_eq!(replace, UiCommand::OpenInputs { add: false });

        app.focus = ConfigField::Input;
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.focus, ConfigField::Output);
        app.handle_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(app.button_focus, Some(NavigationButton::Advance));
    }

    #[test]
    fn moved_mouse_updates_hover_without_changing_settings() {
        let mut app = test_app();
        let area = Rect::new(0, 0, 100, 30);
        app.screen = Screen::Settings;
        app.focus = ConfigField::Container;
        let layout = settings_layout(&app, ui_layout(&app, area).card);
        let row = layout.setting_row[0];
        let before = app.draft.container;

        assert_eq!(
            handle_mouse(
                &mut app,
                mouse(MouseEventKind::Moved, layout.content.x + 2, row),
                area,
            ),
            UiCommand::None
        );
        assert_eq!(
            app.hover,
            Some(HoverTarget::Setting(ConfigField::Container))
        );
        assert_eq!(app.draft.container, before);

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(layout.content.x + 2, row)].bg,
            theme::HOVER
        );

        handle_mouse(&mut app, mouse(MouseEventKind::Moved, 0, 0), area);
        assert_eq!(app.hover, None);
        assert_eq!(app.draft.container, before);
    }

    #[test]
    fn folders_mouse_click_regression_tracks_rendered_path_rows() {
        let mut app = test_app();
        let area = Rect::new(0, 0, 100, 30);
        let card = card_rect(ui_layout(&app, area).card, app.screen);
        let content = card_content_rect(card);
        let column = content.x + 2;
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Moved, column, content.y),
            area,
        );
        assert_eq!(app.hover, None, "the input heading is not a clickable row");

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Moved, column, content.y + 1),
            area,
        );
        assert_eq!(
            app.hover,
            Some(HoverTarget::InputRow),
            "the rendered input path row should hover on its own line"
        );
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(column, content.y + 1)].bg,
            theme::HOVER
        );

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Moved, column, content.y + 2),
            area,
        );
        assert_eq!(app.hover, None, "the blank line is not a clickable row");

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Moved, column, content.y + 3),
            area,
        );
        assert_eq!(app.hover, None, "the output heading is not a clickable row");

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Moved, column, content.y + 4),
            area,
        );
        assert_eq!(
            app.hover,
            Some(HoverTarget::OutputRow),
            "the rendered output path row should hover on its own line"
        );
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(column, content.y + 4)].bg,
            theme::HOVER
        );
    }

    #[test]
    fn picker_hover_does_not_move_or_select_a_row() {
        let root = tempfile::tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("clip.mov"), b"").unwrap();
        let mut app = test_app();
        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            false,
        ));
        let area = Rect::new(0, 0, 100, 30);
        let list = picker_layout(app.picker.as_ref().unwrap(), area).list;
        let cursor = app.picker.as_ref().unwrap().cursor;
        let selected = app.picker.as_ref().unwrap().selected.clone();

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Moved, list.x + 2, list.y + 1),
            area,
        );
        assert_eq!(app.hover, Some(HoverTarget::PickerRow(1)));
        assert_eq!(app.picker.as_ref().unwrap().cursor, cursor);
        assert_eq!(app.picker.as_ref().unwrap().selected, selected);

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert_eq!(
            terminal.backend().buffer()[(list.x + 2, list.y + 1)].bg,
            theme::HOVER
        );

        let buttons = picker_layout(app.picker.as_ref().unwrap(), area).buttons;
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Moved, buttons.x + 2, buttons.y),
            area,
        );
        assert_eq!(app.hover, Some(HoverTarget::PickerCancel));
        assert_eq!(app.picker.as_ref().unwrap().cursor, cursor);
        assert_eq!(app.picker.as_ref().unwrap().selected, selected);
    }

    #[test]
    fn mouse_activates_the_status_hints_and_card_buttons() {
        let mut app = test_app();
        let area = Rect::new(0, 0, 100, 30);
        let layout = ui_layout(&app, area);
        let root = tempfile::tempdir().expect("a temp directory should be created");
        let input = root.path().join("clip.mov");
        let output = root.path().join("exports");
        fs::write(&input, b"").unwrap();
        fs::create_dir(&output).unwrap();

        // The status bar's input chip opens the picker, exactly like the key.
        let hints = hints(&app);
        let chips = chip_rects(layout.status, &hints);
        let (rect, _) = chips
            .iter()
            .find(|(_, (key, _))| *key == 'i')
            .copied()
            .unwrap_or_else(|| panic!("status chips should include input: {chips:?}"));
        let command = handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), rect.x + 2, rect.y),
            area,
        );
        assert_eq!(command, UiCommand::OpenInputs { add: false });
        assert_eq!(app.screen, Screen::Folders);

        // The Folders card advances without depending on which path row held focus.
        let input_text = input.to_string_lossy().into_owned();
        with_inputs(&mut app, &[(input_text.as_str(), Some(probed_media()))]);
        app.draft.output = Some(OutputTarget::Directory(output));
        app.focus = ConfigField::Input;
        let body = ui_layout(&app, area).card;
        let (rect, _) = card_buttons(&app, body)
            .into_iter()
            .find(|(_, code)| *code == KeyCode::Enter)
            .expect("the folders card has a next button");
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), rect.x + 2, rect.y),
            area,
        );
        assert_eq!(app.screen, Screen::Settings);

        // Settings keeps Beaver's left-side Back button reachable by mouse.
        let body = ui_layout(&app, area).card;
        let (rect, _) = card_buttons(&app, body)
            .into_iter()
            .find(|(_, code)| *code == KeyCode::Esc)
            .expect("the settings card has a back button");
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), rect.x + 2, rect.y),
            area,
        );
        assert_eq!(app.screen, Screen::Folders);

        // The Settings card's right-side Review button prepares the next step.
        app.leave_folders();
        let body = ui_layout(&app, area).card;
        let (rect, _) = card_buttons(&app, body)
            .into_iter()
            .find(|(_, code)| *code == KeyCode::Enter)
            .expect("the settings card has a review button");
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), rect.x + 2, rect.y),
            area,
        );
        assert_eq!(app.screen, Screen::Confirm);

        // The start button of the confirm card comes back with Esc on the back
        // button, exactly like the key.
        app.screen = Screen::Confirm;
        let body = ui_layout(&app, area).card;
        let (rect, _) = card_buttons(&app, body)
            .into_iter()
            .find(|(_, code)| *code == KeyCode::Esc)
            .expect("the confirm card has a back button");
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), rect.x + 2, rect.y),
            area,
        );
        assert_eq!(app.screen, Screen::Settings);
    }

    #[test]
    fn renders_target_bitrate_and_numeric_editor() {
        let mut app = test_app();
        app.screen = Screen::Settings;
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
        let confirm = render_text(&app, 100, 30);
        assert!(confirm.contains("3 · Review"), "{confirm}");
        assert!(confirm.contains("input file.mp4"), "{confirm}");

        app.screen = Screen::Running;
        app.queue = vec![record("clip.mp4", "/tmp/output.mp4", JobOutcome::Running)];
        app.job = JobState::Running {
            index: 0,
            pid: 42,
            progress: None,
        };
        let running = render_text(&app, 100, 30);
        assert!(running.contains("4 · Progress"), "{running}");
        assert!(running.contains("Starting FFmpeg"), "{running}");
        assert!(running.contains("No FFmpeg warnings"), "{running}");

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
        assert!(complete.contains("5 · Done"), "{complete}");
        assert!(complete.contains("CONVERSION COMPLETE"), "{complete}");
        assert!(complete.contains("/tmp/output.mp4"), "{complete}");

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
        assert!(error.contains("5 · Done"), "{error}");
        assert!(error.contains("Encoder failed"), "{error}");
    }

    #[test]
    fn done_footer_does_not_revalidate_the_output_it_just_created() {
        let root = tempfile::tempdir().expect("a temp directory should be created");
        let input = root.path().join("clip.mov");
        let output = root.path().join("exports");
        fs::write(&input, b"").unwrap();
        fs::create_dir(&output).unwrap();

        let mut app = test_app();
        let input_text = input.to_string_lossy().into_owned();
        with_inputs(&mut app, &[(input_text.as_str(), Some(probed_media()))]);
        app.draft.output = Some(OutputTarget::Directory(output));
        let final_output = app
            .draft
            .output_path_for(&input)
            .expect("a folder target should derive an output path");
        fs::write(&final_output, b"already written").unwrap();
        assert!(
            app.current_validation_error()
                .is_some_and(|message| message.contains("already exists"))
        );

        app.screen = Screen::Result;
        app.status_message = Some("Conversion completed successfully.".to_owned());
        app.job = JobState::Finished {
            elapsed: Duration::from_secs(1),
            cancelled: false,
        };
        let rendered = render_text(&app, 100, 30);
        assert!(rendered.contains("Conversion completed successfully."));
        assert!(!rendered.contains("already exists"), "{rendered}");
    }

    fn record(input: &str, output: &str, outcome: JobOutcome) -> JobRecord {
        JobRecord {
            input: PathBuf::from(input),
            output: PathBuf::from(output),
            outcome,
        }
    }

    /// A queue has to say what happened to each file: a count alone hides which
    /// one failed, and that is the only thing worth reading on this screen.
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
        app.screen = Screen::Settings;

        let selected = render_text(&app, 100, 32);
        assert!(selected.contains("Container"), "{selected}");
        assert!(selected.contains("Rate control"), "{selected}");

        // The confirmation must name every file the queue would touch, not only
        // the one whose command is previewed.
        app.screen = Screen::Confirm;
        app.command_preview = Some("'/opt/homebrew/bin/ffmpeg' '-i' '/media/a.mov'".to_owned());
        app.queue = vec![
            record(
                "/media/a.mov",
                "/exports/a-transcode.mp4",
                JobOutcome::Pending,
            ),
            record(
                "/media/b.mov",
                "/exports/b-transcode.mp4",
                JobOutcome::Pending,
            ),
        ];
        let confirm = render_text(&app, 100, 32);
        assert!(
            confirm.contains("The same settings run for all 2 files"),
            "{confirm}"
        );
        assert!(confirm.contains("b.mov → b-transcode.mp4"), "{confirm}");
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
                "/exports/a-transcode.mp4",
                JobOutcome::Succeeded {
                    elapsed: Duration::from_secs(3),
                },
            ),
            record(
                "/media/b.mov",
                "/exports/b-transcode.mp4",
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
        app.screen = Screen::Settings;
        assert!(
            render_text(&app, 100, 30).contains("Est. size"),
            "the estimate row should explain why it is empty"
        );

        with_inputs(&mut app, &[("clip.mp4", Some(probed_media()))]);
        app.draft.rate_control_mode = RateControlMode::Bitrate;
        app.draft.video_bitrate_kbps = 5_000;
        app.draft.audio_bitrate_kbps = 192;
        let targeted = render_text(&app, 100, 30);
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
    fn estimated_size_row_aligns_with_setting_rows() {
        let mut app = test_app();
        app.screen = Screen::Settings;
        app.button_focus = Some(NavigationButton::Advance);
        let area = Rect::new(0, 0, 100, 30);
        let layout = settings_layout(&app, ui_layout(&app, area).card);
        let estimate_row = layout.setting_row[ConfigField::SETTINGS.len() - 1] + 1;
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let row_text = |row| {
            (0..area.width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        };
        let container = row_text(layout.setting_row[0]);
        let estimate = row_text(estimate_row);
        let container_label = container.find("Container").unwrap();
        let estimate_label = estimate.find("Est. size").unwrap();
        let container_value = container.find(&app.draft.container.to_string()).unwrap();
        let estimate_value = estimate.find(estimate_placeholder(&app)).unwrap();

        assert_eq!(estimate_label, container_label, "label columns must align");
        assert_eq!(estimate_value, container_value, "value columns must align");
    }

    #[test]
    fn running_stderr_stays_inside_its_card_and_does_not_leak_to_the_next_screen() {
        const STDERR_MARKER: &str = "§";
        const STDERR_CONTENT: &str = "¤";

        let mut app = test_app();
        app.screen = Screen::Running;
        app.queue = vec![record("clip.mp4", "/tmp/output.mp4", JobOutcome::Running)];
        app.job = JobState::Running {
            index: 0,
            pid: 42,
            progress: None,
        };
        for _ in 0..20 {
            app.stderr_tail.push_back(format!(
                "{}{}",
                STDERR_CONTENT.repeat(WIDE_CARD_WIDTH as usize * 2),
                STDERR_MARKER
            ));
        }

        let area = Rect::new(0, 0, 140, 40);
        let running_card = card_rect(ui_layout(&app, area).card, app.screen);
        let button_row = card_buttons(&app, ui_layout(&app, area).card)[0].0.y;
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &app)).unwrap();

        let buffer = terminal.backend().buffer();
        let stderr_cells = (0..area.height)
            .flat_map(|row| (0..area.width).map(move |column| (column, row)))
            .filter(|&(column, row)| buffer[(column, row)].symbol() == STDERR_CONTENT)
            .collect::<Vec<_>>();
        assert!(
            !stderr_cells.is_empty(),
            "the stderr content should be rendered"
        );
        assert!(
            stderr_cells.iter().all(|&(column, row)| {
                column > running_card.x
                    && column < running_card.right() - 1
                    && row > running_card.y
                    && row < button_row
            }),
            "stderr content must not enter the card border, controls, or the area outside it"
        );
        let marker_cells = (0..area.height)
            .flat_map(|row| (0..area.width).map(move |column| (column, row)))
            .filter(|&(column, row)| buffer[(column, row)].symbol() == STDERR_MARKER)
            .collect::<Vec<_>>();
        assert!(
            !marker_cells.is_empty(),
            "the wrapped tail must be reachable"
        );
        assert!(
            marker_cells.iter().all(|&(column, row)| {
                column > running_card.x
                    && column < running_card.right() - 1
                    && row > running_card.y
                    && row < button_row
            }),
            "wrapped tails must stay out of the border and controls"
        );

        app.screen = Screen::Settings;
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        assert!(
            (0..area.height).all(|row| (0..area.width).all(|column| {
                !matches!(
                    buffer[(column, row)].symbol(),
                    STDERR_CONTENT | STDERR_MARKER
                )
            })),
            "stderr content from the progress screen must not remain on the next screen"
        );
    }

    #[test]
    fn review_renders_every_mapping_and_scrolls_with_keys_and_wheel() {
        let mut app = test_app();
        app.screen = Screen::Confirm;
        app.command_preview = Some("ffmpeg -i source.mp4".to_owned());
        app.queue = (1..=12)
            .map(|index| {
                record(
                    &format!("/media/source-{index}.mov"),
                    &format!("/exports/source-{index}-transcode.mp4"),
                    JobOutcome::Pending,
                )
            })
            .collect();

        let top = render_text(&app, 80, 18);
        assert!(top.contains("source-1.mov"), "{top}");
        assert!(!top.contains("source-12.mov"), "{top}");

        app.handle_key(KeyEvent::from(KeyCode::End));
        let bottom = render_text(&app, 80, 18);
        assert!(bottom.contains("source-12.mov"), "{bottom}");
        assert!(!bottom.contains("…"), "{bottom}");

        app.handle_key(KeyEvent::from(KeyCode::Home));
        let area = Rect::new(0, 0, 80, 18);
        let card = card_rect(ui_layout(&app, area).card, app.screen);
        let viewport = scrolling_card_viewport(&app, card);
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::ScrollDown, viewport.x, viewport.y),
            area,
        );
        assert_eq!(app.review_scroll, 3);
    }

    #[test]
    fn progress_wraps_long_lines_and_preserves_manual_scroll() {
        const TAIL: &str = "TAIL_MARKER";
        let mut app = test_app();
        app.screen = Screen::Running;
        app.queue = vec![record("clip.mp4", "/tmp/output.mp4", JobOutcome::Running)];
        app.job = JobState::Running {
            index: 0,
            pid: 42,
            progress: None,
        };
        app.stderr_tail
            .push_back(format!("{} {TAIL}", "long-message ".repeat(100)));

        let bottom = render_text(&app, 62, 30);
        assert!(bottom.contains(TAIL), "{bottom}");

        app.handle_key(KeyEvent::from(KeyCode::Home));
        assert!(!app.progress_follow);
        let top = render_text(&app, 62, 30);
        assert!(!top.contains(TAIL), "{top}");
        app.handle_key(KeyEvent::from(KeyCode::End));
        assert!(app.progress_follow);
        assert!(render_text(&app, 62, 30).contains(TAIL));

        let area = Rect::new(0, 0, 62, 30);
        let card = card_rect(ui_layout(&app, area).card, app.screen);
        let viewport = scrolling_card_viewport(&app, card);
        handle_mouse(
            &mut app,
            mouse(MouseEventKind::ScrollUp, viewport.x, viewport.y),
            area,
        );
        assert!(!app.progress_follow);
        assert_eq!(app.progress_scroll, 3);
    }

    #[test]
    fn renders_without_panicking_in_small_terminals() {
        let mut app = test_app();
        assert!(render_text(&app, 80, 24).contains("otter"));
        // A terminal too short for the stepper keeps the shell of the layout.
        assert!(render_text(&app, 60, 10).contains("otter"));

        // A queue must not push the workspace off a terminal that has no spare rows.
        let sources: Vec<_> = (0..40)
            .map(|index| (format!("/media/clip{index}.mov"), Some(probed_media())))
            .collect();
        let borrowed: Vec<_> = sources
            .iter()
            .map(|(path, media)| (path.as_str(), media.clone()))
            .collect();
        with_inputs(&mut app, &borrowed);
        app.screen = Screen::Settings;
        let rendered = render_text(&app, 80, 24);
        assert!(rendered.contains("Rate control"), "{rendered}");
    }

    // ------------------------------------------------------------- the picker

    #[test]
    fn picker_renders_as_a_modal_card_with_its_buttons() {
        let root = tempfile::tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("holiday.mkv"), b"").unwrap();
        fs::create_dir(root.path().join("archive")).unwrap();
        let mut app = test_app();
        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            false,
        ));

        let rendered = render_text(&app, 100, 30);
        assert!(rendered.contains("INPUT VIDEO FILES"), "{rendered}");
        assert!(rendered.contains("archive"), "{rendered}");
        assert!(rendered.contains("holiday.mkv"), "{rendered}");
        assert!(rendered.contains("[ ]"), "{rendered}");
        assert!(rendered.contains("Cancel (esc)"), "{rendered}");
        assert!(rendered.contains("Parent (←)"), "{rendered}");
        assert!(rendered.contains("Done (0)"), "{rendered}");
        assert!(rendered.contains("s done"), "{rendered}");
    }

    #[test]
    fn input_files_mouse_click_regression_selects_file_through_ui() {
        let root = tempfile::tempdir().expect("a temp directory should be created");
        let file = root.path().join("holiday.mkv");
        fs::write(&file, b"").unwrap();
        let mut app = test_app();
        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            false,
        ));

        let area = Rect::new(0, 0, 100, 30);
        let picker = app.picker.as_ref().expect("the picker should be open");
        let layout = picker_layout(picker, area);
        let file_index = picker
            .rows
            .iter()
            .position(|row| matches!(row, Row::Entry(entry) if entry.path == file))
            .expect("the file should be listed");
        let row = layout.list.y + (file_index - picker.window(layout.list.height as usize)) as u16;

        handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                layout.list.x + 2,
                row,
            ),
            area,
        );

        let picker = app
            .picker
            .as_ref()
            .expect("a single click must keep the picker open");
        assert_eq!(picker.selected, vec![file]);
    }

    #[test]
    fn save_picker_shows_the_name_field() {
        let root = tempfile::tempdir().expect("a temp directory should be created");
        let mut app = test_app();
        app.picker = Some(Picker::open(
            PickerMode::OutputFile,
            root.path().to_owned(),
            Some("clip.mkv".to_owned()),
            false,
        ));
        let rendered = render_text(&app, 100, 30);
        assert!(rendered.contains("SAVE OUTPUT"), "{rendered}");
        assert!(rendered.contains("Name"), "{rendered}");
        assert!(rendered.contains("clip.mkv"), "{rendered}");
        assert!(rendered.contains("Save here (s)"), "{rendered}");
    }

    #[test]
    fn double_click_on_a_file_closes_the_picker_and_selects_it() {
        let root = tempfile::tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("a.mp4"), b"").unwrap();
        let mut app = test_app();
        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            false,
        ));
        let area = Rect::new(0, 0, 100, 30);
        let list = picker_layout(app.picker.as_ref().unwrap(), area).list;
        let row = list.y + 1; // a.mp4, right after the parent row

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), list.x + 2, row),
            area,
        );
        assert!(app.picker.is_some());

        handle_mouse(
            &mut app,
            mouse(MouseEventKind::Down(MouseButton::Left), list.x + 2, row),
            area,
        );
        assert!(app.picker.is_none(), "the second click should confirm");
        assert_eq!(app.draft.inputs, vec![root.path().join("a.mp4")]);
    }

    #[test]
    fn the_primary_button_does_not_fire_with_an_empty_name() {
        let root = tempfile::tempdir().expect("a temp directory should be created");
        let mut app = test_app();
        app.picker = Some(Picker::open(
            PickerMode::OutputFile,
            root.path().to_owned(),
            None, // no default name
            false,
        ));
        let area = Rect::new(0, 0, 100, 30);
        let buttons = picker_layout(app.picker.as_ref().unwrap(), area).buttons;
        let primary = buttons.right().saturating_sub(13);

        handle_mouse(
            &mut app,
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                primary + 1,
                buttons.y,
            ),
            area,
        );
        assert!(app.picker.is_some(), "an empty name must not confirm");
        assert!(render_text(&app, 100, 30).contains("Enter a file name first"));
    }
}
