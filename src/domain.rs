use std::{fmt, path::PathBuf, time::Duration};

use thiserror::Error;

pub const VIDEO_BITRATE_PRESETS: &[u32] = &[1_000, 2_500, 5_000, 8_000, 12_000, 20_000];
pub const AUDIO_BITRATE_PRESETS: &[u32] = &[64, 96, 128, 160, 192, 256, 320];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mov,
    Matroska,
    WebM,
}

impl Container {
    pub const ALL: [Self; 4] = [Self::Mp4, Self::Mov, Self::Matroska, Self::WebM];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Matroska => "mkv",
            Self::WebM => "webm",
        }
    }

    pub const fn muxer(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Matroska => "matroska",
            Self::WebM => "webm",
        }
    }
}

impl fmt::Display for Container {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mp4 => "MP4",
            Self::Mov => "MOV",
            Self::Matroska => "Matroska (MKV)",
            Self::WebM => "WebM",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    H264Hw,
    H265,
    H265Hw,
    Av1,
    Vp9,
}

impl VideoCodec {
    pub const ALL: [Self; 6] = [
        Self::H264,
        Self::H264Hw,
        Self::H265,
        Self::H265Hw,
        Self::Av1,
        Self::Vp9,
    ];

    pub const fn encoder(self) -> &'static str {
        match self {
            Self::H264 => "libx264",
            Self::H264Hw => "h264_videotoolbox",
            Self::H265 => "libx265",
            Self::H265Hw => "hevc_videotoolbox",
            Self::Av1 => "libsvtav1",
            Self::Vp9 => "libvpx-vp9",
        }
    }

    /// VideoToolbox encoders run on the Apple media engine instead of the CPU.
    pub const fn is_hardware(self) -> bool {
        matches!(self, Self::H264Hw | Self::H265Hw)
    }

    /// HEVC needs the `hvc1` tag to stay playable in QuickTime containers.
    pub const fn is_hevc(self) -> bool {
        matches!(self, Self::H265 | Self::H265Hw)
    }
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::H264 => "H.264 (libx264, CPU)",
            Self::H264Hw => "H.264 (VideoToolbox, GPU)",
            Self::H265 => "H.265 (libx265, CPU)",
            Self::H265Hw => "H.265 (VideoToolbox, GPU)",
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
            Self::Quality => "Constant quality",
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
    /// Container size on disk, when ffprobe reported it.
    pub size_bytes: Option<u64>,
    /// Overall container bitrate, covering every stream plus muxing overhead.
    pub bitrate_kbps: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct VideoStreamInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    /// Frames per second, absent when ffprobe could not measure the stream.
    pub frame_rate: Option<f64>,
    /// Bitrate of this stream alone; many containers omit it.
    pub bitrate_kbps: Option<u32>,
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
        Container::Mp4 | Container::Mov => &[
            VideoCodec::H264,
            VideoCodec::H264Hw,
            VideoCodec::H265,
            VideoCodec::H265Hw,
            VideoCodec::Av1,
        ],
        Container::Matroska => &VideoCodec::ALL,
        Container::WebM => &[VideoCodec::Av1, VideoCodec::Vp9],
    }
}

pub const fn supported_audio_codecs(container: Container) -> &'static [AudioCodec] {
    match container {
        Container::Mp4 | Container::Mov => &[AudioCodec::Aac, AudioCodec::None],
        Container::Matroska => &AudioCodec::ALL,
        Container::WebM => &[AudioCodec::Opus, AudioCodec::None],
    }
}

pub const fn default_video_codec(container: Container) -> VideoCodec {
    match container {
        Container::Mp4 | Container::Mov | Container::Matroska => VideoCodec::H264,
        Container::WebM => VideoCodec::Vp9,
    }
}

pub const fn default_audio_codec(container: Container) -> AudioCodec {
    match container {
        Container::Mp4 | Container::Mov | Container::Matroska => AudioCodec::Aac,
        Container::WebM => AudioCodec::Opus,
    }
}

