//! In-terminal file pickers, one card at a time.
//!
//! The old native panels were macOS-styled and AppKit-specific, which needed a
//! helper process to keep AppKit out of the TUI. The picker drawn by beaver in
//! `src/tui/picker.rs` is the model here: a modal card with the path on top, a
//! listing that highlights its row by filling the background, and one button row
//! at the bottom. The host application owns the picker state, so its event loop,
//! screen, and mouse handling stay in one place.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

/// What the picker asks for. The three flows the application had with native
/// panels become three modes of the same card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    /// Legacy folder mode retained for callers that use the picker directly.
    /// The application input workflow uses [`PickerMode::InputFiles`].
    InputFolder,
    /// Any number of media files, one per line or Space-selected.
    InputFiles,
    /// One destination file: a folder plus a file name.
    OutputFile,
    /// One destination folder for a queue.
    OutputFolder,
}

/// The outcome of one key press or click, as a decision the application can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    /// Consumed the input; no decision yet.
    None,
    /// The user chose these paths and the picker closed.
    Done(Vec<PathBuf>),
    /// The user dismissed the picker; no selection was made.
    Cancel,
}

/// One directory entry as the picker shows it.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Display name. File names are bytes on Unix, so lossy conversion keeps the
    /// picker from failing on a name the terminal cannot render alone.
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub hidden: bool,
}

/// The rows of one directory listing. A parent row leads the real entries so
/// "go up" stays available even when the directory is empty or holds only files.
#[derive(Debug, Clone)]
pub enum Row {
    /// Navigate to the parent directory.
    Parent,
    Entry(Entry),
}

#[derive(Debug)]
pub struct Picker {
    pub mode: PickerMode,
    /// Input-mode flag: `true` appends the picked files to the current selection
    /// instead of replacing it.
    pub append: bool,
    pub dir: PathBuf,
    pub rows: Vec<Row>,
    pub cursor: usize,
    /// Extra scroll offset on top of the cursor-visible window, adjusted by the
    /// wheel. The renderer clamps it so the cursor stays in view.
    pub scroll: usize,
    /// Input mode: the files marked with Space.
    pub selected: Vec<PathBuf>,
    /// Output-file mode: the file name being edited.
    pub filename: String,
    /// Output-file mode: `true` while the file name field has keyboard focus.
    pub editing_name: bool,
    pub show_hidden: bool,
    /// A message that is not a selection, e.g. a directory that could not be read.
    pub error: Option<String>,
    /// Row, timestamp of the previous left click, for double-click detection.
    last_click: Option<(usize, Instant)>,
}

impl Picker {
    pub fn open(mode: PickerMode, dir: PathBuf, file_name: Option<String>, append: bool) -> Self {
        let mut picker = Self {
            mode,
            append,
            dir,
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
            selected: Vec::new(),
            filename: file_name.unwrap_or_default(),
            editing_name: false,
            show_hidden: false,
            error: None,
            last_click: None,
        };
        picker.reload();
        picker
    }

