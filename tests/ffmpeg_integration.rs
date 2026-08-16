use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use fftui::{
    domain::{
        AudioCodec, Container, DraftConfig, QualityPreset, RateControlMode, Resolution, VideoCodec,
    },
    media::probe_media,
    toolchain::Toolchain,
    transcode::{OutputArtifact, WorkerEvent, build_command_spec, spawn_transcode_worker},
};
use tempfile::TempDir;

fn available_toolchain() -> Option<Toolchain> {
    match Toolchain::discover() {
        Ok(toolchain)
            if toolchain.supports_video(VideoCodec::H264)
                && toolchain.supports_audio(AudioCodec::Aac) =>
        {
            Some(toolchain)
        }
        Ok(_) => {
            eprintln!("Skipping FFmpeg integration test: libx264 or AAC is unavailable.");
            None
        }
        Err(error) => {
            eprintln!("Skipping FFmpeg integration test: {error}");
            None
        }
    }
}

fn run_ffmpeg(ffmpeg: &Path, args: &[&OsStr]) {
    let output = Command::new(ffmpeg)
        .args(args)
        .output()
        .expect("FFmpeg fixture command should start");
    assert!(
        output.status.success(),
        "FFmpeg fixture command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn generate_media(toolchain: &Toolchain, directory: &TempDir) -> (PathBuf, PathBuf) {
    let with_audio = directory.path().join("source with audio.mp4");
    run_ffmpeg(
        &toolchain.ffmpeg,
        &[
            OsStr::new("-hide_banner"),
            OsStr::new("-loglevel"),
            OsStr::new("error"),
            OsStr::new("-y"),
            OsStr::new("-f"),
            OsStr::new("lavfi"),
            OsStr::new("-i"),
            OsStr::new("testsrc2=size=320x180:rate=24"),
            OsStr::new("-f"),
            OsStr::new("lavfi"),
            OsStr::new("-i"),
            OsStr::new("sine=frequency=1000:sample_rate=48000"),
            OsStr::new("-t"),
            OsStr::new("1"),
            OsStr::new("-c:v"),
            OsStr::new("libx264"),
            OsStr::new("-preset"),
            OsStr::new("ultrafast"),
            OsStr::new("-pix_fmt"),
            OsStr::new("yuv420p"),
            OsStr::new("-c:a"),
            OsStr::new("aac"),
            OsStr::new("-shortest"),
            with_audio.as_os_str(),
        ],
    );

    let without_audio = directory.path().join("silent source.mp4");
    run_ffmpeg(
        &toolchain.ffmpeg,
        &[
            OsStr::new("-hide_banner"),
            OsStr::new("-loglevel"),
            OsStr::new("error"),
            OsStr::new("-y"),
            OsStr::new("-f"),
            OsStr::new("lavfi"),
            OsStr::new("-i"),
            OsStr::new("color=c=blue:size=160x90:rate=24"),
            OsStr::new("-t"),
            OsStr::new("0.5"),
            OsStr::new("-c:v"),
            OsStr::new("libx264"),
            OsStr::new("-preset"),
            OsStr::new("ultrafast"),
            OsStr::new("-pix_fmt"),
            OsStr::new("yuv420p"),
            OsStr::new("-an"),
            without_audio.as_os_str(),
        ],
    );

    (with_audio, without_audio)
}

fn transcode(
    toolchain: &Toolchain,
    input: &Path,
    output: PathBuf,
    container: Container,
    rate_control_mode: RateControlMode,
) -> PathBuf {
    let media = probe_media(&toolchain.ffprobe, input).expect("input should be probed");
    let draft = DraftConfig {
        input: Some(input.to_owned()),
        output: Some(output.clone()),
        container,
        resolution: Resolution::P480,
        rate_control_mode,
        quality: QualityPreset::Balanced,
        video_bitrate_kbps: 800,
        audio_bitrate_kbps: 96,
        ..DraftConfig::default()
    };
    let config = draft
        .validated(&media)
        .expect("configuration should be valid");
    let artifact = OutputArtifact::reserve(output).expect("output should be reserved");
    let spec = build_command_spec(&toolchain.ffmpeg, &config, &media, &artifact);
    let (event_tx, event_rx) = mpsc::channel();
    let _handle = spawn_transcode_worker(spec, artifact, media.duration, event_tx);
    wait_for_finished(&event_rx)
}

fn wait_for_finished(event_rx: &Receiver<WorkerEvent>) -> PathBuf {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for FFmpeg");
        match event_rx
            .recv_timeout(remaining)
            .expect("FFmpeg worker disconnected")
        {
            WorkerEvent::Finished { output, .. } => return output,
            WorkerEvent::Failed(error) => panic!("FFmpeg worker failed: {error}"),
            WorkerEvent::Cancelled => panic!("FFmpeg worker was unexpectedly cancelled"),
            WorkerEvent::Started { .. } | WorkerEvent::Progress(_) | WorkerEvent::StderrLine(_) => {
            }
        }
    }
}

