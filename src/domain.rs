use std::{fmt, path::PathBuf, time::Duration};

use thiserror::Error;

pub const VIDEO_BITRATE_PRESETS: &[u32] = &[1_000, 2_500, 5_000, 8_000, 12_000, 20_000];
pub const AUDIO_BITRATE_PRESETS: &[u32] = &[64, 96, 128, 160, 192, 256, 320];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Matroska,
    WebM,
}

impl Container {
    pub const ALL: [Self; 3] = [Self::Mp4, Self::Matroska, Self::WebM];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Matroska => "mkv",
            Self::WebM => "webm",
        }
    }

    pub const fn muxer(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Matroska => "matroska",
            Self::WebM => "webm",
        }
    }
}

impl fmt::Display for Container {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mp4 => "MP4",
            Self::Matroska => "Matroska (MKV)",
            Self::WebM => "WebM",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    H265,
    Av1,
    Vp9,
}

impl VideoCodec {
    pub const ALL: [Self; 4] = [Self::H264, Self::H265, Self::Av1, Self::Vp9];

    pub const fn encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::H265 => "libx265",
            Self::Av1 => "libsvtav1",
            Self::Vp9 => "libvpx-vp9",
        }
    }
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::H264 => "H.264 (libx264)",
            Self::H265 => "H.265 (libx265)",
            Self::Av1 => "AV1 (SVT-AV1)",
            Self::Vp9 => "VP9 (libvpx-vp9)",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    Aac,
    Opus,
    Mp3,
    None,
}

impl AudioCodec {
    pub const ALL: [Self; 4] = [Self::Aac, Self::Opus, Self::Mp3, Self::None];

    pub const fn encoder(self) -> Option<&'static str> {
        match self {
            Self::Aac => Some("aac"),
            Self::Opus => Some("libopus"),
            Self::Mp3 => Some("libmp3lame"),
            Self::None => None,
        }
    }
}

impl fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Aac => "AAC",
            Self::Opus => "Opus",
            Self::Mp3 => "MP3",
            Self::None => "None",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Source,
    P2160,
    P1440,
    P1080,
    P720,
    P480,
}

impl Resolution {
    pub const ALL: [Self; 6] = [
        Self::Source,
        Self::P2160,
        Self::P1440,
        Self::P1080,
        Self::P720,
        Self::P480,
    ];

    pub const fn canvas(self) -> Option<(u16, u16)> {
        match self {
            Self::Source => None,
            Self::P2160 => Some((3840, 2160)),
            Self::P1440 => Some((2560, 1440)),
            Self::P1080 => Some((1920, 1080)),
            Self::P720 => Some((1280, 720)),
            Self::P480 => Some((854, 480)),
        }
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Source => "Source",
            Self::P2160 => "2160p (4K)",
            Self::P1440 => "1440p",
            Self::P1080 => "1080p",
            Self::P720 => "720p",
            Self::P480 => "480p",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateControlMode {
    Quality,
    Bitrate,
}

impl fmt::Display for RateControlMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Quality => "Quality (CRF)",
            Self::Bitrate => "Target bitrate",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityPreset {
    High,
    Balanced,
    Small,
}

impl QualityPreset {
    pub const ALL: [Self; 3] = [Self::High, Self::Balanced, Self::Small];
}

impl fmt::Display for QualityPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::High => "High",
            Self::Balanced => "Balanced",
            Self::Small => "Small file",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoRateControl {
    Quality(QualityPreset),
    Bitrate(u32),
}

#[derive(Debug, Clone)]
pub struct DraftConfig {
    pub input: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub container: Container,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub resolution: Resolution,
    pub rate_control_mode: RateControlMode,
    pub quality: QualityPreset,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
}

impl Default for DraftConfig {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            container: Container::Mp4,
            video_codec: VideoCodec::H264,
            audio_codec: AudioCodec::Aac,
            resolution: Resolution::Source,
            rate_control_mode: RateControlMode::Quality,
            quality: QualityPreset::Balanced,
            video_bitrate_kbps: 5_000,
            audio_bitrate_kbps: 192,
        }
    }
}

