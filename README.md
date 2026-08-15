# FFmpeg TUI

A macOS-first terminal interface for configuring and running single-file FFmpeg transcodes without writing FFmpeg commands by hand.

FFmpeg TUI lets you choose an input and output with native file dialogs, select compatible containers and codecs, control resolution and bitrate, review the exact command, and monitor conversion progress from one terminal screen.

## Features

- Native macOS Open and Save dialogs
- MP4, MOV, Matroska (MKV), and WebM output
- H.264, H.265, AV1, and VP9 video options where compatible
- AAC, Opus, MP3, or disabled audio where compatible
- Source, 2160p, 1440p, 1080p, 720p, and 480p resolution presets
- Video quality presets using codec-specific CRF values
- Target video bitrate presets and custom values
- Audio bitrate presets and custom values
- Shell-safe command preview before execution
- Live processed time, percentage, speed, and recent FFmpeg messages
- Graceful cancellation with automatic temporary-output cleanup
- Refusal to overwrite an existing output file

## Requirements

- macOS
- Rust 1.85 or newer
- FFmpeg and FFprobe
- An interactive terminal

Install the required tools with Homebrew:

```sh
brew install rust ffmpeg
```

FFmpeg TUI searches `PATH`, `/opt/homebrew/bin`, and `/usr/local/bin`. Custom binary locations can be selected with environment variables:

```sh
FFMPEG_TUI_FFMPEG=/custom/path/ffmpeg \
FFMPEG_TUI_FFPROBE=/custom/path/ffprobe \
cargo run --release
```

## Build and run

```sh
cargo build --release
./target/release/ffmpeg-tui
```

For development:

```sh
cargo run
```

The application must be run directly in an interactive terminal. Redirected stdin or stdout is not supported.

## Workflow

1. Press `i` and choose an input media file.
2. Review the detected video, audio, duration, and dimensions.
3. Press `o` to choose the final output path.
4. Move through settings with `Tab`, `Shift-Tab`, arrow keys, or `h`/`j`/`k`/`l`.
5. Select a container, compatible codecs, resolution, and rate-control mode.
6. Press `Enter` to review the exact FFmpeg command.
7. Press `Enter` or `y` to start the conversion.
8. Watch progress and recent FFmpeg messages. Press `x` to open the cancellation confirmation.

Only one conversion runs at a time.

## Keyboard reference

### Configure

| Key | Action |
| --- | --- |
| `Tab` / `Shift-Tab` | Move between settings |
| `Up` / `Down` or `k` / `j` | Move between settings |
| `Left` / `Right` or `h` / `l` | Change the selected value |
| `i` | Choose the input file |
| `o` | Choose the output file |
| `r` | Probe the selected input again |
| `Enter` | Edit a bitrate or review the command |
| `?` | Show keyboard help |
| `q` | Quit |

### Confirm

| Key | Action |
| --- | --- |
| `Enter` / `y` | Start conversion |
| `Esc` / `n` | Return to settings |
| `q` | Quit |

### Running

| Key | Action |
| --- | --- |
| `x` / `Ctrl-C` | Open the cancellation confirmation |
| `q` / `Esc` | Open the cancellation confirmation |
| `y` / `Enter` | Confirm cancellation |
| `n` / `Esc` | Keep running |

### Result or error

| Key | Action |
| --- | --- |
| `Enter` / `Esc` | Return to settings |
| `?` | Show keyboard help |
| `q` | Quit |

## Supported combinations

The interface prevents container and codec combinations that are known to be incompatible.

| Container | Video codecs | Audio codecs |
| --- | --- | --- |
| MP4 | H.264, H.265, AV1 | AAC, None |
| MOV | H.264, H.265, AV1 | AAC, None |
| Matroska (MKV) | H.264, H.265, AV1, VP9 | AAC, Opus, MP3, None |
| WebM | AV1, VP9 | Opus, None |

A codec is selectable only when its encoder is available in the detected FFmpeg build. The software encoders used are `libx264`, `libx265`, `libsvtav1`, `libvpx-vp9`, `aac`, `libopus`, and `libmp3lame`.

## Rate control

### Quality (CRF)

Quality mode exposes High, Balanced, and Small file presets. Each video codec uses CRF values appropriate for that encoder:

| Codec | High | Balanced | Small file |
| --- | ---: | ---: | ---: |
| H.264 | 18 | 23 | 28 |
| H.265 | 20 | 26 | 30 |
| AV1 | 28 | 35 | 42 |
| VP9 | 24 | 32 | 40 |

A lower CRF generally produces higher quality and a larger file. CRF mode does not set a target video bitrate, except for VP9's required `-b:v 0` setting.

### Target bitrate

Video bitrate presets are 1000, 2500, 5000, 8000, 12000, and 20000 kbps. Custom video bitrates must be between 100 and 200000 kbps.

Audio bitrate presets are 64, 96, 128, 160, 192, 256, and 320 kbps. Custom audio bitrates must be between 32 and 512 kbps. Audio bitrate is disabled when the input has no audio stream or Audio codec is set to None.

## Resolution behavior

Resolution presets define a maximum output canvas. Scaling:

- preserves the source aspect ratio;
- does not enlarge a source smaller than the selected preset; and
- produces dimensions divisible by two for broad codec compatibility.

Select Source to keep the original dimensions.

## Output safety

FFmpeg TUI never executes the preview through a shell. Program arguments remain separate values, so spaces, quotes, Unicode, and shell metacharacters in paths are not interpreted as shell syntax.

The application also:

- rejects an output path that matches the input;
- refuses to overwrite an existing final output;
- writes into an app-owned `.ffmpeg-tui-*` directory beside the destination; and
- atomically renames the completed temporary file to the final path only after FFmpeg succeeds.

Failed and cancelled jobs remove the app-owned temporary directory. Cancellation sends FFmpeg `SIGINT` first and force-stops it only if it does not exit within approximately three seconds.

## MVP limitations

The first release intentionally supports one local input file and one conversion at a time. It does not include:

- batch processing or a persistent queue;
- stream copy;
- subtitle, attachment, or data-stream copying;
- custom FFmpeg arguments;
- hardware-encoder controls;
- saved user presets; or
- automatic replacement of existing outputs.

The first video stream and optional first audio stream are used.

## Development and validation

Run formatting, linting, unit tests, FFmpeg integration tests, and a release build:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

The integration tests generate temporary synthetic media with FFmpeg. They skip when FFmpeg, FFprobe, `libx264`, or AAC is unavailable and do not access user media files.

## File dialogs

The Open and Save panels run in a short-lived helper process, re-executing this binary with a hidden `--file-dialog` argument. AppKit is never initialised inside the long-running TUI process.

This is deliberate. An in-process panel leaves an invisible window behind at `CGShieldingWindowLevel`: its fade-out animation only completes while the process keeps pumping the AppKit run loop, which a TUI stops doing as soon as it returns to its own event loop. macOS then shows the spinning wait cursor over that region for the rest of the session. Ending the helper process removes the window with it.
