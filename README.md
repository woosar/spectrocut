# spectrocut

A terminal UI that turns **one still image + one audio track** into a music video: the image is
scaled and cropped to fill the frame, and a scrolling audio spectrum (`showspectrum`) is blended
over it. The output is an H.264 + AAC MP4.

It is a front-end for this hand-written ffmpeg command. Every option in the command can be edited,
and the defaults reproduce it exactly:

```sh
nice -n 19 ffmpeg -framerate 30 -loop 1 -i aber.jpg -i song.wav -filter_complex \
"[0:v]scale=1920:1080:force_original_aspect_ratio=increase,crop=1920:1080,setsar=1,format=gbrp[bg];
[1:a]showspectrum=s=1920x1080:orientation=horizontal:mode=combined:slide=scroll:color=green:scale=log:fscale=log:legend=0:fps=30,vflip,setsar=1,format=gbrp[sp];
[bg][sp]blend=all_mode=screen:all_opacity=0.85,scale=out_color_matrix=bt709:out_range=tv,format=yuv420p[v]" \
-map "[v]" -map 1:a \
-c:v libx264 -preset veryslow -crf 16 -profile:v high -bf 2 -g 15 -threads 2 \
-colorspace bt709 -color_primaries bt709 -color_trc bt709 \
-c:a aac -b:a 384k -ar 48000 \
-shortest -movflags +faststart out.mp4
```

## Install on a Chromebook

1. **Settings → About ChromeOS → Developers → Linux development environment → Turn on.**
   This gives you a Debian terminal ("Terminal" app → "penguin"). Intel/AMD and ARM Chromebooks
   both work.
2. Install Rust and a C linker (once):
   ```sh
   sudo apt update && sudo apt install -y build-essential curl
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source ~/.cargo/env
   ```
3. Copy this folder over (drag it into "Linux files" in the Files app), then:
   ```sh
   cd spectrocut
   cargo install --path .
   ```
4. Run `spectrocut` from the folder that holds your image and audio.

**You don't need to install ffmpeg yourself.** If it is missing, spectrocut opens on the **Tools**
screen, where you can:

| key | action |
|-----|--------|
| `d` | download or update a static ffmpeg + ffprobe build (BtbN/FFmpeg-Builds, GPL, includes libx264) into `~/.local/share/spectrocut/bin`, with no sudo needed |
| `p` | install with the system package manager instead (`sudo apt-get install ffmpeg` on ChromeOS) |
| `a` | add the private tool folder to your shell `PATH` (`~/.bashrc`, `~/.profile`) so plain `ffmpeg` works in the terminal too |
| `u` | check GitHub for a newer build |
| `r` | rescan |

spectrocut always puts its private tool folder first on its own PATH. A downloaded build therefore
works right away, even before you press `a`.

## Using it

| key | where | action |
|-----|-------|--------|
| `↑ ↓` / `j k` | settings | move between options |
| `← →` | settings | change choices, nudge numbers, flip toggles |
| `Enter` | settings | file browser for Image/Audio, type a value for text and numbers |
| `e` | settings | type a value directly |
| `r` | settings | render (asks before overwriting the output) |
| `s` / `l` | settings | save / load a preset (`~/.config/spectrocut/presets/*.toml`) |
| `D` | settings | reset every option to its default (keeps the files) |
| `t` | anywhere | tools screen |
| `c` | settings | quit and print the full command, ready to paste into a script |
| `q` / `Esc` | settings | quit (the last settings are remembered) |
| `Esc` | rendering | cancel (asks first) |

The right pane always shows the exact command that will run. The render screen shows progress,
speed, ETA and the ffmpeg log.

Tip for Chromebooks: `veryslow` + `nice 19` + `threads 2` keeps the machine usable but takes a
while. Try preset `medium` for quick previews.

## Development

```sh
cargo test                      # unit tests, incl. "defaults == the original command"
cargo test -- --ignored         # network + real-render tests (downloads ~150 MB)
```

## TODO / ideas

- **Grow into a general ffmpeg TUI.** Possible features: arbitrary inputs, a filter-graph editor,
  trimming, concatenation, and format conversion presets. The current design (`Settings` →
  `Field` table → `build_args`) generalises well if each "recipe" becomes its own
  `Settings`/field list.
- More visualizers as the overlay source (`showwaves`, `showcqt`, `avectorscope`).
- Preview a single frame (`-frames:v 1`) before a full render.
