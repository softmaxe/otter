<p align="center">
  <img src="./docs/assets/otter-logo.png" alt="otter logo" width="180">
</p>

<h1 align="center">otter</h1>

<p align="center">
  <a href="README.md"><kbd>English</kbd></a>
  <a href="README.zh-CN.md"><kbd>简体中文</kbd></a>
</p>

Configure and run FFmpeg transcodes from a terminal interface on macOS.

<p align="center">
  <img src="./docs/assets/otter-demo.gif" alt="otter terminal interface demo" width="720">
</p>

otter lets you choose one or many inputs with a built-in file picker, select compatible containers and codecs, control resolution and bitrate, review the exact command, and monitor conversion progress from one terminal screen.

## Features

- One file or many: select a batch and convert it with one set of settings
- Per-file status while a batch runs, and a per-file summary when it ends
- Built-in terminal file picker for video files and destination folders, drawn by the app itself instead of a native dialog
- MP4, MOV, Matroska (MKV), and WebM output
- H.264, H.265, AV1, and VP9 video options where compatible
- VideoToolbox hardware encoding for H.264 and H.265 when the installed FFmpeg provides it
- AAC, Opus, MP3, or disabled audio where compatible
- Source, 2160p, 1440p, 1080p, 720p, and 480p resolution presets
- Video quality presets using codec-specific constant-quality values
- Target video bitrate presets and custom values
- Audio bitrate presets and custom values
- Estimated output size for the whole selection, accurate to a few percent in
  target-bitrate mode and marked `(rough)` under constant quality
- Shell-quoted command preview before execution
- Live processed time, percentage, speed, and recent FFmpeg messages
- Cancelling stops the running conversion, removes its temporary output, and
  leaves the rest of the batch unstarted
- Refusal to overwrite an existing output file

## Installation

### Homebrew

Homebrew installs the prebuilt binary and its FFmpeg dependency:

```sh
brew tap softmaxe/tap
brew install otter
```

Keep it current with `brew update && brew upgrade otter`. You can also install it without tapping first:

```sh
brew install softmaxe/tap/otter
```

Run `otter` directly in an interactive terminal.

### From source

Building from source requires macOS, Rust 1.85 or newer, FFmpeg, FFprobe, and an interactive terminal. Install the build and runtime tools with Homebrew:

```sh
brew install rust ffmpeg
```

```sh
cargo build --release
./target/release/otter
```

For development:

```sh
cargo run
```

The application must be run directly in an interactive terminal. Redirected stdin or stdout is not supported.

otter searches `PATH`, `/opt/homebrew/bin`, and `/usr/local/bin`. Custom binary locations can be selected with environment variables:

```sh
OTTER_FFMPEG=/custom/path/ffmpeg \
OTTER_FFPROBE=/custom/path/ffprobe \
cargo run --release
```

## Workflow

The interface is a five-step wizard: a bar of dots on top marks **Folders**, **Settings**, **Review**, **Progress**, and **Done**. One card shows the current step, with the way forward as a button on its bottom row.

1. On **Folders**, press `i` and choose one or more input video files. Otter probes every selected file with FFprobe.
2. Press `o` to choose the specific destination folder. Each input gets a derived output file inside that folder.
3. On **Settings**, move through the container, codecs, resolution, rate control, and bitrate details with `Tab`, `Shift-Tab`, arrow keys, or `h`/`j`/`k`/`l`.
4. Press `Enter` (or the `Review →` button) to see **Review**: the exact FFmpeg command plus the list of files the batch would write. `Start →` begins.
5. Watch **Progress** and recent FFmpeg messages. **Done** reports the outcome, one line per file when the batch ran as a queue.

Only one conversion runs at a time. A batch runs its files one after another, because FFmpeg already uses the whole machine for one encode.

Mouse control is available throughout. Move the pointer over a row, button, header chip, status chip, or picker listing entry to highlight it. Hover is separate from keyboard focus. Left-click a setting or a card button, use the wheel or right-click to move a setting's value forward or backward, and click the chips on the header or status bar to run the key they stand for. In the file picker, left-click highlights a row, double-click confirms it, and the wheel scrolls the listing without changing the selected row.

