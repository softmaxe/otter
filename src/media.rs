use std::{path::Path, process::Command, time::Duration};

use serde::Deserialize;
use thiserror::Error;

use crate::domain::{InputMedia, VideoStreamInfo};

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("The selected input file does not exist.")]
    InputMissing,
    #[error("Failed to start ffprobe: {0}")]
    Invocation(#[from] std::io::Error),
    #[error("ffprobe failed: {0}")]
    Failed(String),
    #[error("ffprobe returned invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("The selected file does not contain a video stream.")]
    NoVideoStream,
    #[error("The primary video stream has invalid dimensions.")]
    InvalidDimensions,
}

pub fn probe_media(ffprobe: &Path, input: &Path) -> Result<InputMedia, ProbeError> {
    if !input.is_file() {
        return Err(ProbeError::InputMissing);
    }

    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-show_format",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(input)
        .output()?;
    if !output.status.success() {
        return Err(ProbeError::Failed(sanitize_message(
            &String::from_utf8_lossy(&output.stderr),
        )));
    }

    parse_probe_json(&output.stdout)
}

fn parse_probe_json(bytes: &[u8]) -> Result<InputMedia, ProbeError> {
    let response: ProbeResponse = serde_json::from_slice(bytes)?;
    let video = response
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .ok_or(ProbeError::NoVideoStream)?;
    let width = video
        .width
        .filter(|value| *value > 0)
        .ok_or(ProbeError::InvalidDimensions)?;
    let height = video
        .height
        .filter(|value| *value > 0)
        .ok_or(ProbeError::InvalidDimensions)?;
    let audio = response
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
        .map(|stream| codec_name(stream.codec_name.as_deref()));
    let duration = response
        .format
        .as_ref()
        .and_then(|format| format.duration.as_deref())
        .and_then(parse_duration)
        .or_else(|| video.duration.as_deref().and_then(parse_duration));
    // `avg_frame_rate` is the honest average for variable-rate sources; `r_frame_rate`
    // is the container's nominal rate and only fills in when the average is absent or
    // degenerate (ffprobe reports "0/0" for streams it cannot measure).
    let frame_rate = video
        .avg_frame_rate
        .as_deref()
        .and_then(parse_frame_rate)
        .or_else(|| video.r_frame_rate.as_deref().and_then(parse_frame_rate));

    Ok(InputMedia {
        duration,
        video: VideoStreamInfo {
            codec: codec_name(video.codec_name.as_deref()),
            width,
            height,
            frame_rate,
            bitrate_kbps: video.bit_rate.as_deref().and_then(parse_bitrate_kbps),
        },
        audio,
        bitrate_kbps: response
            .format
            .as_ref()
            .and_then(|format| format.bit_rate.as_deref())
            .and_then(parse_bitrate_kbps),
    })
}

fn codec_name(name: Option<&str>) -> String {
    name.unwrap_or("unknown").to_owned()
}

/// ffprobe reports frame rates as the rational `num/den`, using `0/0` when unknown.
fn parse_frame_rate(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.trim().parse::<f64>().ok()?;
    let denominator = denominator.trim().parse::<f64>().ok()?;
    let rate = numerator / denominator;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

fn parse_bitrate_kbps(value: &str) -> Option<u32> {
    let bits_per_second = value.parse::<u64>().ok()?;
    (bits_per_second > 0).then(|| u32::try_from(bits_per_second / 1_000).unwrap_or(u32::MAX))
}

fn parse_duration(value: &str) -> Option<Duration> {
    let seconds = value.parse::<f64>().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

fn sanitize_message(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect::<String>()
        .trim()
        .chars()
        .take(2_000)
        .collect()
}

#[derive(Debug, Deserialize)]
struct ProbeResponse {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
    bit_rate: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_audio_and_duration() {
        let json = br#"{
            "streams": [
                {"codec_type":"video","codec_name":"h264","width":1920,"height":1080},
                {"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000"}
            ],
            "format":{"format_name":"mov,mp4","duration":"12.5"}
        }"#;

        let media = parse_probe_json(json).unwrap();

        assert_eq!(media.video.width, 1920);
        assert_eq!(media.video.height, 1080);
        assert_eq!(media.audio.as_deref(), Some("aac"));
        assert_eq!(media.duration, Some(Duration::from_secs_f64(12.5)));
    }

    #[test]
    fn captures_the_fields_the_size_estimate_needs() {
        let json = br#"{
            "streams": [
                {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
                 "bit_rate":"7500000","r_frame_rate":"30000/1001","avg_frame_rate":"24000/1001"},
                {"codec_type":"audio","codec_name":"aac","channels":2}
            ],
            "format":{"format_name":"mov,mp4","duration":"60","bit_rate":"7692000","size":"57690000"}
        }"#;

        let media = parse_probe_json(json).unwrap();

        // The measured average wins over the container's nominal rate.
        assert_eq!(media.video.frame_rate, Some(24_000.0 / 1_001.0));
        assert_eq!(media.video.bitrate_kbps, Some(7_500));
        assert_eq!(media.bitrate_kbps, Some(7_692));
    }

    #[test]
    fn tolerates_unmeasurable_frame_rates_and_bitrates() {
        let json = br#"{
            "streams": [
                {"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
                 "bit_rate":"N/A","avg_frame_rate":"0/0","r_frame_rate":"25/1"}
            ],
            "format":{"format_name":"matroska","duration":"60","bit_rate":"N/A","size":"N/A"}
        }"#;

        let media = parse_probe_json(json).unwrap();

        // `0/0` is ffprobe's "unknown", so the nominal rate fills in.
        assert_eq!(media.video.frame_rate, Some(25.0));
        assert_eq!(media.video.bitrate_kbps, None);
        assert_eq!(media.bitrate_kbps, None);
    }

    #[test]
    fn accepts_missing_audio_and_unknown_duration() {
        let json = br#"{
            "streams": [{"codec_type":"video","codec_name":"vp9","width":1280,"height":720}],
            "format":{"format_name":"matroska","duration":"N/A"}
        }"#;

        let media = parse_probe_json(json).unwrap();

        assert!(media.audio.is_none());
        assert!(media.duration.is_none());
    }
}
