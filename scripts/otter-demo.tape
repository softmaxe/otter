Require cargo
Require ffmpeg
Require ffprobe

Output docs/assets/otter-demo.gif

Set Shell zsh
Set FontSize 16
Set Width 1000
Set Height 592
Set Padding 12
Set Framerate 24
Set TypingSpeed 80ms
Set Theme "Catppuccin Mocha"
Set CursorBlink false

# Build the app and generate a short real input clip outside the recording.
# Use /tmp/otter-demo instead of $PWD/.demo-recording to avoid leaking the
# absolute user path (e.g. /Users/<name>/...) into the recorded frames.
Hide
Type@0ms "cargo build --release && mkdir -p /tmp/otter-demo/output && rm -f /tmp/otter-demo/input.mp4 /tmp/otter-demo/output/input-transcode.mp4 && ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=1920x1080:rate=30 -f lavfi -i sine=frequency=1000:sample_rate=48000 -t 30 -c:v libx264 -preset ultrafast -crf 28 -pix_fmt yuv420p -c:a aac -shortest /tmp/otter-demo/input.mp4"
Enter
Wait
Type@0ms `HOME="/tmp/otter-demo" ./target/release/otter`
Enter
Sleep 1s
Show

# Folders: pick the generated input, then select the output directory.
Sleep 1s
Enter
Sleep 800ms
Down@180ms 2
Space
Sleep 600ms
Type "s"
Sleep 1s
Type "o"
Sleep 800ms
Down
Enter
Sleep 500ms
Type "s"
Sleep 1s
Tab@250ms 2
Enter

# Settings: choose 720p, target bitrate, 8000 kbps video, and 160 kbps audio.
Sleep 1s
Tab@220ms 3
Right@160ms 4
Sleep 700ms
Tab
Right
Sleep 700ms
Tab
Right
Sleep 700ms
Tab
Left
Sleep 700ms
Tab@250ms 2
Enter

# Review, run, progress, and completion.
Sleep 2s
Enter
Wait+Screen /Conversion completed successfully/
Sleep 2s

# Exit and remove only the generated demo fixtures outside the recording.
Hide
Type "q"
Sleep 500ms
Type@0ms "rm -f /tmp/otter-demo/input.mp4 /tmp/otter-demo/output/input-transcode.mp4 && rmdir /tmp/otter-demo/output /tmp/otter-demo 2>/dev/null; true"
Enter
Wait