#[test]
fn probes_inputs_and_transcodes_mp4_rate_modes_and_mov_output() {
    let Some(toolchain) = available_toolchain() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let (with_audio, without_audio) = generate_media(&toolchain, &directory);

    let audio_media = probe_media(&toolchain.ffprobe, &with_audio).expect("media should be probed");
    assert_eq!(
        (audio_media.video.width, audio_media.video.height),
        (320, 180)
    );
    assert_eq!(audio_media.video.codec, "h264");
    assert_eq!(
        audio_media.audio.as_ref().map(|audio| audio.codec.as_str()),
        Some("aac")
    );

    let silent_media =
        probe_media(&toolchain.ffprobe, &without_audio).expect("silent media should be probed");
    assert!(silent_media.audio.is_none());

    for (name, container, mode) in [
        (
            "quality output.mp4",
            Container::Mp4,
            RateControlMode::Quality,
        ),
        (
            "bitrate output.mp4",
            Container::Mp4,
            RateControlMode::Bitrate,
        ),
        (
            "quality output.mov",
            Container::Mov,
            RateControlMode::Quality,
        ),
    ] {
        let output = transcode(
            &toolchain,
            &with_audio,
            directory.path().join(name),
            container,
            mode,
        );
        assert!(output.exists());
        assert_eq!(
            output.extension().and_then(OsStr::to_str),
            Some(container.extension())
        );
        let result = probe_media(&toolchain.ffprobe, &output).expect("output should be probed");
        assert_eq!(result.video.codec, "h264");
        assert_eq!((result.video.width, result.video.height), (320, 180));
        assert_eq!(
            result.audio.as_ref().map(|audio| audio.codec.as_str()),
            Some("aac")
        );
    }
}

#[test]
fn videotoolbox_encoders_produce_playable_output() {
    let Some(toolchain) = available_toolchain() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let (with_audio, _) = generate_media(&toolchain, &directory);
    let media = probe_media(&toolchain.ffprobe, &with_audio).expect("input should be probed");

    for (codec, name, expected) in [
        (VideoCodec::H264Hw, "hardware.mp4", "h264"),
        (VideoCodec::H265Hw, "hardware.mov", "hevc"),
    ] {
        if !toolchain.supports_video(codec) {
            eprintln!("Skipping {codec}: {} is unavailable.", codec.encoder());
            continue;
        }
        let output = directory.path().join(name);
        let draft = DraftConfig {
            input: Some(with_audio.clone()),
            output: Some(output.clone()),
            container: if codec == VideoCodec::H265Hw {
                Container::Mov
            } else {
                Container::Mp4
            },
            video_codec: codec,
            resolution: Resolution::P480,
            quality: QualityPreset::Balanced,
            ..DraftConfig::default()
        };
        let config = draft
            .validated(&media)
            .expect("hardware configuration should be valid");
        let artifact = OutputArtifact::reserve(output).expect("output should be reserved");
        let spec = build_command_spec(&toolchain.ffmpeg, &config, &media, &artifact);
        let (event_tx, event_rx) = mpsc::channel();
        let _handle = spawn_transcode_worker(spec, artifact, media.duration, event_tx);

        let output = wait_for_finished(&event_rx);
        let result = probe_media(&toolchain.ffprobe, &output).expect("output should be probed");
        assert_eq!(result.video.codec, expected);
        assert_eq!((result.video.width, result.video.height), (320, 180));
        assert_eq!(
            result.audio.as_ref().map(|audio| audio.codec.as_str()),
            Some("aac")
        );
    }
}

#[test]
fn cancellation_removes_final_and_temporary_outputs() {
    let Some(toolchain) = available_toolchain() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let input = directory.path().join("long source.mp4");
    run_ffmpeg(
        &toolchain.ffmpeg,
        &[
            OsStr::new("-hide_banner"),
            OsStr::new("-loglevel"),
            OsStr::new("error"),
            OsStr::new("-y"),
            OsStr::new("-f"),
            OsStr::new("lavfi"),
            OsStr::new("-i"),
            OsStr::new("testsrc2=size=1280x720:rate=30"),
            OsStr::new("-t"),
            OsStr::new("8"),
            OsStr::new("-c:v"),
            OsStr::new("mpeg4"),
            OsStr::new("-q:v"),
            OsStr::new("8"),
            OsStr::new("-an"),
            input.as_os_str(),
        ],
    );

    let media = probe_media(&toolchain.ffprobe, &input).expect("input should be probed");
    let output = directory.path().join("cancelled.mp4");
    let draft = DraftConfig {
        input: Some(input),
        output: Some(output.clone()),
        container: Container::Mp4,
        video_codec: VideoCodec::H264,
        audio_codec: AudioCodec::None,
        resolution: Resolution::Source,
        rate_control_mode: RateControlMode::Quality,
        quality: QualityPreset::Balanced,
        video_bitrate_kbps: 5_000,
        audio_bitrate_kbps: 192,
    };
    let config = draft
        .validated(&media)
        .expect("configuration should be valid");
    let artifact = OutputArtifact::reserve(output.clone()).expect("output should be reserved");
    let spec = build_command_spec(&toolchain.ffmpeg, &config, &media, &artifact);
    let (event_tx, event_rx) = mpsc::channel();
    let handle = spawn_transcode_worker(spec, artifact, media.duration, event_tx);

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for FFmpeg cancellation"
        );
        match event_rx
            .recv_timeout(remaining)
            .expect("FFmpeg worker disconnected")
        {
            WorkerEvent::Started { .. } => handle.cancel(),
            WorkerEvent::Cancelled => break,
            WorkerEvent::Finished { .. } => panic!("FFmpeg completed before cancellation"),
            WorkerEvent::Failed(error) => panic!("FFmpeg worker failed: {error}"),
            WorkerEvent::Progress(_) | WorkerEvent::StderrLine(_) => {}
        }
    }

    assert!(!output.exists());
    let cleanup_deadline = Instant::now() + Duration::from_secs(2);
    while has_app_temporary_directory(directory.path()) && Instant::now() < cleanup_deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!has_app_temporary_directory(directory.path()));
}

fn has_app_temporary_directory(directory: &Path) -> bool {
    fs::read_dir(directory)
        .expect("temporary directory should be readable")
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".fftui-")
        })
}
