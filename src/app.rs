use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    domain::{
        AUDIO_BITRATE_PRESETS, AudioCodec, Container, DraftConfig, InputMedia, OutputTarget,
        QualityPreset, RateControlMode, Resolution, SizeEstimate, TranscodeConfig,
        VIDEO_BITRATE_PRESETS, estimate_queue_size, file_label, quality_setting,
        supported_audio_codecs, supported_video_codecs,
    },
    media::probe_media,
    picker::{Picker, PickerAction, PickerMode, home_dir},
    toolchain::Toolchain,
    transcode::{
        OutputArtifact, ProgressUpdate, QueuedJob, TranscodeHandle, WorkerEvent,
        build_command_spec, render_command_preview, spawn_transcode_worker,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Folders,
    Settings,
    Confirm,
    Running,
    Result,
    Error,
}

#[derive(Debug, Clone)]
pub enum JobState {
    Idle,
    Probing,
    Ready,
    Starting,
    Running {
        /// Position of the running job in [`App::queue`].
        index: usize,
        pid: u32,
        progress: Option<ProgressUpdate>,
    },
    Cancelling,
    /// The queue stopped. Per-file outcomes live in [`App::queue`].
    Finished {
        elapsed: Duration,
        cancelled: bool,
    },
}

/// One planned conversion and what became of it.
#[derive(Debug, Clone)]
pub struct JobRecord {
    pub input: PathBuf,
    pub output: PathBuf,
    pub outcome: JobOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOutcome {
    Pending,
    Running,
    Succeeded {
        elapsed: Duration,
    },
    Failed(String),
    Cancelled,
    /// Never started, because cancellation ended the queue first.
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigField {
    Input,
    Output,
    Container,
    VideoCodec,
    AudioCodec,
    Resolution,
    RateControl,
    RateValue,
    AudioBitrate,
}

impl ConfigField {
    pub(crate) const SETTINGS: [Self; 7] = [
        Self::Container,
        Self::VideoCodec,
        Self::AudioCodec,
        Self::Resolution,
        Self::RateControl,
        Self::RateValue,
        Self::AudioBitrate,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationButton {
    Back,
    Advance,
}

/// The control currently under the pointer. This state is deliberately kept
/// separate from keyboard focus so moving the mouse never changes the action
/// selected by the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoverTarget {
    HeaderChip(char),
    StatusChip(char),
    InputRow,
    OutputRow,
    Setting(ConfigField),
    CardButton(NavigationButton),
    PickerRow(usize),
    PickerCancel,
    PickerParent,
    PickerPrimary,
    PickerName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCommand {
    None,
    /// Show the input panel. `add` keeps the current selection and appends to it.
    OpenInputs {
        add: bool,
    },
    OpenOutput,
    Quit,
}

#[derive(Debug)]
struct NumericEdit {
    field: ConfigField,
    buffer: String,
}

#[derive(Debug)]
enum AppEvent {
    Probe {
        input: PathBuf,
        result: Result<InputMedia, String>,
    },
    Worker(WorkerEvent),
}

#[derive(Debug)]
pub struct App {
    pub toolchain: Toolchain,
    pub draft: DraftConfig,
    /// Probe result per selected input, keyed by path. An input missing from the map
    /// is still being read.
    pub probes: HashMap<PathBuf, Result<InputMedia, String>>,
    /// A previously used source folder, retained only as a picker start fallback
    /// when no selected input provides a parent directory.
    pub input_folder: Option<PathBuf>,
    pub screen: Screen,
    pub job: JobState,
    /// The planned conversions, filled in when a queue is prepared for confirmation
    /// and updated as it runs.
    pub queue: Vec<JobRecord>,
    pub focus: ConfigField,
    pub button_focus: Option<NavigationButton>,
    pub hover: Option<HoverTarget>,
    pub status_message: Option<String>,
    pub stderr_tail: VecDeque<String>,
    pub review_scroll: u16,
    pub progress_scroll: u16,
    pub progress_follow: bool,
    pub progress_scroll_from_top: bool,
    pub command_preview: Option<String>,
    pub help_visible: bool,
    pub cancel_confirmation: bool,
    pub picker: Option<Picker>,
    numeric_edit: Option<NumericEdit>,
    prepared: Option<Vec<QueuedJob>>,
    transcode_handle: Option<TranscodeHandle>,
    event_tx: Sender<AppEvent>,
    event_rx: Receiver<AppEvent>,
}

impl App {
    pub fn new(toolchain: Toolchain) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            toolchain,
            draft: DraftConfig::default(),
            probes: HashMap::new(),
            input_folder: None,
            screen: Screen::Folders,
            job: JobState::Idle,
            queue: Vec::new(),
            focus: ConfigField::Input,
            button_focus: None,
            hover: None,
            status_message: Some("Select one or more video files to begin.".to_owned()),
            stderr_tail: VecDeque::new(),
            review_scroll: 0,
            progress_scroll: 0,
            progress_follow: true,
            progress_scroll_from_top: false,
            command_preview: None,
            help_visible: false,
            cancel_confirmation: false,
            picker: None,
            numeric_edit: None,
            prepared: None,
            transcode_handle: None,
            event_tx,
            event_rx,
        }
    }

    /// Appends `paths` to the selection, which is how files from several folders end
    /// up in one queue. Paths already selected are ignored.
    pub fn add_inputs(&mut self, paths: Vec<PathBuf>) {
        let mut inputs = self.draft.inputs.clone();
        inputs.extend(paths);
        self.select_inputs(inputs);
    }

    /// Reads every selected input again, keeping the rest of the configuration.
    pub fn reprobe_inputs(&mut self) {
        if self.draft.inputs.is_empty() {
            return;
        }
        self.probes.clear();
        self.invalidate_plan();
        self.job = JobState::Probing;
        self.status_message = Some(probing_message(self.draft.inputs.len()));
        self.spawn_probes(self.draft.inputs.clone());
    }

    pub fn select_output(&mut self, path: PathBuf) {
        self.draft.output = Some(OutputTarget::Directory(path));
        self.invalidate_plan();
        self.status_message = Some("Output folder selected. Press Tab to continue.".to_owned());
    }

    /// The output picker always chooses a directory, even for one input file.
    pub fn output_picker_info(&self) -> (Option<PathBuf>, Option<String>) {
        let directory = self
            .draft
            .output
            .as_ref()
            .filter(|target| target.is_directory())
            .map(|target| target.path().to_owned())
            .or_else(|| self.input_folder.clone())
            .or_else(|| self.first_input_directory())
            .or_else(home_dir);
        (directory, None)
    }

    /// Opens the input-file picker. The first input's parent is the most useful
    /// starting directory when a selection already exists.
    pub fn open_inputs_picker(&mut self, add: bool) {
        let start = self
            .first_input_directory()
            .or_else(|| self.input_folder.clone())
            .or_else(home_dir)
            .or_else(|| std::env::current_dir().ok());
        let Some(start) = start else { return };
        self.hover = None;
        self.picker = Some(Picker::open(PickerMode::InputFiles, start, None, add));
    }

    /// Opens the output-folder picker.
    pub fn open_output_picker(&mut self) {
        let (directory, _) = self.output_picker_info();
        let start = directory.unwrap_or_else(|| PathBuf::from("."));
        self.hover = None;
        self.picker = Some(Picker::open(PickerMode::OutputFolder, start, None, false));
    }

    /// Applies the picker's decision and drops it. An empty input selection is
    /// treated as a dismissal: leaving the picker must never clear the queue.
    pub(crate) fn close_picker(&mut self, action: PickerAction) -> UiCommand {
        let Some(picker) = self.picker.take() else {
            return UiCommand::None;
        };
        match action {
            PickerAction::None => {
                self.picker = Some(picker);
            }
            PickerAction::Cancel => {}
            PickerAction::Done(paths) => match picker.mode {
                PickerMode::InputFiles if !paths.is_empty() => {
                    if picker.append {
                        self.add_inputs(paths);
                    } else {
                        self.select_inputs(paths);
                    }
                }
                PickerMode::OutputFile | PickerMode::OutputFolder => {
                    if let Some(path) = paths.into_iter().next() {
                        self.select_output(path);
                    }
                }
                PickerMode::InputFolder => {}
                PickerMode::InputFiles => {}
            },
        }
        UiCommand::None
    }

    /// Surfaces a failure that happened outside the app state machine.
    pub fn report_error(&mut self, message: String) {
        self.status_message = Some(message);
    }

    pub fn poll_background(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::Probe { input, result } => {
                    // A result for a file that is no longer selected is stale.
                    if self.draft.inputs.contains(&input) {
                        self.handle_probe_result(input, result);
                    }
                }
                AppEvent::Worker(event) => self.handle_worker_event(event),
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> UiCommand {
        if self.picker.is_some() {
            let action = self
                .picker
                .as_mut()
                .map(|picker| picker.handle_key(key))
                .unwrap_or(PickerAction::None);
            return self.close_picker(action);
        }
        if self.help_visible {
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                self.help_visible = false;
            }
            return UiCommand::None;
        }
        if self.numeric_edit.is_some() {
            return self.handle_numeric_key(key);
        }

        match self.screen {
            Screen::Folders => self.handle_folders_key(key),
            Screen::Settings => self.handle_settings_key(key),
            Screen::Confirm => self.handle_confirm_key(key),
            Screen::Running => self.handle_running_key(key),
            Screen::Result | Screen::Error => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.screen = Screen::Folders;
                    self.focus = ConfigField::Input;
                    self.button_focus = None;
                    self.job = self.settled_job_state();
                    self.queue.clear();
                    self.stderr_tail.clear();
                    self.refresh_ready_message();
                    UiCommand::None
                }
                KeyCode::Char('q') => UiCommand::Quit,
                KeyCode::Char('?') => {
                    self.help_visible = true;
                    UiCommand::None
                }
                _ => UiCommand::None,
            },
        }
    }

