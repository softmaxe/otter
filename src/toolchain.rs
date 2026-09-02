use std::{
    collections::HashSet,
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

use crate::domain::{AudioCodec, VideoCodec};

/// The encoders this app can offer. Discovery only asks whether the local FFmpeg
/// build reports these; anything else it can do is irrelevant here.
fn encoder_names() -> impl Iterator<Item = &'static str> {
    VideoCodec::ALL
        .into_iter()
        .map(VideoCodec::encoder)
        .chain(AudioCodec::ALL.into_iter().filter_map(AudioCodec::encoder))
}

#[derive(Debug, Clone)]
pub struct Toolchain {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub ffmpeg_version: String,
    encoders: HashSet<String>,
}

impl Toolchain {
    pub fn discover() -> Result<Self, ToolError> {
        let ffmpeg = resolve_tool("OTTER_FFMPEG", "ffmpeg").ok_or(ToolError::NotFound("ffmpeg"))?;
        let ffprobe =
            resolve_tool("OTTER_FFPROBE", "ffprobe").ok_or(ToolError::NotFound("ffprobe"))?;
        let ffmpeg_version = version_line(&ffmpeg)?;
        // The line itself is unused; running it is what proves ffprobe is executable
        // before the first file is read.
        version_line(&ffprobe)?;
        let encoders = detect_encoders(&ffmpeg)?;

        Ok(Self {
            ffmpeg,
            ffprobe,
            ffmpeg_version,
            encoders,
        })
    }

    pub fn supports_video(&self, codec: VideoCodec) -> bool {
        self.encoders.contains(codec.encoder())
    }

    pub fn supports_audio(&self, codec: AudioCodec) -> bool {
        codec
            .encoder()
            .is_none_or(|encoder| self.encoders.contains(encoder))
    }

    #[cfg(test)]
    pub(crate) fn test_fixture() -> Self {
        Self {
            ffmpeg: PathBuf::from("/usr/local/bin/ffmpeg"),
            ffprobe: PathBuf::from("/usr/local/bin/ffprobe"),
            ffmpeg_version: "ffmpeg version 8.1.2".to_owned(),
            encoders: encoder_names().map(str::to_owned).collect(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error(
        "{0} was not found. Install FFmpeg with `brew install ffmpeg` or set an override environment variable."
    )]
    NotFound(&'static str),
    #[error("Failed to run {path}: {source}")]
    Invocation {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} exited with an error: {message}")]
    Failed { path: PathBuf, message: String },
}

fn resolve_tool(override_name: &str, binary: &str) -> Option<PathBuf> {
    if let Some(path) = env::var_os(override_name).map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    if let Some(paths) = env::var_os("PATH") {
        for directory in env::split_paths(&paths) {
            let candidate = directory.join(binary);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    ["/opt/homebrew/bin", "/usr/local/bin"]
        .into_iter()
        .map(|directory| Path::new(directory).join(binary))
        .find(|candidate| candidate.is_file())
}

fn version_line(path: &Path) -> Result<String, ToolError> {
    let output = run(path, [OsStr::new("-version")])?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("Unknown version")
        .to_owned())
}

fn detect_encoders(path: &Path) -> Result<HashSet<String>, ToolError> {
    let output = run(path, [OsStr::new("-hide_banner"), OsStr::new("-encoders")])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let reported: HashSet<&str> = text.split_whitespace().collect();
    Ok(encoder_names()
        .filter(|name| reported.contains(name))
        .map(str::to_owned)
        .collect())
}

fn run<I, S>(path: &Path, args: I) -> Result<std::process::Output, ToolError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output =
        Command::new(path)
            .args(args)
            .output()
            .map_err(|source| ToolError::Invocation {
                path: path.to_owned(),
                source,
            })?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(ToolError::Failed {
            path: path.to_owned(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_override_falls_back_instead_of_panicking() {
        assert!(resolve_tool("OTTER_TEST_MISSING", "definitely-not-a-real-binary").is_none());
    }
}
