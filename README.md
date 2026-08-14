# FFmpeg TUI

A macOS-first terminal interface for configuring and running single-file FFmpeg transcodes without composing commands by hand.

## Status

The project is being initialized. The first release will support output container, codecs, resolution, quality or target bitrate, command preview, live progress, and cancellation.

## Requirements

- macOS
- Rust 1.85 or newer
- FFmpeg and FFprobe available through `PATH` or Homebrew

## Development

```sh
cargo run
```

Batch processing is not part of the first release.
