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
        AudioCodec, Container, DraftConfig, EstimateBasis, OutputTarget, QualityPreset,
        RateControlMode, Resolution, VideoCodec, estimate_output_size,
    },
    media::probe_media,
    toolchain::Toolchain,
    transcode::{
        OutputArtifact, QueuedJob, WorkerEvent, build_command_spec, spawn_transcode_worker,
    },
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
        inputs: vec![input.to_owned()],
        output: Some(OutputTarget::File(output.clone())),
        container,
        resolution: Resolution::P480,
        rate_control_mode,
        quality: QualityPreset::Balanced,
        video_bitrate_kbps: 800,
        audio_bitrate_kbps: 96,
        ..DraftConfig::default()
    };
    let config = draft
        .validated_for(input, &media)
        .expect("configuration should be valid");
    let artifact = OutputArtifact::reserve(output).expect("output should be reserved");
    let spec = build_command_spec(&toolchain.ffmpeg, &config, &media, &artifact);
    let (event_tx, event_rx) = mpsc::channel();
    let _handle = spawn_transcode_worker(
        vec![QueuedJob {
            spec,
            artifact,
            duration: media.duration,
        }],
        event_tx,
    );
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
            WorkerEvent::Failed { error, .. } => panic!("FFmpeg worker failed: {error}"),
            WorkerEvent::Cancelled { .. } => panic!("FFmpeg worker was unexpectedly cancelled"),
            WorkerEvent::Started { .. }
            | WorkerEvent::Progress { .. }
            | WorkerEvent::StderrLine { .. }
            | WorkerEvent::QueueFinished { .. } => {}
        }
    }
}

/// The size estimate is the one claim this project makes about a file that does not
/// exist yet, so it is worth checking against a real encode rather than against its own
/// arithmetic. Only the target-bitrate path is asserted: it is the path that promises
/// accuracy, and unlike the constant-quality model it does not depend on what the
/// footage looks like.
#[test]
fn target_bitrate_estimate_matches_a_real_encode() {
    let Some(toolchain) = available_toolchain() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let source = directory.path().join("estimate source.mp4");
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
            OsStr::new("testsrc2=size=640x360:rate=30"),
            OsStr::new("-f"),
            OsStr::new("lavfi"),
            OsStr::new("-i"),
            OsStr::new("sine=frequency=1000:sample_rate=48000"),
            // Long enough that per-file overhead and the encoder's opening frames stop
            // dominating; a one-second clip cannot test a bitrate promise.
            OsStr::new("-t"),
            OsStr::new("8"),
            OsStr::new("-c:v"),
            OsStr::new("libx264"),
            OsStr::new("-preset"),
            OsStr::new("ultrafast"),
            OsStr::new("-pix_fmt"),
            OsStr::new("yuv420p"),
            OsStr::new("-c:a"),
            OsStr::new("aac"),
            OsStr::new("-shortest"),
            source.as_os_str(),
        ],
    );

    let media = probe_media(&toolchain.ffprobe, &source).expect("source should be probed");
    let output = directory.path().join("estimated.mp4");
    let draft = DraftConfig {
        inputs: vec![source.clone()],
        output: Some(OutputTarget::File(output.clone())),
        resolution: Resolution::Source,
        rate_control_mode: RateControlMode::Bitrate,
        video_bitrate_kbps: 1_200,
        audio_bitrate_kbps: 128,
        ..DraftConfig::default()
    };
    let estimate = estimate_output_size(&draft, &media).expect("probed media should estimate");
    assert_eq!(estimate.basis, EstimateBasis::Targeted);

    let config = draft
        .validated_for(&source, &media)
        .expect("configuration should be valid");
    let artifact = OutputArtifact::reserve(output).expect("output should be reserved");
    let spec = build_command_spec(&toolchain.ffmpeg, &config, &media, &artifact);
    let (event_tx, event_rx) = mpsc::channel();
    let _handle = spawn_transcode_worker(
        vec![QueuedJob {
            spec,
            artifact,
            duration: media.duration,
        }],
        event_tx,
    );
    let produced = wait_for_finished(&event_rx);

    let actual = fs::metadata(&produced).expect("output should exist").len() as f64;
    let ratio = actual / estimate.bytes as f64;
    assert!(
        (0.80..=1.20).contains(&ratio),
        "target-bitrate estimate drifted: predicted {} bytes, produced {actual} bytes (ratio {ratio:.2})",
        estimate.bytes
    );
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
            inputs: vec![with_audio.clone()],
            output: Some(OutputTarget::File(output.clone())),
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
            .validated_for(&with_audio, &media)
            .expect("hardware configuration should be valid");
        let artifact = OutputArtifact::reserve(output).expect("output should be reserved");
        let spec = build_command_spec(&toolchain.ffmpeg, &config, &media, &artifact);
        let (event_tx, event_rx) = mpsc::channel();
        let _handle = spawn_transcode_worker(
            vec![QueuedJob {
                spec,
                artifact,
                duration: media.duration,
            }],
            event_tx,
        );

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