impl DraftConfig {
    pub fn normalize_for_container(&mut self) {
        if !supported_video_codecs(self.container).contains(&self.video_codec) {
            self.video_codec = default_video_codec(self.container);
        }
        if !supported_audio_codecs(self.container).contains(&self.audio_codec) {
            self.audio_codec = default_audio_codec(self.container);
        }
        if let Some(output) = self.output.as_mut() {
            output.set_extension(self.container.extension());
        }
    }

    pub fn validated(&self, media: &InputMedia) -> Result<TranscodeConfig, ValidationError> {
        let input = self.input.clone().ok_or(ValidationError::MissingInput)?;
        let output = self.output.clone().ok_or(ValidationError::MissingOutput)?;
        if !input.exists() {
            return Err(ValidationError::InputMissing);
        }
        if output.exists() {
            return Err(ValidationError::OutputExists);
        }
        if input == output {
            return Err(ValidationError::SameInputAndOutput);
        }
        let parent = output
            .parent()
            .ok_or(ValidationError::InvalidOutputDirectory)?;
        if !parent.is_dir() {
            return Err(ValidationError::InvalidOutputDirectory);
        }
        if !supported_video_codecs(self.container).contains(&self.video_codec)
            || !supported_audio_codecs(self.container).contains(&self.audio_codec)
        {
            return Err(ValidationError::IncompatibleCodecs);
        }
        if !(100..=200_000).contains(&self.video_bitrate_kbps) {
            return Err(ValidationError::InvalidVideoBitrate);
        }
        if self.audio_codec != AudioCodec::None
            && media.audio.is_some()
            && !(32..=512).contains(&self.audio_bitrate_kbps)
        {
            return Err(ValidationError::InvalidAudioBitrate);
        }

        let video_rate_control = match self.rate_control_mode {
            RateControlMode::Quality => VideoRateControl::Quality(self.quality),
            RateControlMode::Bitrate => VideoRateControl::Bitrate(self.video_bitrate_kbps),
        };

        Ok(TranscodeConfig {
            input,
            output,
            container: self.container,
            video_codec: self.video_codec,
            audio_codec: if media.audio.is_some() {
                self.audio_codec
            } else {
                AudioCodec::None
            },
            resolution: self.resolution,
            video_rate_control,
            audio_bitrate_kbps: self.audio_bitrate_kbps,
        })
    }
}

#[derive(Debug, Clone)]
pub struct TranscodeConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub container: Container,
    pub video_codec: VideoCodec,
    pub audio_codec: AudioCodec,
    pub resolution: Resolution,
    pub video_rate_control: VideoRateControl,
    pub audio_bitrate_kbps: u32,
}

#[derive(Debug, Clone)]
pub struct InputMedia {
    pub path: PathBuf,
    pub duration: Option<Duration>,
    pub video: VideoStreamInfo,
    pub audio: Option<AudioStreamInfo>,
    pub format_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VideoStreamInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct AudioStreamInfo {
    pub codec: String,
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ValidationError {
    #[error("Select an input file.")]
    MissingInput,
    #[error("Select an output file.")]
    MissingOutput,
    #[error("The input file no longer exists.")]
    InputMissing,
    #[error("The output file already exists. Choose a new file name.")]
    OutputExists,
    #[error("The input and output paths must be different.")]
    SameInputAndOutput,
    #[error("The output directory does not exist.")]
    InvalidOutputDirectory,
    #[error("The selected container and codecs are incompatible.")]
    IncompatibleCodecs,
    #[error("Video bitrate must be between 100 and 200000 kbps.")]
    InvalidVideoBitrate,
    #[error("Audio bitrate must be between 32 and 512 kbps.")]
    InvalidAudioBitrate,
}

pub const fn supported_video_codecs(container: Container) -> &'static [VideoCodec] {
    match container {
        Container::Mp4 => &[VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1],
        Container::Matroska => &VideoCodec::ALL,
        Container::WebM => &[VideoCodec::Av1, VideoCodec::Vp9],
    }
}

pub const fn supported_audio_codecs(container: Container) -> &'static [AudioCodec] {
    match container {
        Container::Mp4 => &[AudioCodec::Aac, AudioCodec::None],
        Container::Matroska => &AudioCodec::ALL,
        Container::WebM => &[AudioCodec::Opus, AudioCodec::None],
    }
}