/// The constant-quality flag and value that drive one encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualitySetting {
    /// `-crf` for software encoders, `-q:v` for VideoToolbox.
    pub flag: &'static str,
    pub value: u8,
}

impl fmt::Display for QualitySetting {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = if self.flag == "-crf" { "CRF" } else { "Q" };
        write!(f, "{name} {}", self.value)
    }
}

/// Software encoders take a CRF where lower is better. VideoToolbox has no CRF; it
/// exposes an inverted 1–100 constant-quality scale through `-q:v`. The hardware
/// values below were calibrated against the software presets with VMAF so that both
/// paths land on comparable quality for the same preset.
pub const fn quality_setting(codec: VideoCodec, quality: QualityPreset) -> QualitySetting {
    let (flag, value) = match (codec, quality) {
        (VideoCodec::H264, QualityPreset::High) => ("-crf", 18),
        (VideoCodec::H264, QualityPreset::Balanced) => ("-crf", 23),
        (VideoCodec::H264, QualityPreset::Small) => ("-crf", 28),
        (VideoCodec::H264Hw, QualityPreset::High) => ("-q:v", 70),
        (VideoCodec::H264Hw, QualityPreset::Balanced) => ("-q:v", 60),
        (VideoCodec::H264Hw, QualityPreset::Small) => ("-q:v", 47),
        (VideoCodec::H265, QualityPreset::High) => ("-crf", 20),
        (VideoCodec::H265, QualityPreset::Balanced) => ("-crf", 26),
        (VideoCodec::H265, QualityPreset::Small) => ("-crf", 30),
        (VideoCodec::H265Hw, QualityPreset::High) => ("-q:v", 65),
        (VideoCodec::H265Hw, QualityPreset::Balanced) => ("-q:v", 55),
        (VideoCodec::H265Hw, QualityPreset::Small) => ("-q:v", 47),
        (VideoCodec::Av1, QualityPreset::High) => ("-crf", 28),
        (VideoCodec::Av1, QualityPreset::Balanced) => ("-crf", 35),
        (VideoCodec::Av1, QualityPreset::Small) => ("-crf", 42),
        (VideoCodec::Vp9, QualityPreset::High) => ("-crf", 24),
        (VideoCodec::Vp9, QualityPreset::Balanced) => ("-crf", 32),
        (VideoCodec::Vp9, QualityPreset::Small) => ("-crf", 40),
    };
    QualitySetting { flag, value }
}

pub fn scale_filter(resolution: Resolution) -> Option<String> {
    resolution.canvas().map(|(width, height)| {
        format!(
            "scale=w='min({width},iw)':h='min({height},ih)':force_original_aspect_ratio=decrease:force_divisible_by=2"
        )
    })
}

/// Muxing overhead — index tables, per-packet headers, interleaving padding — that
/// the raw stream bitrates do not account for.
const CONTAINER_OVERHEAD: f64 = 1.02;

/// Pixel count the bits-per-pixel table is calibrated against (1080p).
const REFERENCE_PIXELS: f64 = 1920.0 * 1080.0;

/// Frame rate assumed when ffprobe could not measure the source.
const FALLBACK_FRAME_RATE: f64 = 30.0;

/// Constant-quality encoders spend fewer bits per pixel as the frame grows, because
/// detail that mattered at 480p is invisible at 4K. Modelling that falloff as
/// `(pixels / REFERENCE_PIXELS) ^ -exponent` keeps one table usable across the whole
/// resolution ladder instead of needing a row per resolution.
const RESOLUTION_FALLOFF_EXPONENT: f64 = 0.25;

/// How large the configured output is expected to be, and how much to trust it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeEstimate {
    pub bytes: u64,
    pub basis: EstimateBasis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateBasis {
    /// Read off the bitrate the user asked for. Accurate to a few percent.
    Targeted,
    /// Inferred from the bits-per-pixel model. A guide, not a promise.
    Heuristic,
}