/// The queue is the single-file path repeated, so the claim worth checking end to end
/// is that several sources with one set of settings land as several distinct files in
/// the chosen folder, each converted from its own input.
#[test]
fn transcodes_a_queue_of_several_files_into_one_folder() {
    let Some(toolchain) = available_toolchain() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let (with_audio, without_audio) = generate_media(&toolchain, &directory);
    let exports = directory.path().join("exports");
    fs::create_dir(&exports).expect("output folder should be created");

    let draft = DraftConfig {
        inputs: vec![with_audio.clone(), without_audio.clone()],
        output: Some(OutputTarget::Directory(exports.clone())),
        container: Container::Mov,
        resolution: Resolution::P480,
        rate_control_mode: RateControlMode::Bitrate,
        video_bitrate_kbps: 800,
        audio_bitrate_kbps: 96,
        ..DraftConfig::default()
    };
    let sources: Vec<_> = draft
        .inputs
        .iter()
        .map(|input| {
            (
                input.as_path(),
                probe_media(&toolchain.ffprobe, input).expect("input should be probed"),
            )
        })
        .collect();
    let borrowed: Vec<_> = sources
        .iter()
        .map(|(input, media)| (*input, media))
        .collect();
    let configs = draft
        .validated_queue(&borrowed)
        .expect("the queue should be valid");
    assert_eq!(
        configs
            .iter()
            .map(|config| &config.output)
            .collect::<Vec<_>>(),
        vec![
            &exports.join("source with audio.transcoded.mov"),
            &exports.join("silent source.transcoded.mov"),
        ]
    );

    let jobs: Vec<_> = configs
        .iter()
        .zip(&sources)
        .map(|(config, (_, media))| {
            let artifact =
                OutputArtifact::reserve(config.output.clone()).expect("output should be reserved");
            let spec = build_command_spec(&toolchain.ffmpeg, config, media, &artifact);
            QueuedJob {
                spec,
                artifact,
                duration: media.duration,
            }
        })
        .collect();
    let (event_tx, event_rx) = mpsc::channel();
    let _handle = spawn_transcode_worker(jobs, event_tx);

    let mut finished = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for the queue");
        match event_rx
            .recv_timeout(remaining)
            .expect("FFmpeg worker disconnected")
        {
            WorkerEvent::Finished { index, output, .. } => finished.push((index, output)),
            WorkerEvent::Failed { error, .. } => panic!("FFmpeg worker failed: {error}"),
            WorkerEvent::Cancelled { .. } => panic!("the queue was unexpectedly cancelled"),
            WorkerEvent::QueueFinished {
                cancelled,
                remaining,
                ..
            } => {
                assert!(!cancelled);
                assert_eq!(remaining, 0);
                break;
            }
            WorkerEvent::Started { .. }
            | WorkerEvent::Progress { .. }
            | WorkerEvent::StderrLine { .. } => {}
        }
    }

    // Every job reports under its own index, and every output is a real file.
    assert_eq!(finished.len(), 2);
    assert_eq!(finished[0].0, 0);
    assert_eq!(finished[1].0, 1);
    // Each output carries the streams of its own input, not the first one twice: the
    // fixtures differ in both dimensions and audio.
    for ((index, output), (width, height, has_audio)) in
        finished.iter().zip([(320, 180, true), (160, 90, false)])
    {
        assert_eq!(output, &configs[*index].output);
        let produced = probe_media(&toolchain.ffprobe, output).expect("output should be probed");
        assert_eq!(
            (produced.video.width, produced.video.height),
            (width, height)
        );
        assert_eq!(produced.audio.is_some(), has_audio);
    }
    assert!(!has_app_temporary_directory(&exports));
}