    /// Re-reads the current directory. The cursor resets to the parent row so a
    /// change of directory never lands on a row that is no longer there.
    pub fn reload(&mut self) {
        self.rows.clear();
        self.error = None;
        let entries = match fs::read_dir(&self.dir) {
            Err(error) => {
                self.error = Some(format!("Cannot read this directory: {error}"));
                self.rows.push(Row::Parent);
                return;
            }
            Ok(entries) => entries,
        };

        let mut entries: Vec<Row> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let meta = fs::metadata(&path).ok()?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let hidden = name.starts_with('.');
                if hidden && !self.show_hidden {
                    return None;
                }
                if self.mode == PickerMode::InputFolder && !meta.is_dir() {
                    return None;
                }
                // metadata follows symlinks: a link to a folder becomes a folder.
                Some(Row::Entry(Entry {
                    name,
                    path,
                    is_dir: meta.is_dir(),
                    size: meta.len(),
                    hidden,
                }))
            })
            .collect();
        // Folders first, then case-insensitive name order, so "Movie.mp4" and
        // "movie.mp4" stay neighbours instead of splitting by letter case.
        entries.sort_by(|a, b| match (a, b) {
            (Row::Entry(a), Row::Entry(b)) => b
                .is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            _ => std::cmp::Ordering::Equal,
        });
        if self.dir.parent().is_some() {
            self.rows.push(Row::Parent);
        }
        self.rows.extend(entries);
        self.cursor = 0;
        self.scroll = 0;
    }

    /// Frame-based window into [`Picker::rows`]: given the same cursor it returns
    /// the same window, so rendering and the mouse handler agree without the
    /// picker owning the terminal size.
    pub fn window(&self, height: usize) -> usize {
        list_window(self.scroll, self.cursor, self.rows.len(), height)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        if self.editing_name {
            return self.handle_name_key(key);
        }
        match key.code {
            KeyCode::Esc => PickerAction::Cancel,
            KeyCode::Up | KeyCode::Char('k') => {
                self.nudge(-1);
                PickerAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.nudge(1);
                PickerAction::None
            }
            KeyCode::PageUp => {
                self.nudge(-10);
                PickerAction::None
            }
            KeyCode::PageDown => {
                self.nudge(10);
                PickerAction::None
            }
            // h fits the hjkl scheme and means what a shell means: move up.
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                self.go_parent();
                PickerAction::None
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => self.activate(),
            KeyCode::Char(' ') => {
                self.toggle_selected();
                PickerAction::None
            }
            // s confirmed the chosen folder in beaver, and it is as memorable here.
            KeyCode::Char('s') => self.confirm_current(),
            KeyCode::Char('.') => {
                self.show_hidden = !self.show_hidden;
                self.reload();
                PickerAction::None
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.go_home();
                PickerAction::None
            }
            KeyCode::End => {
                self.nudge(self.rows.len() as i32);
                PickerAction::None
            }
            KeyCode::Tab if self.mode == PickerMode::OutputFile => {
                self.editing_name = true;
                PickerAction::None
            }
            _ => PickerAction::None,
        }
    }

    /// Wheel and click handling over the listing, routed by the caller with the
    /// listing rectangle the renderer used.
    pub fn handle_mouse(&mut self, event: MouseEvent, list: Rect) -> PickerAction {
        match event.kind {
            MouseEventKind::ScrollUp => {
                self.scroll = self.scroll.saturating_sub(3);
                PickerAction::None
            }
            MouseEventKind::ScrollDown => {
                self.scroll = self.scroll.saturating_add(3);
                PickerAction::None
            }
            MouseEventKind::Down(MouseButton::Left) if contains(list, event.column, event.row) => {
                let Some(row) = event.row.checked_sub(list.y) else {
                    return PickerAction::None;
                };
                let Some(index) = self.window(list.height as usize).checked_add(row as usize)
                else {
                    return PickerAction::None;
                };
                if index >= self.rows.len() {
                    return PickerAction::None;
                }
                self.cursor = index;
                self.last_click = match self.last_click {
                    Some((previous, at))
                        if previous == index && at.elapsed() < DOUBLE_CLICK_WINDOW =>
                    {
                        None
                    }
                    _ => Some((index, Instant::now())),
                };
                if self.last_click.is_none() {
                    // A double click acts like Enter on the row under the cursor.
                    return self.activate();
                }
                if self.mode == PickerMode::InputFiles {
                    self.toggle_selected();
                }
                PickerAction::None
            }
            _ => PickerAction::None,
        }
    }

    /// The bottom-right primary action: what `s` does, what the card's main
    /// button does, and what the status bar names on the right.
    pub fn primary_label(&self) -> String {
        match self.mode {
            PickerMode::InputFolder => "Use this folder".to_owned(),
            PickerMode::InputFiles => format!("Done ({})", self.selected.len()),
            PickerMode::OutputFile => "Save here".to_owned(),
            PickerMode::OutputFolder => "Use this folder".to_owned(),
        }
    }

    /// Whether the primary action can go through right now.
    pub fn primary_ready(&self) -> bool {
        match self.mode {
            PickerMode::InputFolder => true,
            PickerMode::InputFiles => !self.selected.is_empty(),
            PickerMode::OutputFile => !self.filename.trim().is_empty(),
            PickerMode::OutputFolder => true,
        }
    }

    /// Puts the keyboard in the file name field (output-file mode).
    pub fn focus_name(&mut self) {
        if self.mode == PickerMode::OutputFile {
            self.editing_name = true;
        }
    }

    /// The help line of the status bar, one per mode.
    pub fn footer_help(&self) -> &'static str {
        match self.mode {
            PickerMode::InputFolder => {
                " ↑↓ move   ↵ enter   s use this folder   ← parent   esc cancel "
            }
            PickerMode::InputFiles => {
                " ↑↓ move   ↵ open/confirm   space select   s done   ← parent   esc cancel "
            }
            PickerMode::OutputFile => {
                " ↑↓ move   ↵ open   s save here   tab name   ← parent   esc cancel "
            }
            PickerMode::OutputFolder => {
                " ↑↓ move   ↵ open   s use this folder   ← parent   esc cancel "
            }
        }
    }

    fn handle_name_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Char(c) if c.is_ascii_graphic() || c == ' ' => {
                self.filename.push(c);
                PickerAction::None
            }
            KeyCode::Backspace => {
                self.filename.pop();
                PickerAction::None
            }
            KeyCode::Esc => PickerAction::Cancel,
            KeyCode::Enter => self.confirm_current(),
            KeyCode::Tab => {
                self.editing_name = false;
                PickerAction::None
            }
            _ => PickerAction::None,
        }
    }

    fn nudge(&mut self, delta: i32) {
        let count = self.rows.len() as i32;
        if count == 0 {
            return;
        }
        self.cursor = (self.cursor as i32 + delta).rem_euclid(count) as usize;
    }

    fn go_parent(&mut self) {
        let Some(parent) = self.dir.parent().map(Path::to_owned) else {
            return;
        };
        self.reload_at(&parent);
    }

    fn go_home(&mut self) {
        if let Some(home) = home_dir() {
            self.reload_at(&home);
        }
    }

    fn reload_at(&mut self, dir: &Path) {
        if self.dir == dir || !dir.is_dir() {
            return;
        }
        self.dir = dir.to_owned();
        self.reload();
    }

    /// Enter on the focused row: navigate, or take the file as the answer.
    fn activate(&mut self) -> PickerAction {
        let target = match self.rows.get(self.cursor) {
            Some(Row::Parent) => {
                self.go_parent();
                return PickerAction::None;
            }
            Some(Row::Entry(entry)) if entry.is_dir => Some(entry.path.clone()),
            Some(Row::Entry(entry)) => {
                return match self.mode {
                    PickerMode::InputFolder => PickerAction::None,
                    PickerMode::InputFiles => {
                        let mut paths = self.selected.clone();
                        if !paths
                            .iter()
                            .any(|path| path.as_os_str() == entry.path.as_os_str())
                        {
                            paths.push(entry.path.clone());
                        }
                        PickerAction::Done(paths)
                    }
                    PickerMode::OutputFile => PickerAction::Done(vec![entry.path.clone()]),
                    PickerMode::OutputFolder => PickerAction::None,
                };
            }
            None => return PickerAction::None,
        };
        if let Some(target) = target {
            self.reload_at(&target);
        }
        PickerAction::None
    }

    /// The primary action: the mode's whole point.
    fn confirm_current(&mut self) -> PickerAction {
        match self.mode {
            PickerMode::InputFolder => PickerAction::Done(vec![self.dir.clone()]),
            PickerMode::InputFiles => {
                if self.selected.is_empty() {
                    self.error =
                        Some("No files selected — use Space or click to pick files.".to_owned());
                    return PickerAction::None;
                }
                PickerAction::Done(std::mem::take(&mut self.selected))
            }
            PickerMode::OutputFile => {
                let name = self.filename.trim();
                if name.is_empty() {
                    self.error = Some("Enter a file name first.".to_owned());
                    self.editing_name = true;
                    return PickerAction::None;
                }
                PickerAction::Done(vec![self.dir.join(name)])
            }
            PickerMode::OutputFolder => PickerAction::Done(vec![self.dir.clone()]),
        }
    }

    /// Space: toggle the file under the cursor; directories have nothing to give.
    fn toggle_selected(&mut self) {
        if self.mode != PickerMode::InputFiles {
            return;
        }
        let Some(Row::Entry(entry)) = self.rows.get(self.cursor) else {
            return;
        };
        if entry.is_dir {
            return;
        }
        match self
            .selected
            .iter()
            .position(|path| path.as_os_str() == entry.path.as_os_str())
        {
            Some(index) => {
                self.selected.remove(index);
            }
            None => self.selected.push(entry.path.clone()),
        }
    }
}