## File picker

The input and output pickers are drawn by the application itself instead of a native dialog, so they behave the same on every platform. The picker is a modal card: the folder being visited sits on top, the listing fills the middle, and the buttons sit on the bottom row, `Cancel (esc)`, `Parent (←)`, and the mode's action on the right.

| Key | Action |
| --- | --- |
| `Up` / `Down` or `k` / `j` | Move between rows |
| `Enter` | Open the highlighted folder, or choose a highlighted input file |
| `Space` | Add or remove the highlighted input file |
| `s` | Confirm the selected input files, or use the current output folder |
| `Tab` | Focus the file name field (legacy output-file mode) |
| `h` / `Left` / `Backspace` | Go to the parent folder |
| `g` | Go to the home folder |
| `.` | Toggle hidden files |
| `Esc` | Close the picker without choosing anything |

The input picker selects concrete files, supports multi-selection with `Space`, and passes every selected path to FFprobe. The picker does not guess whether a file is video from its extension. The output picker always selects a directory, including when only one input file is found.

## Batch conversion

Every selected file is converted with the same settings, in the order it was selected.

- Each output is named `<source name>-transcode.<extension>` inside the chosen folder.
- A file that ffprobe cannot read blocks the batch until it is removed from the selection, rather than being skipped silently.
- Two inputs that would produce the same output name are refused before anything runs.
- A file that fails to convert does not stop the ones behind it. The result screen lists every file with its outcome.
- Cancelling stops the running conversion and leaves the queued files untouched.

## Keyboard reference

### Folders

| Key | Action |
| --- | --- |
| `Tab` / `Down` | Move from input video files to output folder to Next |
| `Up` / `Shift-Tab` | Move backward through the input, output, and Next controls |
| `Enter` / `i` | Choose input video files |
| `a` | Add more input video files |
| `c` | Clear the selected input files |
| `r` | Replace the selected input files |
| `o` | Choose the output folder |
| `?` | Show keyboard help |
| `q` | Quit |

### Settings

| Key | Action |
| --- | --- |
| `Tab` / `Shift-Tab` | Move between settings |
| `Up` / `Down` or `k` / `j` | Move between settings |
| `Left` / `Right` or `h` / `l` | Change the selected value |
| `r` | Probe the selected inputs again |
| `Enter` | Edit a bitrate or open the review step |
| `?` | Show keyboard help |
| `q` | Quit |

### Review

| Key | Action |
| --- | --- |
| `Up` / `Down`, `PageUp` / `PageDown`, `Home` / `End` | Scroll the full command and file mapping list |
| `Enter` / `y` | Start conversion |
| `Esc` / `n` | Return to settings |
| `q` | Quit |

### Progress

| Key | Action |
| --- | --- |
| `Up` / `Down`, `PageUp` / `PageDown`, `Home` / `End` | Scroll FFmpeg messages; `End` resumes following the latest output |
| `x` / `Ctrl-C` | Open the cancellation confirmation |
| `q` / `Esc` | Open the cancellation confirmation |
| `y` / `Enter` | Confirm cancellation, stopping the rest of the batch |
| `n` / `Esc` | Keep running |

### Done

| Key | Action |
| --- | --- |
| `Enter` / `Esc` | Return to folders |
| `?` | Show keyboard help |
| `q` | Quit |

## Supported combinations

The interface prevents container and codec combinations that are known to be incompatible.

| Container | Video codecs | Audio codecs |
| --- | --- | --- |
| MP4 | H.264 (CPU/GPU), H.265 (CPU/GPU), AV1 | AAC, None |
| MOV | H.264 (CPU/GPU), H.265 (CPU/GPU), AV1 | AAC, None |
| Matroska (MKV) | H.264 (CPU/GPU), H.265 (CPU/GPU), AV1, VP9 | AAC, Opus, MP3, None |
| WebM | AV1, VP9 | Opus, None |

A codec is selectable only when its encoder is available in the detected FFmpeg build. The software encoders used are `libx264`, `libx265`, `libsvtav1`, `libvpx-vp9`, `aac`, `libopus`, and `libmp3lame`. The hardware encoders are `h264_videotoolbox` and `hevc_videotoolbox`.