pub const fn default_video_codec(container: Container) -> VideoCodec {
    match container {
        Container::Mp4 | Container::Matroska => VideoCodec::H264,
        Container::WebM => VideoCodec::Vp9,
    }
}

pub const fn default_audio_codec(container: Container) -> AudioCodec {
    match container {
        Container::Mp4 | Container::Matroska => AudioCodec::Aac,
        Container::WebM => AudioCodec::Opus,
    }
}

pub const fn quality_crf(codec: VideoCodec, quality: QualityPreset) -> u8 {
    match (codec, quality) {
        (VideoCodec::H264, QualityPreset::High) => 18,
        (VideoCodec::H264, QualityPreset::Balanced) => 23,
        (VideoCodec::H264, QualityPreset::Small) => 28,
        (VideoCodec::H265, QualityPreset::High) => 20,
        (VideoCodec::H265, QualityPreset::Balanced) => 26,
        (VideoCodec::H265, QualityPreset::Small) => 30,
        (VideoCodec::Av1, QualityPreset::High) => 28,
        (VideoCodec::Av1, QualityPreset::Balanced) => 35,
        (VideoCodec::Av1, QualityPreset::Small) => 42,
        (VideoCodec::Vp9, QualityPreset::High) => 24,
        (VideoCodec::Vp9, QualityPreset::Balanced) => 32,
        (VideoCodec::Vp9, QualityPreset::Small) => 40,
    }
}

pub fn scale_filter(resolution: Resolution) -> Option<String> {
    resolution.canvas().map(|(width, height)| {
        format!(
            "scale=w='min({width},iw)':h='min({height},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"
        )
    })
}

pub fn suggested_output_path(input: &std::path::Path, container: Container) -> PathBuf {
    let parent = input.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    parent.join(format!("{stem}.transcoded.{}", container.extension()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_incompatible_webm_settings() {
        let mut draft = DraftConfig {
            container: Container::WebM,
            video_codec: VideoCodec::H264,
            audio_codec: AudioCodec::Aac,
            output: Some(PathBuf::from("movie.mp4")),
            ..DraftConfig::default()
        };

        draft.normalize_for_container();

        assert_eq!(draft.video_codec, VideoCodec::Vp9);
        assert_eq!(draft.audio_codec, AudioCodec::Opus);
        assert_eq!(draft.output, Some(PathBuf::from("movie.webm")));
    }

    #[test]
    fn quality_values_are_codec_specific() {
        assert_eq!(quality_crf(VideoCodec::H264, QualityPreset::Balanced), 23);
        assert_eq!(quality_crf(VideoCodec::Av1, QualityPreset::Balanced), 35);
        assert_eq!(quality_crf(VideoCodec::Vp9, QualityPreset::Small), 40);
    }

    #[test]
    fn scale_preserves_aspect_ratio_and_prevents_upscale() {
        let filter = scale_filter(Resolution::P1080).unwrap();
        assert!(filter.contains("min(1920,iw)"));
        assert!(filter.contains("min(1080,ih)"));
        assert!(filter.contains("force_divisible_by=2"));
    }

    #[test]
    fn suggests_container_extension() {
        let output = suggested_output_path(
            std::path::Path::new("/tmp/My Clip.mov"),
            Container::Matroska,
        );
        assert_eq!(output, PathBuf::from("/tmp/My Clip.transcoded.mkv"));
    }
}