impl SizeEstimate {
    /// The settings-panel value, marking heuristic numbers so an inferred figure is
    /// never mistaken for a computed one.
    pub fn label(self) -> String {
        match self.basis {
            EstimateBasis::Targeted => format!("~{}", format_size(self.bytes)),
            EstimateBasis::Heuristic => format!("~{} (rough)", format_size(self.bytes)),
        }
    }
}

/// Predicts the size of the file this draft would produce.
///
/// Returns `None` when the source duration is unknown, because every path through the
/// model multiplies by it — showing nothing beats showing a fabricated number.
pub fn estimate_output_size(draft: &DraftConfig, media: &InputMedia) -> Option<SizeEstimate> {
    let seconds = media.duration?.as_secs_f64();
    if seconds <= 0.0 || !seconds.is_finite() {
        return None;
    }
    let audio_kbps = if draft.audio_codec == AudioCodec::None || media.audio.is_none() {
        0.0
    } else {
        f64::from(draft.audio_bitrate_kbps)
    };
    let (video_kbps, basis) = match draft.rate_control_mode {
        RateControlMode::Bitrate => (f64::from(draft.video_bitrate_kbps), EstimateBasis::Targeted),
        RateControlMode::Quality => (
            quality_video_bitrate_kbps(draft, media),
            EstimateBasis::Heuristic,
        ),
    };
    let bits = (video_kbps + audio_kbps) * 1_000.0 * seconds * CONTAINER_OVERHEAD;
    Some(SizeEstimate {
        bytes: (bits / 8.0) as u64,
        basis,
    })
}

/// Bits per pixel per frame each encoder spends at 1080p on typical live-action
/// content, for the constant-quality settings in [`quality_setting`].
///
/// These are published rules of thumb rather than measurements from this project's own
/// encodes. Content dominates every term in the model — a static screen recording
/// lands far below these figures and heavy grain far above — so the result is a guide.
/// Recalibrating the quality estimate means editing this table and nothing else.
fn quality_bits_per_pixel(codec: VideoCodec, quality: QualityPreset) -> f64 {
    match (codec, quality) {
        (VideoCodec::H264, QualityPreset::High) => 0.150,
        (VideoCodec::H264, QualityPreset::Balanced) => 0.075,
        (VideoCodec::H264, QualityPreset::Small) => 0.038,
        // VideoToolbox trades efficiency for speed: matching the software presets on
        // quality costs it roughly half again as many bits.
        (VideoCodec::H264Hw, QualityPreset::High) => 0.230,
        (VideoCodec::H264Hw, QualityPreset::Balanced) => 0.115,
        (VideoCodec::H264Hw, QualityPreset::Small) => 0.058,
        (VideoCodec::H265, QualityPreset::High) => 0.100,
        (VideoCodec::H265, QualityPreset::Balanced) => 0.048,
        (VideoCodec::H265, QualityPreset::Small) => 0.030,
        (VideoCodec::H265Hw, QualityPreset::High) => 0.145,
        (VideoCodec::H265Hw, QualityPreset::Balanced) => 0.070,
        (VideoCodec::H265Hw, QualityPreset::Small) => 0.042,
        (VideoCodec::Av1, QualityPreset::High) => 0.085,
        (VideoCodec::Av1, QualityPreset::Balanced) => 0.042,
        (VideoCodec::Av1, QualityPreset::Small) => 0.022,
        (VideoCodec::Vp9, QualityPreset::High) => 0.105,
        (VideoCodec::Vp9, QualityPreset::Balanced) => 0.050,
        (VideoCodec::Vp9, QualityPreset::Small) => 0.026,
    }
}