## Hardware acceleration

Selecting a `VideoToolbox, GPU` codec moves encoding from the CPU to the Apple media engine and adds `-hwaccel videotoolbox` so decoding runs there too. FFmpeg falls back to software decoding for formats the media engine cannot read.

Measured on an M2 Pro with FFmpeg 8.1.2, using a 30-second 1080p30 clip and quality-matched presets:

| Encoder | Wall clock | CPU time | Output size | VMAF |
| --- | ---: | ---: | ---: | ---: |
| `libx264` medium, CRF 23 | 5.06 s | 52.7 s | 25.8 MB | 96.43 |
| `h264_videotoolbox`, q 60 | 4.50 s | 4.8 s | 27.3 MB | 96.75 |
| `libx265` medium, CRF 26 | 13.29 s | 104.8 s | 20.2 MB | 94.88 |
| `hevc_videotoolbox`, q 55 | 4.83 s | 4.8 s | 22.8 MB | 95.27 |

H.265 is where hardware wins outright: roughly 2.7x faster with about 22x less CPU time. H.264 finishes in a similar wall-clock time to a fast multi-core `libx264` run but uses about a tenth of the CPU, which keeps the machine responsive and reduces power draw. Software encoders still compress better at low bitrates, so keep `libx264` or `libx265` when file size matters more than time.

VideoToolbox encoders are hardware-only. A job fails immediately rather than falling back to a slow software VideoToolbox path.

## Rate control

### Constant quality

Quality mode exposes High, Balanced, and Small file presets. Software encoders take a CRF, where a lower value means higher quality and a larger file. VideoToolbox has no CRF and instead uses an inverted `-q:v` scale, where a higher value means higher quality. The hardware values were calibrated against the software presets with VMAF so both paths land on comparable quality.

| Codec | Flag | High | Balanced | Small file |
| --- | --- | ---: | ---: | ---: |
| H.264 (libx264) | `-crf` | 18 | 23 | 28 |
| H.264 (VideoToolbox) | `-q:v` | 70 | 60 | 47 |
| H.265 (libx265) | `-crf` | 20 | 26 | 30 |
| H.265 (VideoToolbox) | `-q:v` | 65 | 55 | 47 |
| AV1 | `-crf` | 28 | 35 | 42 |
| VP9 | `-crf` | 24 | 32 | 40 |

Quality mode does not set a target video bitrate, except for VP9's required `-b:v 0` setting.

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

otter never executes the preview through a shell. Program arguments remain separate values, so spaces, quotes, Unicode, and shell metacharacters in paths are not interpreted as shell syntax.

The application also:

- rejects an output path that matches any selected input;
- refuses to overwrite an existing final output;
- writes into an app-owned `.otter-*` directory beside the destination; and
- atomically renames the completed temporary file to the final path only after FFmpeg succeeds.

Failed and cancelled jobs remove the app-owned temporary directory. Cancellation sends FFmpeg `SIGINT` first and force-stops it only if it does not exit within approximately three seconds.

## Limitations

Conversions run one at a time, from local files. The application does not include:

- parallel encoding or a queue that survives restarting the application;
- per-file settings within one batch;
- stream copy;
- subtitle, attachment, or data-stream copying;
- custom FFmpeg arguments;
- hardware-encoder tuning beyond the quality presets;
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

The integration tests generate temporary synthetic media with FFmpeg. They skip when FFmpeg, FFprobe, `libx264`, or AAC is unavailable and do not access user media files. The VideoToolbox test additionally skips when the hardware encoders are missing.

## Publish a release

Run the **Release** workflow manually to test both macOS packages without publishing a GitHub Release. To publish, first set `package.version` in `Cargo.toml`, then create and push the matching three-part version tag:

```sh
git tag v1.0.0
git push origin v1.0.0
```

The workflow tests and packages native Apple silicon and Intel binaries, publishes both archives with `SHA256SUMS`, then updates `Formula/otter.rb` in `softmaxe/homebrew-tap`. The `bump-tap` job requires an Actions secret named `OTTER_HOMEBREW_TAP_UPDATER` with write access to that repository.
