use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    domain::{
        AUDIO_BITRATE_PRESETS, AudioCodec, Container, DraftConfig, InputMedia, QualityPreset,
        RateControlMode, Resolution, TranscodeConfig, VIDEO_BITRATE_PRESETS, quality_setting,
        suggested_output_path, supported_audio_codecs, supported_video_codecs,
    },
    media::probe_media,
    toolchain::Toolchain,
    transcode::{
        CommandSpec, OutputArtifact, ProgressUpdate, TranscodeHandle, WorkerEvent,
        build_command_spec, render_command_preview, spawn_transcode_worker,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Configure,
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
        pid: u32,
        progress: Option<ProgressUpdate>,
    },
    Cancelling,
    Succeeded {
        output: PathBuf,
        elapsed: Duration,
    },
    Cancelled,
    Failed(String),
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
    const ALL: [Self; 9] = [
        Self::Input,
        Self::Output,
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
pub enum UiCommand {
    None,
    OpenInput,
    OpenOutput,
    Quit,
}

#[derive(Debug)]
struct NumericEdit {
    field: ConfigField,
    buffer: String,
}

#[derive(Debug)]
struct PreparedJob {
    config: TranscodeConfig,
    spec: CommandSpec,
    artifact: OutputArtifact,
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
    pub media: Option<InputMedia>,
    pub screen: Screen,
    pub job: JobState,
    pub focus: ConfigField,
    pub status_message: Option<String>,
    pub stderr_tail: VecDeque<String>,
    pub command_preview: Option<String>,
    pub help_visible: bool,
    pub cancel_confirmation: bool,
    numeric_edit: Option<NumericEdit>,
    prepared: Option<PreparedJob>,
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
            media: None,
            screen: Screen::Configure,
            job: JobState::Idle,
            focus: ConfigField::Input,
            status_message: Some("Select an input file to begin.".to_owned()),
            stderr_tail: VecDeque::with_capacity(20),
            command_preview: None,
            help_visible: false,
            cancel_confirmation: false,
            numeric_edit: None,
            prepared: None,
            transcode_handle: None,
            event_tx,
            event_rx,
        }
    }

    pub fn select_input(&mut self, path: PathBuf) {
        self.prepared = None;
        self.command_preview = None;
        self.media = None;
        self.draft.input = Some(path.clone());
        self.draft.output = Some(suggested_output_path(&path, self.draft.container));
        self.job = JobState::Probing;
        self.status_message = Some("Probing input media…".to_owned());
        let ffprobe = self.toolchain.ffprobe.clone();
        let event_tx = self.event_tx.clone();
        thread::spawn(move || {
            let result = probe_media(&ffprobe, &path).map_err(|error| error.to_string());
            let _ = event_tx.send(AppEvent::Probe {
                input: path,
                result,
            });
        });
    }

    pub fn select_output(&mut self, mut path: PathBuf) {
        path.set_extension(self.draft.container.extension());
        self.draft.output = Some(path);
        self.prepared = None;
        self.command_preview = None;
        self.refresh_ready_message();
    }

    /// Surfaces a failure that happened outside the app state machine.
    pub fn report_error(&mut self, message: String) {
        self.status_message = Some(message);
    }

    pub fn poll_background(&mut self) {
        let events: Vec<_> = self.event_rx.try_iter().collect();
        for event in events {
            match event {
                AppEvent::Probe { input, result } => {
                    if self.draft.input.as_ref() == Some(&input) {
                        self.handle_probe_result(result);
                    }
                }
                AppEvent::Worker(event) => self.handle_worker_event(event),
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> UiCommand {
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
            Screen::Configure => self.handle_configure_key(key),
            Screen::Confirm => self.handle_confirm_key(key),
            Screen::Running => self.handle_running_key(key),
            Screen::Result | Screen::Error => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.screen = Screen::Configure;
                    self.job = if self.media.is_some() {
                        JobState::Ready
                    } else {
                        JobState::Idle
                    };
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

    pub fn current_validation_error(&self) -> Option<String> {
        let media = self.media.as_ref()?;
        if !self.toolchain.supports_video(self.draft.video_codec) {
            return Some(format!(
                "{} is not available in this FFmpeg build.",
                self.draft.video_codec.encoder()
            ));
        }
        if !self.toolchain.supports_audio(self.draft.audio_codec) {
            return Some(format!(
                "{} is not available in this FFmpeg build.",
                self.draft.audio_codec.encoder().unwrap_or("audio encoder")
            ));
        }
        self.draft
            .validated(media)
            .err()
            .map(|error| error.to_string())
    }

    pub fn quality_label(&self) -> String {
        format!(
            "{} ({})",
            self.draft.quality,
            quality_setting(self.draft.video_codec, self.draft.quality)
        )
    }

    pub fn request_cancel(&mut self) {
        if let Some(handle) = &self.transcode_handle {
            handle.cancel();
            self.job = JobState::Cancelling;
            self.cancel_confirmation = false;
            self.status_message = Some("Cancelling FFmpeg…".to_owned());
        }
    }

    fn handle_probe_result(&mut self, result: Result<InputMedia, String>) {
        match result {
            Ok(media) => {
                self.media = Some(media);
                self.job = JobState::Ready;
                self.refresh_ready_message();
            }
            Err(error) => {
                self.media = None;
                self.job = JobState::Failed(error.clone());
                self.status_message = Some(error);
            }
        }
    }

    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Started { pid } => {
                self.job = JobState::Running {
                    pid,
                    progress: None,
                };
                self.status_message = Some("FFmpeg is running.".to_owned());
            }
            WorkerEvent::Progress(progress) => {
                if !matches!(self.job, JobState::Cancelling) {
                    let pid = match self.job {
                        JobState::Running { pid, .. } => pid,
                        _ => 0,
                    };
                    self.job = JobState::Running {
                        pid,
                        progress: Some(progress),
                    };
                }
            }
            WorkerEvent::StderrLine(line) => {
                if self.stderr_tail.len() == 20 {
                    self.stderr_tail.pop_front();
                }
                self.stderr_tail.push_back(line);
            }
            WorkerEvent::Finished { output, elapsed } => {
                self.transcode_handle = None;
                self.job = JobState::Succeeded {
                    output: output.clone(),
                    elapsed,
                };
                self.status_message = Some("Conversion completed successfully.".to_owned());
                self.screen = Screen::Result;
            }
            WorkerEvent::Cancelled => {
                self.transcode_handle = None;
                self.job = JobState::Cancelled;
                self.status_message =
                    Some("Conversion cancelled. Temporary output removed.".to_owned());
                self.screen = Screen::Result;
            }
            WorkerEvent::Failed(error) => {
                self.transcode_handle = None;
                self.job = JobState::Failed(error.clone());
                self.status_message = Some(error);
                self.screen = Screen::Error;
            }
        }
    }

    fn handle_configure_key(&mut self, key: KeyEvent) -> UiCommand {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return UiCommand::Quit;
        }
        match key.code {
            KeyCode::Char('q') => UiCommand::Quit,
            KeyCode::Char('?') => {
                self.help_visible = true;
                UiCommand::None
            }
            KeyCode::Char('i') => UiCommand::OpenInput,
            KeyCode::Char('o') => UiCommand::OpenOutput,
            KeyCode::Char('r') => {
                if let Some(path) = self.draft.input.clone() {
                    self.select_input(path);
                }
                UiCommand::None
            }
            KeyCode::Tab | KeyCode::Down | KeyCode::Char('j') => {
                self.move_focus(1);
                UiCommand::None
            }
            KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => {
                self.move_focus(-1);
                UiCommand::None
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.adjust_focused(-1);
                UiCommand::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.adjust_focused(1);
                UiCommand::None
            }
            KeyCode::Enter => match self.focus {
                ConfigField::Input => UiCommand::OpenInput,
                ConfigField::Output => UiCommand::OpenOutput,
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
                _ => {
                    self.prepare_confirmation();
                    UiCommand::None
                }
            },
            _ => UiCommand::None,
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> UiCommand {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                self.start_prepared();
                UiCommand::None
            }
            KeyCode::Esc | KeyCode::Char('n') => {
                self.prepared = None;
                self.command_preview = None;
                self.screen = Screen::Configure;
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
        if (key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c'))
            || matches!(key.code, KeyCode::Char('x' | 'q') | KeyCode::Esc)
        {
            self.cancel_confirmation = true;
        }
        UiCommand::None
    }

    fn handle_numeric_key(&mut self, key: KeyEvent) -> UiCommand {
        match key.code {
            KeyCode::Char(character) if character.is_ascii_digit() => {
                if let Some(edit) = self.numeric_edit.as_mut()
                    && edit.buffer.len() < 6
                {
                    edit.buffer.push(character);
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

    fn move_focus(&mut self, direction: i32) {
        let current = ConfigField::ALL
            .iter()
            .position(|field| *field == self.focus)
            .unwrap_or(0);
        for offset in 1..=ConfigField::ALL.len() {
            let index = (current as i32 + direction * offset as i32)
                .rem_euclid(ConfigField::ALL.len() as i32) as usize;
            let candidate = ConfigField::ALL[index];
            if self.field_enabled(candidate) {
                self.focus = candidate;
                return;
            }
        }
    }

    fn field_enabled(&self, field: ConfigField) -> bool {
        match field {
            ConfigField::RateValue => true,
            ConfigField::AudioBitrate => self.audio_bitrate_enabled(),
            _ => true,
        }
    }

    fn audio_bitrate_enabled(&self) -> bool {
        self.draft.audio_codec != AudioCodec::None
            && self
                .media
                .as_ref()
                .is_some_and(|media| media.audio.is_some())
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
        self.prepared = None;
        self.command_preview = None;
        self.refresh_ready_message();
    }

    fn prepare_confirmation(&mut self) {
        let Some(media) = self.media.as_ref() else {
            self.status_message = Some("Select and probe an input file first.".to_owned());
            return;
        };
        if let Some(error) = self.current_validation_error() {
            self.status_message = Some(error);
            return;
        }
        let config = match self.draft.validated(media) {
            Ok(config) => config,
            Err(error) => {
                self.status_message = Some(error.to_string());
                return;
            }
        };
        let artifact = match OutputArtifact::reserve(config.output.clone()) {
            Ok(artifact) => artifact,
            Err(error) => {
                self.status_message = Some(error.to_string());
                return;
            }
        };
        let spec = build_command_spec(&self.toolchain.ffmpeg, &config, media, &artifact);
        self.command_preview = Some(render_command_preview(&spec));
        self.prepared = Some(PreparedJob {
            config,
            spec,
            artifact,
        });
        self.screen = Screen::Confirm;
    }

    fn start_prepared(&mut self) {
        let Some(prepared) = self.prepared.take() else {
            self.screen = Screen::Configure;
            return;
        };
        let duration = self.media.as_ref().and_then(|media| media.duration);
        self.stderr_tail.clear();
        self.screen = Screen::Running;
        self.job = JobState::Starting;
        self.status_message = Some(format!(
            "Starting {} → {}…",
            prepared.config.input.display(),
            prepared.config.output.display()
        ));
        let worker_tx = self.event_tx.clone();
        let (event_tx, event_rx) = mpsc::channel();
        thread::spawn(move || {
            for event in event_rx {
                let _ = worker_tx.send(AppEvent::Worker(event));
            }
        });
        self.transcode_handle = Some(spawn_transcode_worker(
            prepared.spec,
            prepared.artifact,
            duration,
            event_tx,
        ));
    }

    fn refresh_ready_message(&mut self) {
        if self.media.is_some() {
            self.status_message = self
                .current_validation_error()
                .or_else(|| Some("Configuration is ready. Press Enter to review.".to_owned()));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if let Some(handle) = &self.transcode_handle {
            handle.cancel();
        }
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

fn cycle_numeric(values: &[u32], current: u32, direction: i32) -> u32 {
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or_else(|| {
            values
                .iter()
                .enumerate()
                .min_by_key(|(_, value)| value.abs_diff(current))
                .map(|(index, _)| index)
                .unwrap_or(0)
        });
    let next = (index as i32 + direction).rem_euclid(values.len() as i32) as usize;
    values[next]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_custom_bitrate_to_nearest_preset() {
        assert_eq!(cycle_numeric(VIDEO_BITRATE_PRESETS, 4_900, 1), 8_000);
        assert_eq!(cycle_numeric(VIDEO_BITRATE_PRESETS, 4_900, -1), 2_500);
    }

    #[test]
    fn ignores_probe_results_for_a_previous_input() {
        let mut app = App::new(Toolchain::test_fixture());
        app.draft.input = Some(PathBuf::from("current.mp4"));
        app.event_tx
            .send(AppEvent::Probe {
                input: PathBuf::from("previous.mp4"),
                result: Err("stale error".to_owned()),
            })
            .unwrap();

        app.poll_background();

        assert!(matches!(app.job, JobState::Idle));
        assert_ne!(app.status_message.as_deref(), Some("stale error"));
    }

    #[test]
    fn wraps_enum_selection() {
        assert_eq!(cycle(&Container::ALL, Container::Mp4, -1), Container::WebM);
    }
}