fn quality_video_bitrate_kbps(draft: &DraftConfig, media: &InputMedia) -> f64 {
    let (width, height) = output_dimensions(draft.resolution, &media.video);
    let pixels = f64::from(width) * f64::from(height);
    let frame_rate = media.video.frame_rate.unwrap_or(FALLBACK_FRAME_RATE);
    let falloff = (pixels / REFERENCE_PIXELS).powf(-RESOLUTION_FALLOFF_EXPONENT);
    let kbps =
        quality_bits_per_pixel(draft.video_codec, draft.quality) * falloff * pixels * frame_rate
            / 1_000.0;

    match source_video_bitrate_kbps(media) {
        // A lossy re-encode that does not enlarge the frame practically never needs
        // more bits than the source already spent, so the source doubles as a ceiling
        // and pulls the estimate back down for already-compressed inputs. It is the
        // wrong ceiling in exactly one case: a high-quality preset applied to a badly
        // compressed source, where faithfully preserving the existing artifacts really
        // does cost more than the original. That case is why this is a guide.
        Some(ceiling) if width <= media.video.width && height <= media.video.height => {
            kbps.min(f64::from(ceiling))
        }
        _ => kbps,
    }
}

/// The source's video bitrate, falling back to the whole-container figure when the
/// stream carries none. Video dominates every file this tool accepts, so the container
/// bitrate is a close enough stand-in.
fn source_video_bitrate_kbps(media: &InputMedia) -> Option<u32> {
    media.video.bitrate_kbps.or(media.bitrate_kbps)
}

/// The frame size the encoder will actually see, mirroring [`scale_filter`]: fit inside
/// the target canvas, preserve the aspect ratio, never upscale, keep both sides even.
pub fn output_dimensions(resolution: Resolution, source: &VideoStreamInfo) -> (u32, u32) {
    let Some((canvas_width, canvas_height)) = resolution.canvas() else {
        return (source.width, source.height);
    };
    let canvas_width = u32::from(canvas_width);
    let canvas_height = u32::from(canvas_height);
    if source.width <= canvas_width && source.height <= canvas_height {
        return (source.width, source.height);
    }
    let scale = f64::min(
        f64::from(canvas_width) / f64::from(source.width),
        f64::from(canvas_height) / f64::from(source.height),
    );
    (
        round_to_even(f64::from(source.width) * scale),
        round_to_even(f64::from(source.height) * scale),
    )
}

/// `force_divisible_by=2` in the scale filter; encoders reject odd dimensions.
fn round_to_even(value: f64) -> u32 {
    let rounded = value.round().max(2.0) as u32;
    rounded - rounded % 2
}