    pub fn numeric_edit_value(&self) -> Option<(&str, ConfigField)> {
        self.numeric_edit
            .as_ref()
            .map(|edit| (edit.buffer.as_str(), edit.field))
    }

    /// The probed streams of one selected input.
    pub fn media_for(&self, input: &Path) -> Option<&InputMedia> {
        self.probes
            .get(input)
            .and_then(|result| result.as_ref().ok())
    }

    /// Why one selected input could not be read.
    pub fn probe_error_for(&self, input: &Path) -> Option<&str> {
        match self.probes.get(input) {
            Some(Err(error)) => Some(error.as_str()),
            _ => None,
        }
    }

    /// The streams of the only selected input, for the file-shaped source panel.
    pub fn single_media(&self) -> Option<&InputMedia> {
        self.media_for(self.draft.single_input()?)
    }

    /// Every input that was read successfully, in selection order.
    pub fn probed_media(&self) -> impl Iterator<Item = &InputMedia> {
        self.draft
            .inputs
            .iter()
            .filter_map(|input| self.media_for(input))
    }

    pub fn probed_count(&self) -> usize {
        self.probed_media().count()
    }

    pub fn failed_probe_count(&self) -> usize {
        self.draft
            .inputs
            .iter()
            .filter(|input| self.probe_error_for(input).is_some())
            .count()
    }

    fn all_probed(&self) -> bool {
        self.draft
            .inputs
            .iter()
            .all(|input| self.probes.contains_key(input))
    }

    pub fn current_validation_error(&self) -> Option<String> {
        if self.draft.inputs.is_empty() || !self.all_probed() {
            return None;
        }
        self.validated_configs().err()
    }

    fn validated_configs(&self) -> Result<Vec<TranscodeConfig>, String> {
        if !self.toolchain.supports_video(self.draft.video_codec) {
            return Err(format!(
                "{} is not available in this FFmpeg build.",
                self.draft.video_codec.encoder()
            ));
        }
        if !self.toolchain.supports_audio(self.draft.audio_codec) {
            return Err(format!(
                "{} is not available in this FFmpeg build.",
                self.draft.audio_codec.encoder().unwrap_or("audio encoder")
            ));
        }
        // A file that could not be read cannot be converted, so it blocks the queue
        // instead of being silently dropped from it.
        for input in &self.draft.inputs {
            if let Some(error) = self.probe_error_for(input) {
                return Err(format!("{}: {error}", file_label(input)));
            }
        }
        self.draft
            .validated_queue(&self.sources())
            .map_err(|error| error.to_string())
    }

    pub fn quality_label(&self) -> String {
        format!(
            "{} ({})",
            self.draft.quality,
            quality_setting(self.draft.video_codec, self.draft.quality)
        )
    }

    /// The predicted total size of the files the current draft would produce. `None`
    /// until at least one source has been probed with a usable duration.
    pub fn size_estimate(&self) -> Option<SizeEstimate> {
        estimate_queue_size(&self.draft, self.probed_media())
    }

