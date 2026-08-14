use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use serde::Deserialize;
use thiserror::Error;

use crate::domain::{AudioStreamInfo, InputMedia, VideoStreamInfo};

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

    parse_probe_json(input.to_owned(), &output.stdout)
}

fn parse_probe_json(path: PathBuf, bytes: &[u8]) -> Result<InputMedia, ProbeError> {
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
        .map(|stream| AudioStreamInfo {
            codec: stream
                .codec_name
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            channels: stream.channels,
            sample_rate: stream.sample_rate.as_deref().and_then(parse_u32),
        });
    let duration = response
        .format
        .as_ref()
        .and_then(|format| format.duration.as_deref())
        .and_then(parse_duration)
        .or_else(|| video.duration.as_deref().and_then(parse_duration));

    Ok(InputMedia {
        path,
        duration,
        video: VideoStreamInfo {
            codec: video
                .codec_name
                .clone()
                .unwrap_or_else(|| "unknown".to_owned()),
            width,
            height,
        },
        audio,
        format_name: response.format.and_then(|format| format.format_name),
    })
}

fn parse_duration(value: &str) -> Option<Duration> {
    let seconds = value.parse::<f64>().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

fn parse_u32(value: &str) -> Option<u32> {
    value.parse().ok()
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
    channels: Option<u32>,
    sample_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    duration: Option<String>,
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

        let media = parse_probe_json(PathBuf::from("clip.mp4"), json).unwrap();

        assert_eq!(media.video.width, 1920);
        assert_eq!(media.video.height, 1080);
        assert_eq!(media.audio.unwrap().sample_rate, Some(48_000));
        assert_eq!(media.duration, Some(Duration::from_secs_f64(12.5)));
    }

    #[test]
    fn accepts_missing_audio_and_unknown_duration() {
        let json = br#"{
            "streams": [{"codec_type":"video","codec_name":"vp9","width":1280,"height":720}],
            "format":{"format_name":"matroska","duration":"N/A"}
        }"#;

        let media = parse_probe_json(PathBuf::from("clip.mkv"), json).unwrap();

        assert!(media.audio.is_none());
        assert!(media.duration.is_none());
    }
}