/// Formats a byte count the way file managers and ffmpeg report one: SI units, with
/// the tier chosen so the number stays short enough to sit in a settings row.
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = KB * KB;
    const GB: f64 = MB * KB;

    let bytes = bytes as f64;
    if bytes < MB {
        format!("{:.0} KB", (bytes / KB).max(1.0))
    } else if bytes < GB {
        format!("{:.1} MB", bytes / MB)
    } else {
        format!("{:.2} GB", bytes / GB)
    }
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
    fn normalizes_incompatible_mov_settings() {
        let mut draft = DraftConfig {
            container: Container::Mov,
            video_codec: VideoCodec::Vp9,
            audio_codec: AudioCodec::Opus,
            output: Some(PathBuf::from("movie.webm")),
            ..DraftConfig::default()
        };

        draft.normalize_for_container();

        assert_eq!(draft.video_codec, VideoCodec::H264);
        assert_eq!(draft.audio_codec, AudioCodec::Aac);
        assert_eq!(draft.output, Some(PathBuf::from("movie.mov")));
    }

    #[test]
    fn mov_uses_mp4_codec_policy() {
        assert_eq!(
            Container::ALL,
            [
                Container::Mp4,
                Container::Mov,
                Container::Matroska,
                Container::WebM,
            ]
        );
        assert_eq!(Container::Mov.extension(), "mov");
        assert_eq!(Container::Mov.muxer(), "mov");
        assert_eq!(Container::Mov.to_string(), "MOV");
        assert_eq!(
            supported_video_codecs(Container::Mov),
            supported_video_codecs(Container::Mp4)
        );
        assert_eq!(
            supported_audio_codecs(Container::Mov),
            supported_audio_codecs(Container::Mp4)
        );
        assert_eq!(default_video_codec(Container::Mov), VideoCodec::H264);
        assert_eq!(default_audio_codec(Container::Mov), AudioCodec::Aac);
    }

    #[test]
    fn quality_values_are_codec_specific() {
        let balanced_h264 = quality_setting(VideoCodec::H264, QualityPreset::Balanced);
        assert_eq!(balanced_h264.flag, "-crf");
        assert_eq!(balanced_h264.value, 23);
        assert_eq!(balanced_h264.to_string(), "CRF 23");
        assert_eq!(
            quality_setting(VideoCodec::Av1, QualityPreset::Balanced).value,
            35
        );
        assert_eq!(
            quality_setting(VideoCodec::Vp9, QualityPreset::Small).value,
            40
        );
    }

    #[test]
    fn hardware_encoders_use_an_inverted_quality_scale() {
        for codec in [VideoCodec::H264Hw, VideoCodec::H265Hw] {
            assert!(codec.is_hardware());
            let mut previous = u8::MAX;
            for quality in QualityPreset::ALL {
                let setting = quality_setting(codec, quality);
                assert_eq!(setting.flag, "-q:v");
                // Higher `-q:v` means better quality, so the presets must descend.
                assert!(
                    setting.value < previous,
                    "{codec} {quality} is not descending"
                );
                previous = setting.value;
            }
        }
        assert_eq!(
            quality_setting(VideoCodec::H264Hw, QualityPreset::Balanced).to_string(),
            "Q 60"
        );
        assert!(!VideoCodec::H264.is_hardware());
        assert!(VideoCodec::H265Hw.is_hevc());
        assert!(!VideoCodec::H264Hw.is_hevc());
    }

    #[test]
    fn quicktime_containers_offer_both_hardware_and_software_h26x() {
        for container in [Container::Mp4, Container::Mov] {
            let codecs = supported_video_codecs(container);
            for codec in [
                VideoCodec::H264,
                VideoCodec::H264Hw,
                VideoCodec::H265,
                VideoCodec::H265Hw,
            ] {
                assert!(codecs.contains(&codec), "{container} should allow {codec}");
            }
        }
        // VideoToolbox cannot produce VP9 or AV1, so WebM stays software-only.
        assert!(
            !supported_video_codecs(Container::WebM)
                .iter()
                .any(|codec| codec.is_hardware())
        );
    }

    #[test]
    fn scale_preserves_aspect_ratio_and_prevents_upscale() {
        let filter = scale_filter(Resolution::P1080).unwrap();
        assert!(filter.contains("min(1920,iw)"));
        assert!(filter.contains("min(1080,ih)"));
        assert!(filter.contains("force_divisible_by=2"));
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
                // High enough that the source ceiling never binds unless a test
                // deliberately lowers it.
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

    #[test]
    fn target_bitrate_estimate_follows_the_requested_bitrate() {
        let draft = DraftConfig {
            rate_control_mode: RateControlMode::Bitrate,
            video_bitrate_kbps: 5_000,
            audio_bitrate_kbps: 192,
            ..DraftConfig::default()
        };

        let estimate = estimate_output_size(&draft, &probed_media()).unwrap();

        assert_eq!(estimate.basis, EstimateBasis::Targeted);
        // (5000 + 192) kbps over 10 s, plus 2% muxing overhead.
        let expected = (5_192.0 * 1_000.0 * 10.0 * 1.02 / 8.0) as u64;
        assert_eq!(estimate.bytes, expected);
        assert_eq!(estimate.label(), "~6.6 MB");
    }

    #[test]
    fn doubling_the_bitrate_doubles_the_estimate() {
        let media = probed_media();
        let base = DraftConfig {
            rate_control_mode: RateControlMode::Bitrate,
            video_bitrate_kbps: 5_000,
            audio_codec: AudioCodec::None,
            ..DraftConfig::default()
        };
        let doubled = DraftConfig {
            video_bitrate_kbps: 10_000,
            ..base.clone()
        };

        let small = estimate_output_size(&base, &media).unwrap().bytes;
        let large = estimate_output_size(&doubled, &media).unwrap().bytes;

        assert_eq!(large, small * 2);
    }

    #[test]
    fn quality_estimate_is_marked_as_rough() {
        let draft = DraftConfig {
            rate_control_mode: RateControlMode::Quality,
            ..DraftConfig::default()
        };

        let estimate = estimate_output_size(&draft, &probed_media()).unwrap();

        assert_eq!(estimate.basis, EstimateBasis::Heuristic);
        assert!(
            estimate.label().ends_with("(rough)"),
            "heuristic labels must disclose themselves: {}",
            estimate.label()
        );
    }

    /// The bits-per-pixel table is meant to be recalibrated, so these assertions pin the
    /// relationships between its rows rather than the values themselves.
    #[test]
    fn quality_bits_per_pixel_ordering_holds_across_the_table() {
        for codec in VideoCodec::ALL {
            let high = quality_bits_per_pixel(codec, QualityPreset::High);
            let balanced = quality_bits_per_pixel(codec, QualityPreset::Balanced);
            let small = quality_bits_per_pixel(codec, QualityPreset::Small);
            assert!(
                high > balanced && balanced > small,
                "{codec} is not ordered"
            );
        }
        for quality in QualityPreset::ALL {
            // Newer codecs reach the same quality with fewer bits.
            assert!(
                quality_bits_per_pixel(VideoCodec::H265, quality)
                    < quality_bits_per_pixel(VideoCodec::H264, quality)
            );
            assert!(
                quality_bits_per_pixel(VideoCodec::Av1, quality)
                    < quality_bits_per_pixel(VideoCodec::H265, quality)
            );
            // VideoToolbox buys speed with bitrate.
            assert!(
                quality_bits_per_pixel(VideoCodec::H264Hw, quality)
                    > quality_bits_per_pixel(VideoCodec::H264, quality)
            );
            assert!(
                quality_bits_per_pixel(VideoCodec::H265Hw, quality)
                    > quality_bits_per_pixel(VideoCodec::H265, quality)
            );
        }
    }

    #[test]
    fn balanced_h264_lands_in_a_plausible_band_for_1080p30() {
        let draft = DraftConfig {
            rate_control_mode: RateControlMode::Quality,
            audio_codec: AudioCodec::None,
            ..DraftConfig::default()
        };

        let bytes = estimate_output_size(&draft, &probed_media()).unwrap().bytes;
        let megabits_per_second = bytes as f64 * 8.0 / 10.0 / 1_000_000.0;

        // x264 CRF 23 at 1080p30 sits around 4-5 Mbps on live-action footage. A wide
        // band keeps the table tunable while still catching an order-of-magnitude slip.
        assert!(
            (2.0..8.0).contains(&megabits_per_second),
            "implausible 1080p30 estimate: {megabits_per_second} Mbps"
        );
    }

    #[test]
    fn downscaling_shrinks_the_quality_estimate() {
        let media = probed_media();
        let source = DraftConfig {
            rate_control_mode: RateControlMode::Quality,
            resolution: Resolution::Source,
            ..DraftConfig::default()
        };
        let downscaled = DraftConfig {
            resolution: Resolution::P480,
            ..source.clone()
        };

        assert!(
            estimate_output_size(&downscaled, &media).unwrap().bytes
                < estimate_output_size(&source, &media).unwrap().bytes
        );
    }

    #[test]
    fn source_bitrate_caps_the_quality_estimate() {
        let mut media = probed_media();
        media.video.bitrate_kbps = Some(800);
        let draft = DraftConfig {
            rate_control_mode: RateControlMode::Quality,
            quality: QualityPreset::High,
            audio_codec: AudioCodec::None,
            ..DraftConfig::default()
        };

        let bytes = estimate_output_size(&draft, &media).unwrap().bytes;

        // The uncapped model would ask for several megabits; the source only ever spent
        // 800 kbps, so the estimate is pulled down to that ceiling.
        assert_eq!(bytes, (800.0 * 1_000.0 * 10.0 * 1.02 / 8.0) as u64);
    }

    #[test]
    fn upscaling_ignores_the_source_ceiling() {
        let mut media = probed_media();
        media.video.width = 640;
        media.video.height = 360;
        media.video.bitrate_kbps = Some(800);
        let draft = DraftConfig {
            rate_control_mode: RateControlMode::Quality,
            resolution: Resolution::Source,
            audio_codec: AudioCodec::None,
            ..DraftConfig::default()
        };

        // The scale filter never upscales, so a 640x360 source stays at its own size
        // and the ceiling still applies.
        let bytes = estimate_output_size(&draft, &media).unwrap().bytes;
        assert_eq!(bytes, (800.0 * 1_000.0 * 10.0 * 1.02 / 8.0) as u64);
    }

    #[test]
    fn silent_output_drops_the_audio_bitrate() {
        let media = probed_media();
        let with_audio = DraftConfig {
            rate_control_mode: RateControlMode::Bitrate,
            video_bitrate_kbps: 5_000,
            audio_bitrate_kbps: 192,
            ..DraftConfig::default()
        };
        let muted = DraftConfig {
            audio_codec: AudioCodec::None,
            ..with_audio.clone()
        };

        let mut silent_source = media.clone();
        silent_source.audio = None;

        let audible = estimate_output_size(&with_audio, &media).unwrap().bytes;
        let muted_bytes = estimate_output_size(&muted, &media).unwrap().bytes;
        let no_track = estimate_output_size(&with_audio, &silent_source)
            .unwrap()
            .bytes;

        assert!(muted_bytes < audible);
        // A source without an audio track cannot gain one, whatever the draft says.
        assert_eq!(no_track, muted_bytes);
    }

    #[test]
    fn estimates_need_a_duration() {
        let mut media = probed_media();
        media.duration = None;

        assert!(estimate_output_size(&DraftConfig::default(), &media).is_none());

        media.duration = Some(Duration::ZERO);
        assert!(estimate_output_size(&DraftConfig::default(), &media).is_none());
    }

    #[test]
    fn missing_frame_rate_falls_back_instead_of_failing() {
        let mut media = probed_media();
        media.video.frame_rate = None;
        let draft = DraftConfig {
            rate_control_mode: RateControlMode::Quality,
            ..DraftConfig::default()
        };

        assert!(estimate_output_size(&draft, &media).is_some());
    }

    #[test]
    fn output_dimensions_mirror_the_scale_filter() {
        let source = |width, height| VideoStreamInfo {
            codec: "h264".to_owned(),
            width,
            height,
            frame_rate: Some(30.0),
            bitrate_kbps: None,
        };

        assert_eq!(
            output_dimensions(Resolution::P1080, &source(3840, 2160)),
            (1920, 1080)
        );
        // Never upscale.
        assert_eq!(
            output_dimensions(Resolution::P2160, &source(1280, 720)),
            (1280, 720)
        );
        assert_eq!(
            output_dimensions(Resolution::Source, &source(3840, 2160)),
            (3840, 2160)
        );
        // Fit the wider axis and keep both sides even.
        let (width, height) = output_dimensions(Resolution::P1080, &source(3000, 500));
        assert_eq!((width, height), (1920, 320));
        let (width, height) = output_dimensions(Resolution::P720, &source(1999, 1001));
        assert_eq!(width % 2, 0);
        assert_eq!(height % 2, 0);
        assert!(width <= 1280 && height <= 720);
    }

    #[test]
    fn formats_sizes_in_readable_tiers() {
        assert_eq!(format_size(0), "1 KB");
        assert_eq!(format_size(512_000), "512 KB");
        assert_eq!(format_size(6_619_800), "6.6 MB");
        assert_eq!(format_size(2_500_000_000), "2.50 GB");
    }

    #[test]
    fn suggests_container_extension() {
        let mkv_output = suggested_output_path(
            std::path::Path::new("/tmp/My Clip.mov"),
            Container::Matroska,
        );
        assert_eq!(mkv_output, PathBuf::from("/tmp/My Clip.transcoded.mkv"));

        let mov_output =
            suggested_output_path(std::path::Path::new("/tmp/My Clip.mp4"), Container::Mov);
        assert_eq!(mov_output, PathBuf::from("/tmp/My Clip.transcoded.mov"));
    }
}