/// The first visible row. The wheel offset is clamped so the cursor never leaves
/// the screen: rendering and click handling both call this, so they always agree.
pub fn list_window(scroll: usize, cursor: usize, rows: usize, height: usize) -> usize {
    if height == 0 || rows == 0 {
        return 0;
    }
    let mut window = scroll.min(rows.saturating_sub(1));
    if cursor < window {
        window = cursor;
    } else if cursor >= window.saturating_add(height) {
        window = cursor.saturating_add(1).saturating_sub(height);
    }
    window.min(rows.saturating_sub(1))
}

fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    (!home.is_empty()).then(|| PathBuf::from(home))
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use tempfile::tempdir;

    fn picker(dir: &Path) -> Picker {
        Picker::open(PickerMode::InputFiles, dir.to_owned(), None, false)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn entry_names(picker: &Picker) -> Vec<String> {
        picker
            .rows
            .iter()
            .map(|row| match row {
                Row::Parent => "<..>".to_owned(),
                Row::Entry(entry) => entry.name.clone(),
            })
            .collect()
    }

    #[test]
    fn lists_folders_first_in_case_insensitive_order() {
        let root = tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("z.mp4"), b"").unwrap();
        fs::write(root.path().join("A.mp4"), b"").unwrap();
        fs::write(root.path().join("notes.txt"), b"").unwrap();
        fs::create_dir(root.path().join("movies")).unwrap();
        fs::create_dir(root.path().join("archive")).unwrap();

        let picker = picker(root.path());
        assert_eq!(
            entry_names(&picker),
            vec!["<..>", "archive", "movies", "A.mp4", "notes.txt", "z.mp4"]
        );
    }

    #[test]
    fn hidden_files_are_skipped_until_toggled() {
        let root = tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("clip.mkv"), b"").unwrap();
        fs::write(root.path().join(".system"), b"").unwrap();

        let mut picker = picker(root.path());
        assert!(
            !picker
                .rows
                .iter()
                .any(|row| row_name(row) == Some(".system"))
        );

        picker.show_hidden = true;
        picker.reload();
        assert!(
            picker
                .rows
                .iter()
                .any(|row| row_name(row) == Some(".system"))
        );
    }

    fn row_name(row: &Row) -> Option<&str> {
        match row {
            Row::Entry(entry) => Some(&entry.name),
            _ => None,
        }
    }

    #[test]
    fn enter_navigates_into_directories_and_back_out() {
        let root = tempdir().expect("a temp directory should be created");
        let nested = root.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("b.mp4"), b"").unwrap();

        let mut picker = picker(root.path());
        picker.cursor = 1; // nested, right after the parent row
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerAction::None);
        assert_eq!(picker.dir, nested);

        picker.handle_key(key(KeyCode::Down));
        picker.handle_key(key(KeyCode::Backspace));
        assert_eq!(picker.dir, root.path());
    }

    #[test]
    fn space_toggles_the_multi_selection() {
        let root = tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("a.mp4"), b"").unwrap();
        fs::write(root.path().join("b.mp4"), b"").unwrap();
        fs::create_dir(root.path().join("folder")).unwrap();

        let mut picker = picker(root.path());
        picker.cursor = 2; // a.mp4
        picker.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(picker.selected, vec![root.path().join("a.mp4")]);

        // A directory cannot be added, and Space on it does nothing.
        picker.cursor = 1;
        picker.handle_key(key(KeyCode::Char(' ')));
        assert_eq!(picker.selected, vec![root.path().join("a.mp4")]);

        // Space again un-picks the file.
        picker.cursor = 2;
        picker.handle_key(key(KeyCode::Char(' ')));
        assert!(picker.selected.is_empty());
    }

    #[test]
    fn an_empty_selection_reports_instead_of_closing() {
        let root = tempdir().expect("a temp directory should be created");
        let mut picker = picker(root.path());
        picker.handle_key(key(KeyCode::Char('s')));
        assert!(picker.error.is_some());
        assert!(picker.selected.is_empty());
    }

    #[test]
    fn save_mode_edits_a_name_and_confirms() {
        let root = tempdir().expect("a temp directory should be created");
        let mut picker = Picker::open(
            PickerMode::OutputFile,
            root.path().to_owned(),
            Some("clip".to_owned()),
            false,
        );
        assert_eq!(picker.handle_key(key(KeyCode::Tab)), PickerAction::None);
        assert!(picker.editing_name);

        picker.handle_key(key(KeyCode::Char('z')));
        assert_eq!(picker.filename, "clipz");
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            PickerAction::Done(vec![root.path().join("clipz")])
        );
    }

    #[test]
    fn save_mode_joins_name_with_the_current_directory() {
        let root = tempdir().expect("a temp directory should be created");
        let mut picker = Picker::open(
            PickerMode::OutputFile,
            root.path().to_owned(),
            Some("clip.mkv".to_owned()),
            false,
        );
        assert_eq!(
            picker.handle_key(key(KeyCode::Char('s'))),
            PickerAction::Done(vec![root.path().join("clip.mkv")])
        );
    }

    #[test]
    fn folder_mode_selects_the_current_directory() {
        let root = tempdir().expect("a temp directory should be created");
        let mut picker = Picker::open(
            PickerMode::OutputFolder,
            root.path().to_owned(),
            None,
            false,
        );
        assert_eq!(
            picker.handle_key(key(KeyCode::Char('s'))),
            PickerAction::Done(vec![root.path().to_owned()])
        );
    }

    #[test]
    fn input_folder_mode_lists_directories_and_selects_current_directory() {
        let root = tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("clip.mp4"), b"").unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();

        let mut picker = Picker::open(PickerMode::InputFolder, root.path().to_owned(), None, false);
        assert_eq!(
            entry_names(&picker),
            vec!["<..>", "nested"],
            "input-folder mode should hide files and show directories"
        );
        assert_eq!(
            picker.handle_key(key(KeyCode::Char('s'))),
            PickerAction::Done(vec![root.path().to_owned()])
        );

        picker.cursor = 1;
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), PickerAction::None);
        assert_eq!(picker.dir, root.path().join("nested"));
        assert_eq!(
            picker.handle_key(key(KeyCode::Char('s'))),
            PickerAction::Done(vec![root.path().join("nested")])
        );
    }

    #[test]
    fn enter_on_a_file_selects_just_that_file() {
        let root = tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("a.mp4"), b"").unwrap();
        let mut picker = picker(root.path());
        picker.cursor = 1; // a.mp4
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            PickerAction::Done(vec![root.path().join("a.mp4")])
        );
    }

    #[test]
    fn mouse_click_regression_selects_files_without_toggling_on_double_click() {
        let root = tempdir().expect("a temp directory should be created");
        let folder = root.path().join("folder");
        let file = root.path().join("a.mp4");
        fs::create_dir(&folder).unwrap();
        fs::write(&file, b"").unwrap();
        let mut picker = picker(root.path());
        let list = Rect::new(0, 4, 60, 20);
        let click = |picker: &mut Picker, index: usize| {
            picker.handle_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 2,
                    row: list.y + index as u16,
                    modifiers: KeyModifiers::NONE,
                },
                list,
            )
        };
        let folder_index = picker
            .rows
            .iter()
            .position(|row| matches!(row, Row::Entry(entry) if entry.path == folder))
            .expect("the folder should be listed");
        let file_index = picker
            .rows
            .iter()
            .position(|row| matches!(row, Row::Entry(entry) if entry.path == file))
            .expect("the file should be listed");

        assert_eq!(click(&mut picker, 0), PickerAction::None);
        assert!(picker.selected.is_empty(), "Parent is not a file selection");
        assert_eq!(click(&mut picker, folder_index), PickerAction::None);
        assert!(
            picker.selected.is_empty(),
            "directories are not file selections"
        );

        assert_eq!(click(&mut picker, file_index), PickerAction::None);
        assert_eq!(picker.selected, vec![file.clone()]);
        assert_eq!(
            click(&mut picker, file_index),
            PickerAction::Done(vec![file])
        );
        assert_eq!(picker.selected, vec![root.path().join("a.mp4")]);
    }

    #[test]
    fn escaping_always_cancels() {
        let root = tempdir().expect("a temp directory should be created");
        let mut picker = picker(root.path());
        assert_eq!(picker.handle_key(key(KeyCode::Esc)), PickerAction::Cancel);
    }

    #[test]
    fn window_keeps_the_cursor_visible() {
        assert_eq!(list_window(0, 0, 10, 5), 0);
        assert_eq!(list_window(0, 4, 10, 5), 0);
        // Moving past the bottom pushes the window exactly one row.
        assert_eq!(list_window(0, 5, 10, 5), 1);
        assert_eq!(list_window(4, 9, 10, 5), 5);
        // The scroll offset is honoured only while the cursor stays in view.
        assert_eq!(list_window(2, 7, 10, 5), 3);
        // Moving above the window pulls it straight back up.
        assert_eq!(list_window(3, 1, 10, 5), 1);
    }

    #[test]
    fn click_highlights_the_row_and_double_click_activates() {
        let root = tempdir().expect("a temp directory should be created");
        fs::write(root.path().join("a.mp4"), b"").unwrap();
        let mut picker = picker(root.path());

        // The list starts below the header in a real screen; column and row are
        // absolute, so the handler subtracts the rectangle origin.
        let list = Rect::new(0, 4, 60, 20);
        let click = |kind| MouseEvent {
            kind,
            column: 2,
            row: 5, // list.y + 1 -> a.mp4
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            picker.handle_mouse(click(MouseEventKind::Down(MouseButton::Left)), list),
            PickerAction::None
        );
        assert_eq!(picker.cursor, 1);
        assert_eq!(
            picker.handle_mouse(click(MouseEventKind::Down(MouseButton::Left)), list),
            PickerAction::Done(vec![root.path().join("a.mp4")])
        );
    }

    #[test]
    fn wheel_changes_the_offset_but_not_the_cursor() {
        let root = tempdir().expect("a temp directory should be created");
        let mut picker = picker(root.path());
        let scroll = picker.scroll;
        let cursor = picker.cursor;
        picker.handle_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            Rect::new(0, 0, 60, 20),
        );
        assert_eq!(picker.scroll, scroll + 3);
        assert_eq!(picker.cursor, cursor);
    }
}
