<p align="center">
  <img src="./docs/assets/otter-logo.png" alt="otter logo" width="180">
</p>

<h1 align="center">otter</h1>

<p align="center">
  <a href="README.md"><kbd>English</kbd></a>
  <a href="README.zh-CN.md"><kbd>简体中文</kbd></a>
</p>

A macOS terminal interface for configuring and running FFmpeg transcodes.

<p align="center">
  <img src="./docs/assets/otter-demo.gif" alt="otter terminal interface demo" width="720">
</p>

Select input files, set the output format and quality, review the exact FFmpeg command, and follow each conversion from one screen. Single files and batches use the same workflow.

## What it does

- Converts one or many local videos with the same settings
- Outputs MP4, MOV, MKV, or WebM with compatible H.264, H.265, AV1, or VP9 video
- Uses CPU encoders or VideoToolbox hardware encoding for H.264 and H.265
- Offers resolution, constant-quality, video bitrate, and audio bitrate presets
- Shows the exact command, estimated total output size, progress, speed, and FFmpeg messages
- Refuses to overwrite files and cleans up temporary output after failure or cancellation

## Install

Homebrew installs otter and FFmpeg:

```sh
brew install softmaxe/tap/otter
otter
```

The Homebrew package currently supports Apple silicon Macs. otter has no separate command-line options; it opens the terminal interface when run in an interactive terminal.

Update later with:

```sh
brew update && brew upgrade otter
```

otter requires macOS and an interactive terminal. Redirected stdin or stdout is not supported.

### Build from source

Source builds require Rust 1.85 or newer, FFmpeg, and FFprobe:

```sh
brew install rust ffmpeg
cargo build --release
./target/release/otter
```

For development, run `cargo run`. To use FFmpeg binaries outside the standard search paths:

```sh
OTTER_FFMPEG=/custom/path/ffmpeg \
OTTER_FFPROBE=/custom/path/ffprobe \
cargo run --release
```

otter searches `PATH`, `/opt/homebrew/bin`, and `/usr/local/bin` by default.

## Quick start

1. Press `i` and choose one or more input files. Use `Space` for multi-selection and `s` to confirm.
2. Press `o` and choose an output folder.
3. Set the container, codecs, resolution, and rate control.
4. Choose Review to inspect the exact FFmpeg command and output paths, then choose Start.
5. Follow each file on the Progress screen. The Done screen lists the result of every file.

Use `Tab` or arrow keys to move, `Enter` to select, `Esc` to go back, and `q` to quit. `h`/`j`/`k`/`l` also work. Press `?` for the in-app help. To cancel a running queue, press `x` and confirm. The interface supports clicking, scrolling, and double-clicking in the file picker.

## Formats

otter only shows codecs available in the installed FFmpeg build and allowed by the selected container.

| Container | Video | Audio |
| --- | --- | --- |
| MP4 | H.264, H.265, AV1 | AAC, none |
| MOV | H.264, H.265, AV1 | AAC, none |
| MKV | H.264, H.265, AV1, VP9 | AAC, Opus, MP3, none |
| WebM | AV1, VP9 | Opus, none |

Available software video encoders are `libx264`, `libx265`, `libsvtav1`, and `libvpx-vp9`. H.264 and H.265 can also use `h264_videotoolbox` and `hevc_videotoolbox` when FFmpeg provides them.

Resolution presets are Source, 2160p, 1440p, 1080p, 720p, and 480p. Scaling preserves aspect ratio, never enlarges a smaller source, and keeps dimensions divisible by two.

## Batch and output behavior

- Files run one at a time, in selection order. A failed file does not stop the rest of the batch.
- All files in a batch use the same settings.
- Outputs are named `<source name>-transcode.<extension>` in the selected folder.
- Unreadable inputs and duplicate output names block the batch before conversion starts.
- Cancelling stops the current conversion and leaves queued files unstarted.
- Existing output files are never overwritten.

otter passes arguments directly to FFmpeg instead of executing the preview through a shell. It writes each conversion to a private `.otter-*` directory beside the destination, then atomically moves the completed file into place. Failed and cancelled jobs remove their temporary directory.

## Rate control

Quality mode provides High, Balanced, and Small file presets. Software encoders use CRF, where lower values mean higher quality. VideoToolbox uses `-q:v`, where higher values mean higher quality.

| Codec | Flag | High | Balanced | Small file |
| --- | --- | ---: | ---: | ---: |
| H.264 (`libx264`) | `-crf` | 18 | 23 | 28 |
| H.264 (VideoToolbox) | `-q:v` | 70 | 60 | 47 |
| H.265 (`libx265`) | `-crf` | 20 | 26 | 30 |
| H.265 (VideoToolbox) | `-q:v` | 65 | 55 | 47 |
| AV1 | `-crf` | 28 | 35 | 42 |
| VP9 | `-crf` | 24 | 32 | 40 |

Target-bitrate mode accepts 100 to 200000 kbps for video and 32 to 512 kbps for audio. Audio bitrate is disabled when the input has no audio or the audio codec is set to none.

VideoToolbox usually finishes H.265 much faster and uses far less CPU than software encoding. Software encoders remain useful when compression efficiency matters more than encoding time. VideoToolbox jobs fail if the hardware encoder is unavailable; they do not fall back to a software encoder.

## Limitations

- Local files only
- No parallel encoding or persistent queue
- No per-file settings within a batch
- No stream copy, subtitle, attachment, or data-stream copying
- No custom FFmpeg arguments or hardware-encoder tuning beyond the quality presets
- No saved presets or automatic replacement of existing outputs
- Uses the first video stream and, when present, the first audio stream

## Development

Run the full local check with:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

Integration tests create temporary synthetic media with FFmpeg. Tests that need unavailable encoders are skipped.

To publish a release, update `package.version` in `Cargo.toml`, then create and push the matching `vX.Y.Z` tag. The Release workflow builds an Apple silicon (`aarch64-apple-darwin`) archive, publishes checksums, and updates `softmaxe/homebrew-tap` when its updater token is configured. Release binaries are unsigned, so macOS Gatekeeper may warn the first time you run one.

## License

[GNU Affero General Public License v3](LICENSE), `AGPL-3.0-or-later`.
