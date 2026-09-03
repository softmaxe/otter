<p align="center">
  <img src="./docs/assets/otter-logo.png" alt="otter logo" width="180">
</p>

<h1 align="center">otter</h1>

<p align="center">
  <a href="README.md"><kbd>English</kbd></a>
  <a href="README.zh-CN.md"><kbd>简体中文</kbd></a>
</p>

在 macOS 终端中配置并运行 FFmpeg 转码。

<p align="center">
  <img src="./docs/assets/otter-demo.gif" alt="otter 终端界面演示" width="720">
</p>

在同一个界面中选择输入文件、设置输出格式和质量、检查实际 FFmpeg 命令并跟踪每个任务。单文件和批量任务使用相同流程。

## 能做什么

- 用同一套设置转换一个或多个本地视频
- 输出 MP4、MOV、MKV 或 WebM，并按容器匹配 H.264、H.265、AV1 或 VP9
- 使用 CPU 编码，或通过 VideoToolbox 硬件编码 H.264 和 H.265
- 提供分辨率、恒定质量、视频比特率和音频比特率预设
- 显示实际命令、预计总大小、进度、速度和 FFmpeg 消息
- 拒绝覆盖已有文件，失败或取消后自动清理临时输出

## 安装

Homebrew 会安装 otter 和 FFmpeg：

```sh
brew install softmaxe/tap/otter
otter
```

Homebrew 目前只提供 Apple silicon Mac 版本。otter 没有单独的命令行选项，在交互式终端中运行后会打开终端界面。

后续更新：

```sh
brew update && brew upgrade otter
```

otter 需要 macOS 和交互式终端，不支持重定向 stdin 或 stdout。

### 从源码构建

源码构建需要 Rust 1.85 或更新版本、FFmpeg 和 FFprobe：

```sh
brew install rust ffmpeg
cargo build --release
./target/release/otter
```

开发时运行 `cargo run`。如果 FFmpeg 不在默认搜索路径中，可以显式指定：

```sh
OTTER_FFMPEG=/custom/path/ffmpeg \
OTTER_FFPROBE=/custom/path/ffprobe \
cargo run --release
```

otter 默认搜索 `PATH`、`/opt/homebrew/bin` 和 `/usr/local/bin`。

## 快速上手

1. 按 `i` 选择一个或多个输入文件。按 `Space` 多选，按 `s` 确认。
2. 按 `o` 选择输出文件夹。
3. 设置容器、编解码器、分辨率和码率控制。
4. 选择 Review，检查实际 FFmpeg 命令和输出路径，然后选择 Start。
5. 在 Progress 页面查看每个文件的进度。Done 页面会列出所有文件的结果。

使用 `Tab` 或方向键移动，`Enter` 选择，`Esc` 返回，`q` 退出。也可以使用 `h`/`j`/`k`/`l`。按 `?` 打开界面内帮助；如需取消运行中的队列，按 `x` 后确认。界面支持鼠标点击和滚动，文件选择器支持双击。

## 支持格式

otter 只显示当前 FFmpeg 提供、且与所选容器兼容的编解码器。

| 容器 | 视频 | 音频 |
| --- | --- | --- |
| MP4 | H.264、H.265、AV1 | AAC、无音频 |
| MOV | H.264、H.265、AV1 | AAC、无音频 |
| MKV | H.264、H.265、AV1、VP9 | AAC、Opus、MP3、无音频 |
| WebM | AV1、VP9 | Opus、无音频 |

软件视频 encoder 包括 `libx264`、`libx265`、`libsvtav1` 和 `libvpx-vp9`。如果 FFmpeg 提供 `h264_videotoolbox` 和 `hevc_videotoolbox`，H.264 和 H.265 也可使用硬件编码。

分辨率预设包括 Source、2160p、1440p、1080p、720p 和 480p。缩放会保持宽高比，不会放大小于目标尺寸的源文件，并确保尺寸能被二整除。

## 批量与输出规则

- 文件按选择顺序逐个处理。单个文件失败不会中断后续任务。
- 同一批次中的所有文件共用一套设置。
- 输出文件命名为 `<source name>-transcode.<extension>`，保存在所选文件夹中。
- 无法读取的输入和重复的输出名称会在转码前阻止任务开始。
- 取消会停止当前转码，尚未开始的文件不会运行。
- otter 不会覆盖已有输出文件。

otter 直接把参数传给 FFmpeg，不会通过 shell 执行预览命令。每次转码会先写入目标文件旁的私有 `.otter-*` 目录，成功后再原子移动到最终路径。失败或取消时，临时目录会被删除。

## 码率控制

质量模式提供 High、Balanced 和 Small file 三档预设。软件 encoder 使用 CRF，数值越低质量越高。VideoToolbox 使用 `-q:v`，数值越高质量越高。

| 编解码器 | 参数 | High | Balanced | Small file |
| --- | --- | ---: | ---: | ---: |
| H.264 (`libx264`) | `-crf` | 18 | 23 | 28 |
| H.264 (VideoToolbox) | `-q:v` | 70 | 60 | 47 |
| H.265 (`libx265`) | `-crf` | 20 | 26 | 30 |
| H.265 (VideoToolbox) | `-q:v` | 65 | 55 | 47 |
| AV1 | `-crf` | 28 | 35 | 42 |
| VP9 | `-crf` | 24 | 32 | 40 |

目标比特率模式接受 100 到 200000 kbps 的视频比特率，以及 32 到 512 kbps 的音频比特率。输入没有音频或音频设为无时，音频比特率不可用。

VideoToolbox 通常能更快完成 H.265 转码，并显著减少 CPU 占用。如果压缩效率比转码时间更重要，可以使用软件 encoder。硬件 encoder 不可用时，VideoToolbox 任务会失败，不会自动切换到软件 encoder。

## 限制

- 只支持本地文件
- 不支持并行编码或重启后保留队列
- 同一批次不能逐文件设置参数
- 不支持 stream copy，也不复制字幕、附件或数据流
- 不支持自定义 FFmpeg 参数，也不能在质量预设之外调整硬件 encoder
- 不支持保存预设或自动替换已有输出
- 使用第一个视频流，以及存在时的第一个音频流

## 开发

运行完整的本地检查：

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo build --release
```

集成测试使用 FFmpeg 创建临时合成媒体。依赖未安装 encoder 的测试会跳过。

发布版本时，先更新 `Cargo.toml` 中的 `package.version`，再创建并推送对应的 `vX.Y.Z` tag。Release workflow 会构建 Apple silicon（`aarch64-apple-darwin`）压缩包并发布校验和；配置更新凭据后，它还会更新 `softmaxe/homebrew-tap`。发布的二进制未签名，首次运行时 macOS Gatekeeper 可能会发出警告。

## 许可证

[GNU Affero General Public License v3](LICENSE)，`AGPL-3.0-or-later`。