/// Cancelling a queue stops the file being written and never starts the rest.
#[test]
fn cancelling_a_queue_leaves_the_files_behind_it_untouched() {
    let Some(toolchain) = available_toolchain() else {
        return;
    };
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let mut inputs = Vec::new();
    for name in ["long one.mp4", "long two.mp4"] {
        let input = directory.path().join(name);
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
        inputs.push(input);
    }
    let exports = directory.path().join("exports");
    fs::create_dir(&exports).expect("output folder should be created");

    let draft = DraftConfig {
        inputs: inputs.clone(),
        output: Some(OutputTarget::Directory(exports.clone())),
        audio_codec: AudioCodec::None,
        ..DraftConfig::default()
    };
    let media: Vec<_> = inputs
        .iter()
        .map(|input| probe_media(&toolchain.ffprobe, input).expect("input should be probed"))
        .collect();
    let borrowed: Vec<_> = inputs
        .iter()
        .map(|input| input.as_path())
        .zip(media.iter())
        .collect();
    let configs = draft
        .validated_queue(&borrowed)
        .expect("the queue should be valid");
    let jobs: Vec<_> = configs
        .iter()
        .zip(&media)
        .map(|(config, media)| {
            let artifact =
                OutputArtifact::reserve(config.output.clone()).expect("output should be reserved");
            let spec = build_command_spec(&toolchain.ffmpeg, config, media, &artifact);
            QueuedJob {
                spec,
                artifact,
                duration: media.duration,
            }
        })
        .collect();
    let (event_tx, event_rx) = mpsc::channel();
    let handle = spawn_transcode_worker(jobs, event_tx);

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut started = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "timed out waiting for cancellation");
        match event_rx
            .recv_timeout(remaining)
            .expect("FFmpeg worker disconnected")
        {
            WorkerEvent::Started { index, .. } => {
                started += 1;
                assert_eq!(index, 0, "the second file must never start");
                handle.cancel();
            }
            WorkerEvent::QueueFinished {
                cancelled,
                remaining,
                ..
            } => {
                assert!(cancelled);
                assert_eq!(remaining, 1, "the queued file must be left unstarted");
                break;
            }
            WorkerEvent::Finished { .. } => panic!("FFmpeg completed before cancellation"),
            WorkerEvent::Failed { error, .. } => panic!("FFmpeg worker failed: {error}"),
            WorkerEvent::Cancelled { index } => assert_eq!(index, 0),
            WorkerEvent::Progress { .. } | WorkerEvent::StderrLine { .. } => {}
        }
    }

    assert_eq!(started, 1);
    for config in &configs {
        assert!(!config.output.exists());
    }
    let cleanup_deadline = Instant::now() + Duration::from_secs(2);
    while has_app_temporary_directory(&exports) && Instant::now() < cleanup_deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!has_app_temporary_directory(&exports));
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
        inputs: vec![input.clone()],
        output: Some(OutputTarget::File(output.clone())),
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
        .validated_for(&input, &media)
        .expect("configuration should be valid");
    let artifact = OutputArtifact::reserve(output.clone()).expect("output should be reserved");
    let spec = build_command_spec(&toolchain.ffmpeg, &config, &media, &artifact);
    let (event_tx, event_rx) = mpsc::channel();
    let handle = spawn_transcode_worker(
        vec![QueuedJob {
            spec,
            artifact,
            duration: media.duration,
        }],
        event_tx,
    );

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
            WorkerEvent::Cancelled { .. } => break,
            WorkerEvent::Finished { .. } => panic!("FFmpeg completed before cancellation"),
            WorkerEvent::Failed { error, .. } => panic!("FFmpeg worker failed: {error}"),
            WorkerEvent::Progress { .. }
            | WorkerEvent::StderrLine { .. }
            | WorkerEvent::QueueFinished { .. } => {}
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
        .any(|entry| entry.file_name().to_string_lossy().starts_with(".fftui-"))
}