    pub fn succeeded_count(&self) -> usize {
        self.queue
            .iter()
            .filter(|record| matches!(record.outcome, JobOutcome::Succeeded { .. }))
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.queue
            .iter()
            .filter(|record| matches!(record.outcome, JobOutcome::Failed(_)))
            .count()
    }

    pub fn request_cancel(&mut self) {
        if let Some(handle) = &self.transcode_handle {
            handle.cancel();
            self.job = JobState::Cancelling;
            self.cancel_confirmation = false;
            self.status_message = Some(if self.queue.len() > 1 {
                "Cancelling FFmpeg and stopping the queue…".to_owned()
            } else {
                "Cancelling FFmpeg…".to_owned()
            });
        }
    }

    /// Replaces the selection with `paths`, ignoring duplicates.
    pub fn select_inputs(&mut self, paths: Vec<PathBuf>) {
        let mut selected = HashSet::with_capacity(paths.len());
        self.draft.inputs = paths
            .into_iter()
            .filter(|path| selected.insert(path.clone()))
            .collect();
        self.invalidate_plan();
        // Keep what is already known: re-selecting a file that was probed a moment
        // ago should not read it again.
        self.probes.retain(|path, _| selected.contains(path));
        self.refresh_output_target();

        let pending: Vec<PathBuf> = self
            .draft
            .inputs
            .iter()
            .filter(|input| !self.probes.contains_key(*input))
            .cloned()
            .collect();
        if self.draft.inputs.is_empty() {
            self.job = JobState::Idle;
            self.status_message = Some("Select one or more video files to begin.".to_owned());
            return;
        }
        if pending.is_empty() {
            self.job = JobState::Ready;
            self.refresh_ready_message();
            return;
        }
        self.job = JobState::Probing;
        self.status_message = Some(probing_message(pending.len()));
        self.spawn_probes(pending);
    }

    /// Every input writes a matching file into one destination directory.
    fn refresh_output_target(&mut self) {
        self.draft.output = match self.draft.inputs.as_slice() {
            [] => None,
            [first, ..] => match self.draft.output.take() {
                Some(target) if target.is_directory() => Some(target),
                _ => Some(OutputTarget::Directory(
                    first
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .map(ToOwned::to_owned)
                        .or_else(|| self.input_folder.clone())
                        .unwrap_or_else(|| PathBuf::from(".")),
                )),
            },
        };
    }

