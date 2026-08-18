use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use nix::{
    sys::signal::{self, Signal},
    unistd::Pid,
};
use tempfile::{Builder, TempDir};
use thiserror::Error;

use crate::domain::{
    AudioCodec, Container, InputMedia, TranscodeConfig, VideoCodec, VideoRateControl,
    quality_setting, scale_filter,
};

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub temporary_output: PathBuf,
    pub final_output: PathBuf,
}

#[derive(Debug)]
pub struct OutputArtifact {
    directory: TempDir,
    temporary_path: PathBuf,
    final_path: PathBuf,
}

impl OutputArtifact {
    pub fn reserve(final_path: PathBuf) -> Result<Self, TranscodeError> {
        if final_path.exists() {
            return Err(TranscodeError::OutputExists);
        }
        let parent = final_path
            .parent()
            .filter(|path| path.is_dir())
            .ok_or(TranscodeError::InvalidOutputDirectory)?;
        let directory = Builder::new().prefix(".fftui-").tempdir_in(parent)?;
        let extension = final_path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("media");
        let temporary_path = directory.path().join(format!("output.{extension}"));

        Ok(Self {
            directory,
            temporary_path,
            final_path,
        })
    }

    pub fn temporary_path(&self) -> &Path {
        &self.temporary_path
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    fn persist(self) -> Result<PathBuf, TranscodeError> {
        if self.final_path.exists() {
            return Err(TranscodeError::OutputExists);
        }
        fs::rename(&self.temporary_path, &self.final_path)?;
        let output = self.final_path.clone();
        drop(self.directory);
        Ok(output)
    }
}

#[derive(Debug, Error)]
pub enum TranscodeError {
    #[error("The output file already exists. Choose a new file name.")]
    OutputExists,
    #[error("The output directory does not exist.")]
    InvalidOutputDirectory,
    #[error("Failed to manage the temporary output: {0}")]
    Io(#[from] std::io::Error),
}

pub fn build_command_spec(
    ffmpeg: &Path,
    config: &TranscodeConfig,
    media: &InputMedia,
    artifact: &OutputArtifact,
) -> CommandSpec {
    let mut args = os_args(["-hide_banner", "-nostdin", "-n", "-loglevel", "warning"]);
    if config.video_codec.is_hardware() {
        // Decode on the media engine too. FFmpeg silently falls back to software
        // decoding for formats VideoToolbox cannot handle, so this is safe to set
        // unconditionally for hardware jobs.
        args.extend(os_args(["-hwaccel", "videotoolbox"]));
    }
    args.push(OsString::from("-i"));
    args.push(config.input.as_os_str().to_owned());
    args.extend(os_args(["-map", "0:v:0"]));
    if config.audio_codec == AudioCodec::None || media.audio.is_none() {
        args.push(OsString::from("-an"));
    } else {
        args.extend(os_args(["-map", "0:a:0?", "-c:a"]));
        args.push(OsString::from(config.audio_codec.encoder().unwrap()));
        args.extend(os_args([
            "-b:a",
            &format!("{}k", config.audio_bitrate_kbps),
        ]));
    }
    args.extend(os_args(["-sn", "-dn"]));
    if let Some(filter) = scale_filter(config.resolution) {
        args.extend([OsString::from("-vf"), OsString::from(filter)]);
    }
    args.extend([
        OsString::from("-c:v"),
        OsString::from(config.video_codec.encoder()),
    ]);
    append_video_encoder_args(&mut args, config.video_codec);
    append_rate_control_args(&mut args, config.video_codec, config.video_rate_control);

    if matches!(config.container, Container::Mp4 | Container::Mov) {
        if config.video_codec.is_hevc() {
            args.extend(os_args(["-tag:v", "hvc1"]));
        }
        args.extend(os_args(["-movflags", "+faststart"]));
    }
    args.extend(os_args(["-f", config.container.muxer()]));
    args.extend(os_args(["-progress", "pipe:1", "-nostats"]));
    args.push(artifact.temporary_path().as_os_str().to_owned());

    CommandSpec {
        program: ffmpeg.to_owned(),
        args,
        temporary_output: artifact.temporary_path().to_owned(),
        final_output: artifact.final_path().to_owned(),
    }
}

fn append_video_encoder_args(args: &mut Vec<OsString>, codec: VideoCodec) {
    match codec {
        VideoCodec::H264 | VideoCodec::H265 => {
            args.extend(os_args(["-preset", "medium", "-pix_fmt", "yuv420p"]));
        }
        VideoCodec::Av1 => {
            args.extend(os_args(["-preset", "6", "-pix_fmt", "yuv420p"]));
        }
        VideoCodec::Vp9 => {
            args.extend(os_args(["-deadline", "good", "-pix_fmt", "yuv420p"]));
        }
        // VideoToolbox has no speed preset, and pinning the pixel format would add a
        // pointless conversion before the encoder uploads the frame anyway. Leaving
        // `-allow_sw` at its default keeps the job on the media engine or fails
        // immediately, instead of quietly falling back to a slow software path.
        VideoCodec::H264Hw | VideoCodec::H265Hw => {}
    }
}

fn append_rate_control_args(
    args: &mut Vec<OsString>,
    codec: VideoCodec,
    rate_control: VideoRateControl,
) {
    match rate_control {
        VideoRateControl::Quality(quality) => {
            if codec == VideoCodec::Vp9 {
                args.extend(os_args(["-b:v", "0"]));
            }
            let setting = quality_setting(codec, quality);
            args.extend([
                OsString::from(setting.flag),
                OsString::from(setting.value.to_string()),
            ]);
        }
        VideoRateControl::Bitrate(kbps) => {
            args.extend(os_args(["-b:v", &format!("{kbps}k")]));
        }
    }
}

fn os_args<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}

pub fn render_command_preview(spec: &CommandSpec) -> String {
    std::iter::once(spec.program.as_os_str())
        .chain(spec.args.iter().map(OsString::as_os_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &OsStr) -> String {
    let value = value.to_string_lossy();
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_+-./:=,@%".contains(character))
    {
        value.into_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub processed: Duration,
    pub percent: Option<f64>,
    pub speed: Option<String>,
    pub finished: bool,
}

#[derive(Debug, Default)]
pub struct ProgressParser {
    values: HashMap<String, String>,
    duration: Option<Duration>,
}

impl ProgressParser {
    pub fn new(duration: Option<Duration>) -> Self {
        Self {
            values: HashMap::new(),
            duration,
        }
    }

    pub fn push_line(&mut self, line: &str) -> Option<ProgressUpdate> {
        let (key, value) = line.trim().split_once('=')?;
        self.values.insert(key.to_owned(), value.to_owned());
        if key != "progress" {
            return None;
        }

        let micros = self
            .values
            .get("out_time_us")
            .or_else(|| self.values.get("out_time_ms"))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let processed = Duration::from_micros(micros);
        let percent = self.duration.and_then(|duration| {
            let total = duration.as_secs_f64();
            (total > 0.0).then(|| (processed.as_secs_f64() / total * 100.0).clamp(0.0, 100.0))
        });
        let update = ProgressUpdate {
            processed,
            percent,
            speed: self.values.get("speed").cloned(),
            finished: value == "end",
        };
        self.values.clear();
        Some(update)
    }
}

#[derive(Debug)]
pub enum WorkerEvent {
    Started { pid: u32 },
    Progress(ProgressUpdate),
    StderrLine(String),
    Finished { output: PathBuf, elapsed: Duration },
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
enum WorkerCommand {
    Cancel,
}

#[derive(Debug)]
pub struct TranscodeHandle {
    command_tx: Sender<WorkerCommand>,
}

impl TranscodeHandle {
    pub fn cancel(&self) {
        let _ = self.command_tx.send(WorkerCommand::Cancel);
    }
}

pub fn spawn_transcode_worker(
    spec: CommandSpec,
    artifact: OutputArtifact,
    duration: Option<Duration>,
    event_tx: Sender<WorkerEvent>,
) -> TranscodeHandle {
    let (command_tx, command_rx) = mpsc::channel();
    thread::spawn(move || run_worker(spec, artifact, duration, event_tx, command_rx));
    TranscodeHandle { command_tx }
}

fn run_worker(
    spec: CommandSpec,
    artifact: OutputArtifact,
    duration: Option<Duration>,
    event_tx: Sender<WorkerEvent>,
    command_rx: Receiver<WorkerCommand>,
) {
    let started_at = Instant::now();
    let mut child = match Command::new(&spec.program)
        .args(&spec.args)
        .env("AV_LOG_FORCE_NOCOLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Failed(format!(
                "Failed to start FFmpeg: {error}"
            )));
            return;
        }
    };

    let pid = child.id();
    let _ = event_tx.send(WorkerEvent::Started { pid });
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let progress_tx = event_tx.clone();
    let stdout_thread = thread::spawn(move || {
        let mut parser = ProgressParser::new(duration);
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(update) = parser.push_line(&line) {
                    let _ = progress_tx.send(WorkerEvent::Progress(update));
                }
            }
        }
    });
    let stderr_tx = event_tx.clone();
    let stderr_thread = thread::spawn(move || {
        if let Some(stderr) = stderr {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let sanitized: String = line
                    .chars()
                    .filter(|character| !character.is_control() || *character == '\t')
                    .take(2_000)
                    .collect();
                let _ = stderr_tx.send(WorkerEvent::StderrLine(sanitized));
            }
        }
    });

    let mut cancelling_at: Option<Instant> = None;
    let status = loop {
        if cancelling_at.is_none() {
            match command_rx.try_recv() {
                Ok(WorkerCommand::Cancel) => {
                    let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGINT);
                    cancelling_at = Some(Instant::now());
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }
        }

        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = event_tx.send(WorkerEvent::Failed(format!(
                    "Failed while waiting for FFmpeg: {error}"
                )));
                break None;
            }
        }

        if cancelling_at.is_some_and(|time| time.elapsed() >= Duration::from_secs(3)) {
            let _ = child.kill();
        }
        thread::sleep(Duration::from_millis(50));
    };

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let Some(status) = status else {
        return;
    };
    if cancelling_at.is_some() {
        let _ = event_tx.send(WorkerEvent::Cancelled);
    } else if status.success() {
        match artifact.persist() {
            Ok(output) => {
                let _ = event_tx.send(WorkerEvent::Finished {
                    output,
                    elapsed: started_at.elapsed(),
                });
            }
            Err(error) => {
                let _ = event_tx.send(WorkerEvent::Failed(error.to_string()));
            }
        }
    } else {
        let description = status.code().map_or_else(
            || "terminated by a signal".to_owned(),
            |code| format!("exit code {code}"),
        );
        let _ = event_tx.send(WorkerEvent::Failed(format!(
            "FFmpeg failed with {description}."
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AudioStreamInfo, Container, InputMedia, QualityPreset, Resolution, TranscodeConfig,
        VideoStreamInfo,
    };

    fn media(path: PathBuf) -> InputMedia {
        InputMedia {
            path,
            duration: Some(Duration::from_secs(10)),
            video: VideoStreamInfo {
                codec: "h264".to_owned(),
                width: 1920,
                height: 1080,
                frame_rate: Some(30.0),
                bitrate_kbps: Some(8_000),
            },
            audio: Some(AudioStreamInfo {
                codec: "aac".to_owned(),
                channels: Some(2),
                sample_rate: Some(48_000),
            }),
            format_name: Some("mov,mp4".to_owned()),
            size_bytes: Some(10_000_000),
            bitrate_kbps: Some(8_192),
        }
    }

    fn config(input: PathBuf, output: PathBuf, rate: VideoRateControl) -> TranscodeConfig {
        TranscodeConfig {
            input,
            output,
            container: Container::Mp4,
            video_codec: VideoCodec::H264,
            audio_codec: AudioCodec::Aac,
            resolution: Resolution::P720,
            video_rate_control: rate,
            audio_bitrate_kbps: 192,
        }
    }

    #[test]
    fn quality_and_target_bitrate_are_mutually_exclusive() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("source file.mov");
        fs::write(&input, b"test").unwrap();
        let output = directory.path().join("result.mp4");
        let artifact = OutputArtifact::reserve(output.clone()).unwrap();
        let quality = build_command_spec(
            Path::new("/usr/bin/ffmpeg"),
            &config(
                input.clone(),
                output.clone(),
                VideoRateControl::Quality(QualityPreset::Balanced),
            ),
            &media(input.clone()),
            &artifact,
        );
        let quality_args = quality
            .args
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(quality_args.windows(2).any(|pair| pair == ["-crf", "23"]));
        assert!(!quality_args.iter().any(|value| value == "5000k"));

        let bitrate = build_command_spec(
            Path::new("/usr/bin/ffmpeg"),
            &config(input.clone(), output, VideoRateControl::Bitrate(5_000)),
            &media(input),
            &artifact,
        );
        let bitrate_args = bitrate
            .args
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(
            bitrate_args
                .windows(2)
                .any(|pair| pair == ["-b:v", "5000k"])
        );
        assert!(!bitrate_args.iter().any(|value| value == "-crf"));
    }

    #[test]
    fn mov_family_containers_use_compatible_flags() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("source.mp4");
        fs::write(&input, b"test").unwrap();

        for (container, extension, muxer) in [
            (Container::Mp4, "mp4", "mp4"),
            (Container::Mov, "mov", "mov"),
        ] {
            let output = directory.path().join(format!("result.{extension}"));
            let artifact = OutputArtifact::reserve(output.clone()).unwrap();
            let mut transcode_config = config(
                input.clone(),
                output,
                VideoRateControl::Quality(QualityPreset::Balanced),
            );
            transcode_config.container = container;
            transcode_config.video_codec = VideoCodec::H265;

            let spec = build_command_spec(
                Path::new("/usr/bin/ffmpeg"),
                &transcode_config,
                &media(input.clone()),
                &artifact,
            );
            let args = spec
                .args
                .iter()
                .map(|value| value.to_string_lossy())
                .collect::<Vec<_>>();

            assert_eq!(
                spec.temporary_output.extension().and_then(OsStr::to_str),
                Some(extension)
            );
            assert!(args.windows(2).any(|pair| pair == ["-f", muxer]));
            assert!(args.windows(2).any(|pair| pair == ["-tag:v", "hvc1"]));
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["-movflags", "+faststart"])
            );
        }
    }

    #[test]
    fn hardware_encoders_use_videotoolbox_flags() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("source.mp4");
        fs::write(&input, b"test").unwrap();
        let output = directory.path().join("hw.mp4");
        let artifact = OutputArtifact::reserve(output.clone()).unwrap();
        let mut transcode_config = config(
            input.clone(),
            output,
            VideoRateControl::Quality(QualityPreset::Balanced),
        );
        transcode_config.video_codec = VideoCodec::H265Hw;

        let spec = build_command_spec(
            Path::new("/usr/bin/ffmpeg"),
            &transcode_config,
            &media(input.clone()),
            &artifact,
        );
        let args = spec
            .args
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>();

        assert!(
            args.windows(2)
                .any(|pair| pair == ["-hwaccel", "videotoolbox"])
        );
        // Hardware decoding only helps when it is selected before the input.
        let hwaccel = args.iter().position(|value| value == "-hwaccel").unwrap();
        let input_flag = args.iter().position(|value| value == "-i").unwrap();
        assert!(hwaccel < input_flag);

        assert!(
            args.windows(2)
                .any(|pair| pair == ["-c:v", "hevc_videotoolbox"])
        );
        assert!(args.windows(2).any(|pair| pair == ["-q:v", "55"]));
        assert!(args.windows(2).any(|pair| pair == ["-tag:v", "hvc1"]));
        // VideoToolbox rejects libx26x-style speed presets.
        assert!(!args.iter().any(|value| value == "-preset"));
        assert!(!args.iter().any(|value| value == "-crf"));

        // Software jobs must not pick up hardware decoding.
        transcode_config.video_codec = VideoCodec::H265;
        let software = build_command_spec(
            Path::new("/usr/bin/ffmpeg"),
            &transcode_config,
            &media(input),
            &artifact,
        );
        let software_args = software
            .args
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(!software_args.iter().any(|value| value == "-hwaccel"));
        assert!(software_args.windows(2).any(|pair| pair == ["-crf", "26"]));
    }

    #[test]
    fn unusual_path_remains_one_argument() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("a 'quoted' $clip.mov");
        fs::write(&input, b"test").unwrap();
        let output = directory.path().join("result.mp4");
        let artifact = OutputArtifact::reserve(output.clone()).unwrap();
        let spec = build_command_spec(
            Path::new("ffmpeg"),
            &config(input.clone(), output, VideoRateControl::Bitrate(2_500)),
            &media(input.clone()),
            &artifact,
        );
        assert_eq!(
            spec.args
                .iter()
                .filter(|arg| *arg == input.as_os_str())
                .count(),
            1
        );
        assert!(render_command_preview(&spec).contains("'\\''quoted'\\''"));
    }

    #[test]
    fn parses_progress_and_clamps_percentage() {
        let mut parser = ProgressParser::new(Some(Duration::from_secs(2)));
        assert!(parser.push_line("out_time_us=3000000").is_none());
        assert!(parser.push_line("speed=1.5x").is_none());
        let update = parser.push_line("progress=continue").unwrap();
        assert_eq!(update.percent, Some(100.0));
        assert_eq!(update.speed.as_deref(), Some("1.5x"));
    }
}