    fn first_input_directory(&self) -> Option<PathBuf> {
        self.draft
            .inputs
            .first()
            .and_then(|input| input.parent())
            .map(|parent| {
                if parent.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    parent.to_owned()
                }
            })
    }

    /// Reads the queued files one after another. ffprobe is cheap, but a folder full
    /// of media would otherwise start a process per file at once.
    fn spawn_probes(&self, inputs: Vec<PathBuf>) {
        let ffprobe = self.toolchain.ffprobe.clone();
        let event_tx = self.event_tx.clone();
        thread::spawn(move || {
            for input in inputs {
                let result = probe_media(&ffprobe, &input).map_err(|error| error.to_string());
                if event_tx.send(AppEvent::Probe { input, result }).is_err() {
                    return;
                }
            }
        });
    }

    /// The probed sources, in queue order. Only complete when every input was read.
    fn sources(&self) -> Vec<(&Path, &InputMedia)> {
        self.draft
            .inputs
            .iter()
            .filter_map(|input| Some((input.as_path(), self.media_for(input)?)))
            .collect()
    }

    fn handle_probe_result(&mut self, input: PathBuf, result: Result<InputMedia, String>) {
        self.probes.insert(input, result);
        if matches!(self.job, JobState::Probing) && self.all_probed() {
            self.job = JobState::Ready;
        }
        if matches!(self.job, JobState::Probing) {
            self.status_message = Some(probing_message(
                self.draft.inputs.len().saturating_sub(self.probes.len()),
            ));
        } else {
            self.refresh_ready_message();
        }
    }

    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Started { index, pid } => {
                // Messages belong to the file that produced them.
                self.stderr_tail.clear();
                self.progress_scroll = 0;
                self.progress_follow = true;
                self.progress_scroll_from_top = false;
                self.set_outcome(index, JobOutcome::Running);
                if !matches!(self.job, JobState::Cancelling) {
                    self.job = JobState::Running {
                        index,
                        pid,
                        progress: None,
                    };
                }
                self.status_message = Some(match self.queue.get(index) {
                    Some(record) if self.queue.len() > 1 => format!(
                        "Converting {} of {}: {}",
                        index + 1,
                        self.queue.len(),
                        file_label(&record.input)
                    ),
                    _ => "FFmpeg is running.".to_owned(),
                });
            }
            WorkerEvent::Progress { index, update } => {
                if !matches!(self.job, JobState::Cancelling) {
                    let pid = match self.job {
                        JobState::Running { pid, .. } => pid,
                        _ => 0,
                    };
                    self.job = JobState::Running {
                        index,
                        pid,
                        progress: Some(update),
                    };
                }
            }
            WorkerEvent::StderrLine { line, .. } => {
                self.stderr_tail.push_back(line);
            }
            WorkerEvent::Finished {
                index,
                output,
                elapsed,
            } => {
                if let Some(record) = self.queue.get_mut(index) {
                    record.output = output;
                    record.outcome = JobOutcome::Succeeded { elapsed };
                }
            }
            WorkerEvent::Failed { index, error } => {
                self.set_outcome(index, JobOutcome::Failed(error));
            }
            WorkerEvent::Cancelled { index } => {
                self.set_outcome(index, JobOutcome::Cancelled);
            }
            WorkerEvent::QueueFinished {
                elapsed,
                cancelled,
                remaining: _,
            } => self.finish_queue(elapsed, cancelled),
        }
    }

    fn set_outcome(&mut self, index: usize, outcome: JobOutcome) {
        if let Some(record) = self.queue.get_mut(index) {
            record.outcome = outcome;
        }
    }

    fn finish_queue(&mut self, elapsed: Duration, cancelled: bool) {
        self.transcode_handle = None;
        for record in &mut self.queue {
            if matches!(record.outcome, JobOutcome::Pending | JobOutcome::Running) {
                record.outcome = JobOutcome::Skipped;
            }
        }
        let succeeded = self.succeeded_count();
        let failed = self.failed_count();
        self.job = JobState::Finished { elapsed, cancelled };
        self.status_message = Some(if cancelled {
            match succeeded {
                0 => "Conversion cancelled. Temporary output removed.".to_owned(),
                converted => format!(
                    "Cancelled after converting {converted} of {}. Temporary output removed.",
                    self.queue.len()
                ),
            }
        } else if failed == 0 {
            match self.queue.len() {
                1 => "Conversion completed successfully.".to_owned(),
                total => format!("All {total} files converted successfully."),
            }
        } else {
            format!("{succeeded} of {} files converted.", self.queue.len())
        });
        // Cancelling is a choice, not an error, so only a run that produced nothing
        // it was asked for lands on the error screen.
        self.screen = if succeeded > 0 || cancelled {
            Screen::Result
        } else {
            Screen::Error
        };
    }

    fn handle_folders_key(&mut self, key: KeyEvent) -> UiCommand {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return UiCommand::Quit;
        }
        match key.code {
            KeyCode::Char('q') => UiCommand::Quit,
            KeyCode::Char('?') => {
                self.help_visible = true;
                UiCommand::None
            }
            KeyCode::Char('i') => UiCommand::OpenInputs { add: false },
            KeyCode::Char('a') => UiCommand::OpenInputs { add: true },
            KeyCode::Char('c') => {
                self.select_inputs(Vec::new());
                self.input_folder = None;
                UiCommand::None
            }
            KeyCode::Char('o') => UiCommand::OpenOutput,
            KeyCode::Char('r') => UiCommand::OpenInputs { add: false },
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => {
                self.move_folder_focus(1);
                UiCommand::None
            }
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => {
                self.move_folder_focus(-1);
                UiCommand::None
            }
            KeyCode::Enter => match self.button_focus {
                Some(NavigationButton::Advance) => {
                    self.leave_folders();
                    UiCommand::None
                }
                _ => match self.focus {
                    ConfigField::Input => UiCommand::OpenInputs { add: false },
                    ConfigField::Output => UiCommand::OpenOutput,
                    _ => UiCommand::None,
                },
            },
            _ => UiCommand::None,
        }
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> UiCommand {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return UiCommand::Quit;
        }
        match key.code {
            KeyCode::Char('q') => UiCommand::Quit,
            KeyCode::Char('?') => {
                self.help_visible = true;
                UiCommand::None
            }
            KeyCode::Char('r') => {
                self.reprobe_inputs();
                UiCommand::None
            }
            KeyCode::Esc => {
                self.screen = Screen::Folders;
                self.focus = ConfigField::Output;
                self.button_focus = None;
                UiCommand::None
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => {
                self.move_settings_focus(1);
                UiCommand::None
            }
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => {
                self.move_settings_focus(-1);
                UiCommand::None
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.button_focus.is_none() {
                    self.adjust_focused(-1);
                }
                UiCommand::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.button_focus.is_none() {
                    self.adjust_focused(1);
                }
                UiCommand::None
            }
            KeyCode::Enter => match self.button_focus {
                Some(NavigationButton::Back) => {
                    self.screen = Screen::Folders;
                    self.focus = ConfigField::Output;
                    self.button_focus = None;
                    UiCommand::None
                }
                Some(NavigationButton::Advance) => {
                    self.prepare_confirmation();
                    UiCommand::None
                }
                None => match self.focus {
                    ConfigField::RateValue
                        if self.draft.rate_control_mode == RateControlMode::Bitrate =>
                    {
                        self.begin_numeric_edit();
                        UiCommand::None
                    }
                    ConfigField::AudioBitrate if self.audio_bitrate_enabled() => {
                        self.begin_numeric_edit();
                        UiCommand::None
                    }
                    _ => UiCommand::None,
                },
            },
            _ => UiCommand::None,
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> UiCommand {
        match key.code {
            KeyCode::Up => {
                self.scroll_review_up(1);
                UiCommand::None
            }
            KeyCode::Down => {
                self.review_scroll = self.review_scroll.saturating_add(1);
                UiCommand::None
            }
            KeyCode::PageUp => {
                self.scroll_review_up(8);
                UiCommand::None
            }
            KeyCode::PageDown => {
                self.review_scroll = self.review_scroll.saturating_add(8);
                UiCommand::None
            }
            KeyCode::Home => {
                self.review_scroll = 0;
                UiCommand::None
            }
            KeyCode::End => {
                self.review_scroll = u16::MAX;
                UiCommand::None
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                self.start_prepared();
                UiCommand::None
            }
            KeyCode::Esc | KeyCode::Char('n') => {
                self.invalidate_plan();
                self.screen = Screen::Settings;
                self.button_focus = Some(NavigationButton::Advance);
                UiCommand::None
            }
            KeyCode::Char('q') => UiCommand::Quit,
            _ => UiCommand::None,
        }
    }

    fn handle_running_key(&mut self, key: KeyEvent) -> UiCommand {
        if self.cancel_confirmation {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => self.request_cancel(),
                KeyCode::Char('n') | KeyCode::Esc => self.cancel_confirmation = false,
                _ => {}
            }
            return UiCommand::None;
        }
        match key.code {
            KeyCode::Up => self.scroll_progress_up(1),
            KeyCode::Down => self.scroll_progress_down(1),
            KeyCode::PageUp => self.scroll_progress_up(8),
            KeyCode::PageDown => self.scroll_progress_down(8),
            KeyCode::Home => {
                self.progress_scroll = 0;
                self.progress_follow = false;
                self.progress_scroll_from_top = true;
            }
            KeyCode::End => {
                self.progress_scroll = 0;
                self.progress_follow = true;
                self.progress_scroll_from_top = false;
            }
            _ => {}
        }
        if (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'))
            || matches!(key.code, KeyCode::Char('x' | 'q') | KeyCode::Esc)
        {
            self.cancel_confirmation = true;
        }
        UiCommand::None
    }

    pub fn scroll_review_up(&mut self, amount: u16) {
        self.review_scroll = self.review_scroll.saturating_sub(amount);
    }

    pub fn scroll_progress_up(&mut self, amount: u16) {
        self.progress_scroll = if self.progress_follow {
            amount
        } else if self.progress_scroll_from_top {
            self.progress_scroll.saturating_sub(amount)
        } else {
            self.progress_scroll.saturating_add(amount)
        };
        self.progress_follow = false;
    }

    pub fn scroll_progress_down(&mut self, amount: u16) {
        if self.progress_scroll_from_top {
            self.progress_scroll = self.progress_scroll.saturating_add(amount);
        } else {
            self.progress_scroll = self.progress_scroll.saturating_sub(amount);
        }
        if !self.progress_scroll_from_top && self.progress_scroll == 0 {
            self.progress_follow = true;
        }
    }

    fn handle_numeric_key(&mut self, key: KeyEvent) -> UiCommand {
        match key.code {
            KeyCode::Char(character) if character.is_ascii_digit() => {
                if let Some(edit) = self.numeric_edit.as_mut() {
                    if edit.buffer.len() < 6 {
                        edit.buffer.push(character);
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(edit) = self.numeric_edit.as_mut() {
                    edit.buffer.pop();
                }
            }
            KeyCode::Esc => self.numeric_edit = None,
            KeyCode::Enter => self.commit_numeric_edit(),
            _ => {}
        }
        UiCommand::None
    }

    fn begin_numeric_edit(&mut self) {
        let value = match self.focus {
            ConfigField::RateValue if self.draft.rate_control_mode == RateControlMode::Bitrate => {
                self.draft.video_bitrate_kbps
            }
            ConfigField::AudioBitrate if self.audio_bitrate_enabled() => {
                self.draft.audio_bitrate_kbps
            }
            _ => return,
        };
        self.numeric_edit = Some(NumericEdit {
            field: self.focus,
            buffer: value.to_string(),
        });
    }

    fn commit_numeric_edit(&mut self) {
        let Some(edit) = self.numeric_edit.take() else {
            return;
        };
        let Ok(value) = edit.buffer.parse::<u32>() else {
            self.status_message = Some("Enter a valid whole-number bitrate.".to_owned());
            return;
        };
        match edit.field {
            ConfigField::RateValue if (100..=200_000).contains(&value) => {
                self.draft.video_bitrate_kbps = value;
                self.status_message = Some(format!("Video bitrate set to {value} kbps."));
            }
            ConfigField::AudioBitrate if (32..=512).contains(&value) => {
                self.draft.audio_bitrate_kbps = value;
                self.status_message = Some(format!("Audio bitrate set to {value} kbps."));
            }
            ConfigField::RateValue => {
                self.status_message = Some("Video bitrate must be 100–200000 kbps.".to_owned());
            }
            ConfigField::AudioBitrate => {
                self.status_message = Some("Audio bitrate must be 32–512 kbps.".to_owned());
            }
            _ => {}
        }
    }

    fn move_folder_focus(&mut self, direction: i32) {
        let current = match self.button_focus {
            Some(NavigationButton::Advance) => 2,
            _ if self.focus == ConfigField::Output => 1,
            _ => 0,
        };
        let next = (current + direction).rem_euclid(3);
        match next {
            0 => {
                self.focus = ConfigField::Input;
                self.button_focus = None;
            }
            1 => {
                self.focus = ConfigField::Output;
                self.button_focus = None;
            }
            _ => self.button_focus = Some(NavigationButton::Advance),
        }
    }

    fn move_settings_focus(&mut self, direction: i32) {
        let count = ConfigField::SETTINGS.len() + 2;
        let current = match self.button_focus {
            Some(NavigationButton::Back) => ConfigField::SETTINGS.len(),
            Some(NavigationButton::Advance) => ConfigField::SETTINGS.len() + 1,
            None => ConfigField::SETTINGS
                .iter()
                .position(|field| *field == self.focus)
                .unwrap_or(0),
        };
        for offset in 1..=count {
            let index =
                (current as i32 + direction * offset as i32).rem_euclid(count as i32) as usize;
            if index < ConfigField::SETTINGS.len() {
                let candidate = ConfigField::SETTINGS[index];
                if self.field_enabled(candidate) {
                    self.focus = candidate;
                    self.button_focus = None;
                    return;
                }
            } else if index == ConfigField::SETTINGS.len() {
                self.button_focus = Some(NavigationButton::Back);
                return;
            } else {
                self.button_focus = Some(NavigationButton::Advance);
                return;
            }
        }
    }

    pub(crate) fn leave_folders(&mut self) {
        if self.draft.inputs.is_empty() {
            self.status_message = Some("Select one or more video files first.".to_owned());
            return;
        }
        if !matches!(self.draft.output, Some(OutputTarget::Directory(_))) {
            self.status_message = Some("Select an output folder first.".to_owned());
            return;
        }
        self.screen = Screen::Settings;
        self.focus = ConfigField::Container;
        self.button_focus = None;
        self.status_message = Some("Choose encoding settings, then review.".to_owned());
    }

    fn field_enabled(&self, field: ConfigField) -> bool {
        field != ConfigField::AudioBitrate || self.audio_bitrate_enabled()
    }

    /// True while any queued source carries audio the encoder would have to write.
    pub fn audio_bitrate_enabled(&self) -> bool {
        self.draft.audio_codec != AudioCodec::None
            && (self.probed_count() == 0 || self.probed_media().any(|media| media.audio.is_some()))
    }

    /// True once every queued source is known to be silent.
    pub fn all_sources_silent(&self) -> bool {
        self.probed_count() > 0 && self.probed_media().all(|media| media.audio.is_none())
    }

    fn adjust_focused(&mut self, direction: i32) {
        match self.focus {
            ConfigField::Container => {
                self.draft.container = cycle(&Container::ALL, self.draft.container, direction);
                self.draft.normalize_for_container();
            }
            ConfigField::VideoCodec => {
                let available: Vec<_> = supported_video_codecs(self.draft.container)
                    .iter()
                    .copied()
                    .filter(|codec| self.toolchain.supports_video(*codec))
                    .collect();
                if !available.is_empty() {
                    self.draft.video_codec = cycle(&available, self.draft.video_codec, direction);
                }
            }
            ConfigField::AudioCodec => {
                let available: Vec<_> = supported_audio_codecs(self.draft.container)
                    .iter()
                    .copied()
                    .filter(|codec| self.toolchain.supports_audio(*codec))
                    .collect();
                if !available.is_empty() {
                    self.draft.audio_codec = cycle(&available, self.draft.audio_codec, direction);
                }
            }
            ConfigField::Resolution => {
                self.draft.resolution = cycle(&Resolution::ALL, self.draft.resolution, direction);
            }
            ConfigField::RateControl => {
                self.draft.rate_control_mode = match self.draft.rate_control_mode {
                    RateControlMode::Quality => RateControlMode::Bitrate,
                    RateControlMode::Bitrate => RateControlMode::Quality,
                };
            }
            ConfigField::RateValue => match self.draft.rate_control_mode {
                RateControlMode::Quality => {
                    self.draft.quality = cycle(&QualityPreset::ALL, self.draft.quality, direction);
                }
                RateControlMode::Bitrate => {
                    self.draft.video_bitrate_kbps = cycle_numeric(
                        VIDEO_BITRATE_PRESETS,
                        self.draft.video_bitrate_kbps,
                        direction,
                    );
                }
            },
            ConfigField::AudioBitrate if self.audio_bitrate_enabled() => {
                self.draft.audio_bitrate_kbps = cycle_numeric(
                    AUDIO_BITRATE_PRESETS,
                    self.draft.audio_bitrate_kbps,
                    direction,
                );
            }
            ConfigField::Input | ConfigField::Output | ConfigField::AudioBitrate => {}
        }
        self.invalidate_plan();
        self.refresh_ready_message();
    }

    /// Anything that changes the settings or the selection invalidates a plan built
    /// from the previous ones, including the artifacts it reserved.
    fn invalidate_plan(&mut self) {
        self.prepared = None;
        self.command_preview = None;
        self.queue.clear();
    }

    pub(crate) fn prepare_confirmation(&mut self) {
        if self.draft.inputs.is_empty() {
            self.status_message = Some("Select one or more video files first.".to_owned());
            return;
        }
        if !matches!(self.draft.output, Some(OutputTarget::Directory(_))) {
            self.status_message = Some("Select an output folder first.".to_owned());
            return;
        }
        if !self.all_probed() {
            self.status_message = Some("Still reading the selected files…".to_owned());
            return;
        }
        let configs = match self.validated_configs() {
            Ok(configs) => configs,
            Err(error) => {
                self.status_message = Some(error);
                return;
            }
        };

        let mut jobs = Vec::with_capacity(configs.len());
        let mut records = Vec::with_capacity(configs.len());
        let mut preview = None;
        for config in configs {
            let artifact = match OutputArtifact::reserve(config.output.clone()) {
                Ok(artifact) => artifact,
                Err(error) => {
                    // Dropping the jobs built so far releases the directories they
                    // reserved, so a rejected plan leaves nothing behind.
                    self.status_message = Some(format!("{}: {error}", file_label(&config.input)));
                    return;
                }
            };
            let Some(media) = self.media_for(&config.input) else {
                continue;
            };
            let spec = build_command_spec(&self.toolchain.ffmpeg, &config, media, &artifact);
            preview.get_or_insert_with(|| render_command_preview(&spec));
            records.push(JobRecord {
                input: config.input.clone(),
                output: config.output.clone(),
                outcome: JobOutcome::Pending,
            });
            jobs.push(QueuedJob {
                spec,
                artifact,
                duration: media.duration,
            });
        }

        if jobs.is_empty() {
            self.status_message = Some("Nothing to convert in the current selection.".to_owned());
            return;
        }
        self.command_preview = preview;
        self.queue = records;
        self.prepared = Some(jobs);
        self.review_scroll = 0;
        self.screen = Screen::Confirm;
        self.button_focus = Some(NavigationButton::Advance);
    }

    fn start_prepared(&mut self) {
        let Some(jobs) = self.prepared.take() else {
            self.screen = Screen::Settings;
            return;
        };
        self.stderr_tail.clear();
        self.progress_scroll = 0;
        self.progress_follow = true;
        self.progress_scroll_from_top = false;
        self.screen = Screen::Running;
        self.job = JobState::Starting;
        self.status_message = Some(match self.queue.as_slice() {
            [record] => format!(
                "Starting {} → {}…",
                record.input.display(),
                record.output.display()
            ),
            records => format!("Starting a queue of {} files…", records.len()),
        });
        let worker_tx = self.event_tx.clone();
        let (event_tx, event_rx) = mpsc::channel();
        thread::spawn(move || {
            for event in event_rx {
                let _ = worker_tx.send(AppEvent::Worker(event));
            }
        });
        self.transcode_handle = Some(spawn_transcode_worker(jobs, event_tx));
    }

    /// The state the configure screen returns to once a run is over.
    fn settled_job_state(&self) -> JobState {
        if self.draft.inputs.is_empty() {
            JobState::Idle
        } else if self.all_probed() {
            JobState::Ready
        } else {
            JobState::Probing
        }
    }

    fn refresh_ready_message(&mut self) {
        if self.draft.inputs.is_empty() || !self.all_probed() {
            return;
        }
        self.status_message = self.current_validation_error().or_else(|| {
            Some(match self.draft.inputs.len() {
                1 => "Configuration is ready. Press Enter to review.".to_owned(),
                count => format!("{count} files queued. Press Enter to review."),
            })
        });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(handle) = &self.transcode_handle {
            handle.cancel();
        }
    }
}

fn probing_message(remaining: usize) -> String {
    match remaining {
        0 | 1 => "Probing input media…".to_owned(),
        count => format!("Probing input media… {count} files left."),
    }
}

fn cycle<T: Copy + PartialEq>(values: &[T], current: T, direction: i32) -> T {
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    let next = (index as i32 + direction).rem_euclid(values.len() as i32) as usize;
    values[next]
}

/// Steps through the presets from the one nearest the current value, so a bitrate
/// typed by hand still lands somewhere sensible. An exact match has distance zero, so
/// it always wins.
fn cycle_numeric(values: &[u32], current: u32, direction: i32) -> u32 {
    let nearest = values
        .iter()
        .copied()
        .min_by_key(|value| value.abs_diff(current))
        .unwrap_or(current);
    cycle(values, nearest, direction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::VideoStreamInfo;

    fn media() -> InputMedia {
        InputMedia {
            duration: Some(Duration::from_secs(10)),
            video: VideoStreamInfo {
                codec: "h264".to_owned(),
                width: 1920,
                height: 1080,
                frame_rate: Some(30.0),
                bitrate_kbps: Some(8_000),
            },
            audio: Some("aac".to_owned()),
            bitrate_kbps: Some(8_192),
        }
    }

    #[test]
    fn cycles_custom_bitrate_to_nearest_preset() {
        assert_eq!(cycle_numeric(VIDEO_BITRATE_PRESETS, 4_900, 1), 8_000);
        assert_eq!(cycle_numeric(VIDEO_BITRATE_PRESETS, 4_900, -1), 2_500);
    }

    #[test]
    fn ignores_probe_results_for_a_previous_input() {
        let mut app = App::new(Toolchain::test_fixture());
        app.draft.inputs = vec![PathBuf::from("current.mp4")];
        app.event_tx
            .send(AppEvent::Probe {
                input: PathBuf::from("previous.mp4"),
                result: Err("stale error".to_owned()),
            })
            .unwrap();

        app.poll_background();

        assert!(app.probes.is_empty());
        assert!(matches!(app.job, JobState::Idle));
        assert_ne!(app.status_message.as_deref(), Some("stale error"));
    }

    #[test]
    fn wraps_enum_selection() {
        assert_eq!(cycle(&Container::ALL, Container::Mp4, -1), Container::WebM);
    }

    #[test]
    fn opening_the_input_picker_lands_in_the_last_held_folder() {
        let mut app = App::new(Toolchain::test_fixture());
        app.open_inputs_picker(false);
        let picker = app.picker.as_ref().expect("the picker should open");
        assert_eq!(picker.mode, PickerMode::InputFiles);
        assert!(!picker.rows.is_empty());
    }

    #[test]
    fn input_picker_uses_first_input_parent_before_the_saved_folder() {
        let first = tempfile::tempdir().unwrap();
        let saved = tempfile::tempdir().unwrap();
        let input = first.path().join("clip.mov");
        std::fs::write(&input, b"").unwrap();

        let mut app = App::new(Toolchain::test_fixture());
        app.input_folder = Some(saved.path().to_owned());
        app.draft.inputs = vec![input.clone()];
        app.open_inputs_picker(false);

        let picker = app.picker.as_ref().expect("the picker should open");
        assert_eq!(picker.mode, PickerMode::InputFiles);
        assert_eq!(picker.dir, first.path());
    }

    #[test]
    fn input_picker_replaces_or_appends_concrete_files() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first.mov");
        let second = root.path().join("second.mov");
        let third = root.path().join("third.mov");
        for path in [&first, &second, &third] {
            std::fs::write(path, b"").unwrap();
        }

        let mut app = App::new(Toolchain::test_fixture());
        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            false,
        ));
        app.close_picker(PickerAction::Done(vec![first.clone(), second.clone()]));
        assert_eq!(app.draft.inputs, vec![first.clone(), second.clone()]);

        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            true,
        ));
        app.close_picker(PickerAction::Done(vec![third.clone(), second.clone()]));
        assert_eq!(app.draft.inputs, vec![first, second, third]);
    }

    #[test]
    fn opening_the_input_picker_preserves_the_add_flag() {
        let mut app = App::new(Toolchain::test_fixture());

        app.open_inputs_picker(false);
        assert!(!app.picker.as_ref().unwrap().append);

        app.open_inputs_picker(true);
        assert!(app.picker.as_ref().unwrap().append);
    }

    /// Every input writes a derived file into the selected destination folder.
    #[test]
    fn output_target_follows_the_size_of_the_selection() {
        let mut app = App::new(Toolchain::test_fixture());
        app.select_inputs(vec![PathBuf::from("/media/clips/a.mov")]);
        assert_eq!(
            app.draft.output,
            Some(OutputTarget::Directory(PathBuf::from("/media/clips")))
        );

        app.add_inputs(vec![PathBuf::from("/media/clips/b.mov")]);
        assert_eq!(
            app.draft.output,
            Some(OutputTarget::Directory(PathBuf::from("/media/clips")))
        );
        assert_eq!(
            app.draft.output_path_for(Path::new("/media/clips/b.mov")),
            Some(PathBuf::from("/media/clips/b-transcode.mp4"))
        );

        // A folder the user chose survives further additions.
        app.select_output(PathBuf::from("/exports"));
        app.add_inputs(vec![PathBuf::from("/media/clips/c.mov")]);
        assert_eq!(
            app.draft.output,
            Some(OutputTarget::Directory(PathBuf::from("/exports")))
        );
        assert_eq!(app.draft.inputs.len(), 3);
    }

    #[test]
    fn selecting_the_same_file_twice_queues_it_once() {
        let mut app = App::new(Toolchain::test_fixture());
        app.select_inputs(vec![
            PathBuf::from("/media/a.mov"),
            PathBuf::from("/media/a.mov"),
            PathBuf::from("/media/b.mov"),
        ]);
        assert_eq!(app.draft.inputs.len(), 2);

        app.add_inputs(vec![PathBuf::from("/media/b.mov")]);
        assert_eq!(app.draft.inputs.len(), 2);
    }

    #[test]
    fn probing_finishes_only_when_every_input_has_been_read() {
        let mut app = App::new(Toolchain::test_fixture());
        app.draft.inputs = vec![PathBuf::from("a.mp4"), PathBuf::from("b.mp4")];
        app.job = JobState::Probing;

        app.handle_probe_result(PathBuf::from("a.mp4"), Ok(media()));
        assert!(matches!(app.job, JobState::Probing));

        app.handle_probe_result(PathBuf::from("b.mp4"), Err("unreadable".to_owned()));
        assert!(matches!(app.job, JobState::Ready));
        assert_eq!(app.probed_count(), 1);
        assert_eq!(app.failed_probe_count(), 1);
        // A file that could not be read blocks the queue instead of being dropped.
        assert_eq!(
            app.current_validation_error().as_deref(),
            Some("b.mp4: unreadable")
        );
    }

    #[test]
    fn the_estimate_covers_the_whole_queue() {
        let mut app = App::new(Toolchain::test_fixture());
        app.draft.inputs = vec![PathBuf::from("a.mp4"), PathBuf::from("b.mp4")];
        app.draft.rate_control_mode = RateControlMode::Bitrate;
        app.probes.insert(PathBuf::from("a.mp4"), Ok(media()));
        let single = app.size_estimate().expect("one probed source estimates");

        app.probes.insert(PathBuf::from("b.mp4"), Ok(media()));
        let both = app.size_estimate().expect("two probed sources estimate");

        assert_eq!(both.bytes, single.bytes * 2);
    }

    #[test]
    fn a_cancelled_queue_marks_the_files_it_never_reached() {
        let mut app = App::new(Toolchain::test_fixture());
        app.queue = ["a", "b", "c"]
            .into_iter()
            .map(|name| JobRecord {
                input: PathBuf::from(format!("{name}.mov")),
                output: PathBuf::from(format!("{name}.mp4")),
                outcome: JobOutcome::Pending,
            })
            .collect();
        app.screen = Screen::Running;

        app.handle_worker_event(WorkerEvent::Finished {
            index: 0,
            output: PathBuf::from("a.mp4"),
            elapsed: Duration::from_secs(2),
        });
        app.handle_worker_event(WorkerEvent::Cancelled { index: 1 });
        app.handle_worker_event(WorkerEvent::QueueFinished {
            elapsed: Duration::from_secs(3),
            cancelled: true,
            remaining: 1,
        });

        assert_eq!(app.succeeded_count(), 1);
        assert_eq!(app.queue[1].outcome, JobOutcome::Cancelled);
        assert_eq!(app.queue[2].outcome, JobOutcome::Skipped);
        // Work was completed, so a cancelled queue is a result, not an error.
        assert_eq!(app.screen, Screen::Result);
    }

    /// One file failing must not take the rest of the queue with it.
    #[test]
    fn a_failed_file_leaves_the_queue_on_the_result_screen() {
        let mut app = App::new(Toolchain::test_fixture());
        app.queue = ["a", "b"]
            .into_iter()
            .map(|name| JobRecord {
                input: PathBuf::from(format!("{name}.mov")),
                output: PathBuf::from(format!("{name}.mp4")),
                outcome: JobOutcome::Pending,
            })
            .collect();
        app.screen = Screen::Running;

        app.handle_worker_event(WorkerEvent::Failed {
            index: 0,
            error: "FFmpeg failed with exit code 1.".to_owned(),
        });
        app.handle_worker_event(WorkerEvent::Finished {
            index: 1,
            output: PathBuf::from("b.mp4"),
            elapsed: Duration::from_secs(1),
        });
        app.handle_worker_event(WorkerEvent::QueueFinished {
            elapsed: Duration::from_secs(4),
            cancelled: false,
            remaining: 0,
        });

        assert_eq!(app.failed_count(), 1);
        assert_eq!(app.succeeded_count(), 1);
        assert_eq!(app.screen, Screen::Result);
        assert_eq!(
            app.status_message.as_deref(),
            Some("1 of 2 files converted.")
        );
    }

    /// A lone file that fails is still an error, exactly as before.
    #[test]
    fn a_single_failure_lands_on_the_error_screen() {
        let mut app = App::new(Toolchain::test_fixture());
        app.queue = vec![JobRecord {
            input: PathBuf::from("a.mov"),
            output: PathBuf::from("a.mp4"),
            outcome: JobOutcome::Pending,
        }];
        app.screen = Screen::Running;

        app.handle_worker_event(WorkerEvent::Failed {
            index: 0,
            error: "Encoder failed.".to_owned(),
        });
        app.handle_worker_event(WorkerEvent::QueueFinished {
            elapsed: Duration::from_secs(1),
            cancelled: false,
            remaining: 0,
        });

        assert_eq!(app.screen, Screen::Error);
    }

    /// Closing the input picker selects concrete files, and a dismissal changes nothing.
    #[test]
    fn picker_results_flow_into_input_files() {
        let mut app = App::new(Toolchain::test_fixture());
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("b.mov"), b"").unwrap();
        std::fs::write(root.path().join("a.mov"), b"").unwrap();
        app.open_inputs_picker(false);
        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            false,
        ));
        app.close_picker(PickerAction::Cancel);
        assert!(app.picker.is_none());
        app.picker = Some(Picker::open(
            PickerMode::InputFiles,
            root.path().to_owned(),
            None,
            false,
        ));
        app.close_picker(PickerAction::Done(vec![
            root.path().join("a.mov"),
            root.path().join("b.mov"),
        ]));
        assert_eq!(
            app.draft.inputs,
            vec![root.path().join("a.mov"), root.path().join("b.mov")]
        );
    }

    /// The output picker always reports a destination directory.
    #[test]
    fn output_picker_info_matches_the_selection_shape() {
        let mut app = App::new(Toolchain::test_fixture());
        app.select_inputs(vec![PathBuf::from("/media/clips/a.mov")]);

        let (directory, file_name) = app.output_picker_info();
        assert_eq!(directory.as_deref(), Some(Path::new("/media/clips")));
        assert!(file_name.is_none());

        app.add_inputs(vec![PathBuf::from("/media/clips/b.mov")]);
        let (directory, file_name) = app.output_picker_info();
        assert_eq!(directory.as_deref(), Some(Path::new("/media/clips")));
        assert!(file_name.is_none());

        // The mode stays a folder even for one input.
        app.select_inputs(vec![PathBuf::from("/media/clips/a.mov")]);
        app.open_output_picker();
        assert_eq!(app.picker.as_ref().unwrap().mode, PickerMode::OutputFolder);
    }

    #[test]
    fn a_saved_output_folder_lands_in_the_app() {
        let mut app = App::new(Toolchain::test_fixture());
        app.select_inputs(vec![PathBuf::from("/media/clips/a.mov")]);
        app.open_output_picker();
        app.close_picker(PickerAction::Done(vec![PathBuf::from(
            "/media/clips/exports",
        )]));
        assert_eq!(
            app.draft.output,
            Some(OutputTarget::Directory(PathBuf::from(
                "/media/clips/exports"
            )))
        );
    }

    #[test]
    fn folders_gate_settings_and_done_returns_to_folders() {
        let mut app = App::new(Toolchain::test_fixture());

        app.leave_folders();
        assert_eq!(app.screen, Screen::Folders);
        assert_eq!(
            app.status_message.as_deref(),
            Some("Select one or more video files first.")
        );

        app.input_folder = Some(PathBuf::from("/media/clips"));
        app.draft.inputs = vec![PathBuf::from("/media/clips/a.mov")];
        app.leave_folders();
        assert_eq!(app.screen, Screen::Folders);
        assert_eq!(
            app.status_message.as_deref(),
            Some("Select an output folder first.")
        );

        app.draft.output = Some(OutputTarget::Directory(PathBuf::from("/exports")));
        app.leave_folders();
        assert_eq!(app.screen, Screen::Settings);
        assert_eq!(app.focus, ConfigField::Container);

        app.screen = Screen::Confirm;
        app.handle_key(KeyEvent::from(KeyCode::Esc));
        assert_eq!(app.screen, Screen::Settings);

        app.screen = Screen::Result;
        app.handle_key(KeyEvent::from(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Folders);
        assert_eq!(app.focus, ConfigField::Input);
    }

    #[test]
    fn stderr_keeps_every_line_for_the_current_file_and_clears_on_start() {
        let mut app = App::new(Toolchain::test_fixture());
        for index in 0..25 {
            app.handle_worker_event(WorkerEvent::StderrLine {
                index: 0,
                line: format!("message-{index}"),
            });
        }

        assert_eq!(app.stderr_tail.len(), 25);
        assert_eq!(
            app.stderr_tail.front().map(String::as_str),
            Some("message-0")
        );
        assert_eq!(
            app.stderr_tail.back().map(String::as_str),
            Some("message-24")
        );

        app.handle_worker_event(WorkerEvent::Started { index: 1, pid: 42 });
        assert!(app.stderr_tail.is_empty());
    }
}
