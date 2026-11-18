use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::data_structures::Settings;

/// `~/.local/share/spectrocut/bin` (or the platform equivalent): where downloaded tools live.
pub fn local_bin_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "spectrocut").map(|d| d.data_dir().join("bin"))
}

/// Puts our local tool directory in front of PATH for this process and its children.
pub fn prepend_local_bin_to_path() {
    let Some(dir) = local_bin_dir() else { return };
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::env::split_paths(&current).collect();
    if paths.iter().any(|p| p == &dir) {
        return;
    }
    paths.insert(0, dir);
    if let Ok(joined) = std::env::join_paths(paths) {
        // SAFETY: called once at startup, before any other threads exist.
        unsafe { std::env::set_var("PATH", joined) };
    }
}

pub fn exe_name(tool: &str) -> String {
    if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.to_string()
    }
}

/// Searches PATH for an executable, like `which`.
pub fn find_in_path(tool: &str) -> Option<PathBuf> {
    let name = exe_name(tool);
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
}

/// First line of `<tool> -version`, shortened to the version token when possible.
pub fn tool_version(path: &Path) -> Option<String> {
    let out = Command::new(path)
        .arg("-version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?;
    // "ffmpeg version 7.1-static https://..." -> "7.1-static"
    Some(first.split_whitespace().nth(2).unwrap_or(first).to_string())
}

/// Formats a float without trailing noise (0.8500000001 -> "0.85").
pub fn fmt_float(v: f64) -> String {
    let rounded = (v * 100.0).round() / 100.0;
    format!("{rounded}")
}

pub fn build_filter_complex(s: &Settings) -> String {
    let (w, h) = (s.width, s.height);
    let bg = if s.fit == "fit" {
        format!(
            "[0:v]scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=gbrp[bg]"
        )
    } else {
        format!(
            "[0:v]scale={w}:{h}:force_original_aspect_ratio=increase,\
             crop={w}:{h},setsar=1,format=gbrp[bg]"
        )
    };
    let sp = format!(
        "[1:a]showspectrum=s={w}x{h}:orientation={}:mode={}:slide={}:color={}:scale={}:fscale={}:legend={}:fps={}{},setsar=1,format=gbrp[sp]",
        s.orientation,
        s.mode,
        s.slide,
        s.color,
        s.scale,
        s.fscale,
        u8::from(s.legend),
        s.fps,
        if s.vflip { ",vflip" } else { "" },
    );
    let out_scale = if s.bt709 {
        "scale=out_color_matrix=bt709:out_range=tv,"
    } else {
        ""
    };
    let blend = format!(
        "[bg][sp]blend=all_mode={}:all_opacity={},{out_scale}format=yuv420p[v]",
        s.blend_mode,
        fmt_float(s.opacity),
    );
    format!("{bg};\n{sp};\n{blend}")
}

/// ffmpeg arguments (without the program name) exactly as the user would type them.
pub fn build_args(s: &Settings) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|x| x.to_string()));

    let fps = s.fps.to_string();
    push(&[
        "-framerate",
        &fps,
        "-loop",
        "1",
        "-i",
        &s.image,
        "-i",
        &s.audio,
    ]);
    push(&["-filter_complex", &build_filter_complex(s)]);
    push(&["-map", "[v]", "-map", "1:a"]);
    push(&[
        "-c:v",
        "libx264",
        "-preset",
        &s.preset,
        "-crf",
        &s.crf.to_string(),
    ]);
    push(&[
        "-profile:v",
        &s.profile,
        "-bf",
        &s.bframes.to_string(),
        "-g",
        &s.gop.to_string(),
    ]);
    if s.threads > 0 {
        push(&["-threads", &s.threads.to_string()]);
    }
    if s.bt709 {
        push(&[
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
        ]);
    }
    push(&[
        "-c:a",
        &s.audio_codec,
        "-b:a",
        &format!("{}k", s.audio_bitrate),
        "-ar",
        &s.sample_rate,
    ]);
    if s.shortest {
        push(&["-shortest"]);
    }
    if s.faststart {
        push(&["-movflags", "+faststart"]);
    }
    push(&[&s.output]);
    a
}

/// The program to launch plus its arguments, wrapped in `nice` on Unix when requested.
pub fn full_command(s: &Settings) -> (String, Vec<String>) {
    let args = build_args(s);
    if cfg!(unix) && s.nice > 0 {
        let mut wrapped = vec!["-n".to_string(), s.nice.to_string(), "ffmpeg".to_string()];
        wrapped.extend(args);
        ("nice".into(), wrapped)
    } else {
        ("ffmpeg".into(), args)
    }
}

pub fn shell_quote(arg: &str) -> String {
    let safe = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:+=,@%".contains(c));
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// A copy-pasteable, multi-line shell rendering of the command.
pub fn pretty_command(s: &Settings) -> String {
    let (program, args) = full_command(s);
    let mut out = program.clone();
    let mut iter = args.iter().peekable();
    if program == "nice" {
        // Keep `nice -n 19 ffmpeg` on the first line.
        for a in iter.by_ref().take(3) {
            out.push(' ');
            out.push_str(a);
        }
    }
    // Line breaks mirror the hand-written layout: inputs / filter / maps / video / colour / audio / tail.
    let starts_line = |flag: &str, prev: &str| match flag {
        "-c:v" | "-colorspace" | "-c:a" => true,
        "-map" => prev != "-map",
        "-shortest" | "-movflags" => !matches!(prev, "-shortest" | "-movflags"),
        _ => false,
    };
    let mut prev = "";
    while let Some(arg) = iter.next() {
        let is_flag = arg.starts_with('-') && arg.len() > 1;
        if !is_flag {
            out.push(' ');
            out.push_str(&shell_quote(arg));
            continue;
        }
        if starts_line(arg, prev) {
            out.push_str(" \\\n");
        } else {
            out.push(' ');
        }
        out.push_str(arg);
        let next_is_value = iter
            .peek()
            .is_some_and(|n| !(n.starts_with('-') && n.len() > 1));
        let is_last_positional = iter.len() == 1;
        if next_is_value && arg != "-shortest" && !is_last_positional {
            let v = iter.next().expect("peeked");
            if arg == "-filter_complex" {
                out.push_str(" \\\n");
            } else {
                out.push(' ');
            }
            out.push_str(&shell_quote(v));
        }
        prev = arg;
    }
    out
}

/// One-line shell rendering, used for printing on exit.
pub fn one_line_command(s: &Settings) -> String {
    let (program, args) = full_command(s);
    std::iter::once(program)
        .chain(args.iter().map(|a| shell_quote(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Duration of a media file in seconds, via ffprobe.
pub fn probe_duration(path: &str) -> Option<f64> {
    if path.is_empty() || !Path::new(path).is_file() {
        return None;
    }
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

/// Parses one `key=value` line of `ffmpeg -progress` output.
pub fn parse_progress_line(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.trim().split_once('=')?;
    Some((k.trim(), v.trim()))
}

pub fn fmt_duration(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    format!(
        "{}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

pub fn fmt_bytes(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1_048_576.0)
}

/// "2026-11-18T03:12:00Z" -> "2026-11-18".
pub fn short_date(iso: &str) -> &str {
    iso.get(..10).unwrap_or(iso)
}

/// Path for display, without the Windows `\\?\` prefix that `canonicalize` adds.
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// A rectangle of the given percentage size, centred in `area`.
pub fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let [_, mid, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .areas(area);
    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .areas(mid);
    center
}

/// A centred box of fixed height (for one-line prompts).
pub fn centered_line_box(area: Rect, percent_x: u16, height: u16) -> Rect {
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .areas(area);
    centered_rect(mid, percent_x, 100)
}

/// A one-line text input popup.
pub fn draw_input(frame: &mut Frame, area: Rect, title: &str, buffer: &str, is_error: bool) {
    let rect = centered_line_box(area, 70, 3);
    frame.render_widget(Clear, rect);
    let border = if is_error { Color::Red } else { Color::Green };
    frame.render_widget(
        Paragraph::new(format!("{buffer}█")).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .title(title.to_string()),
        ),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGINAL_FILTER: &str = "[0:v]scale=1920:1080:force_original_aspect_ratio=increase,crop=1920:1080,setsar=1,format=gbrp[bg];
[1:a]showspectrum=s=1920x1080:orientation=horizontal:mode=combined:slide=scroll:color=green:scale=log:fscale=log:legend=0:fps=30,vflip,setsar=1,format=gbrp[sp];
[bg][sp]blend=all_mode=screen:all_opacity=0.85,scale=out_color_matrix=bt709:out_range=tv,format=yuv420p[v]";

    fn original_settings() -> Settings {
        Settings {
            image: "aber.jpg".into(),
            audio: "whyugodamenkovic2.wav".into(),
            output: "hallo37.mp4".into(),
            ..Settings::default()
        }
    }

    #[test]
    fn default_reproduces_original_filtergraph() {
        assert_eq!(build_filter_complex(&original_settings()), ORIGINAL_FILTER);
    }

    #[test]
    fn default_reproduces_original_args() {
        let expected: Vec<String> = [
            "-framerate",
            "30",
            "-loop",
            "1",
            "-i",
            "aber.jpg",
            "-i",
            "whyugodamenkovic2.wav",
            "-filter_complex",
            ORIGINAL_FILTER,
            "-map",
            "[v]",
            "-map",
            "1:a",
            "-c:v",
            "libx264",
            "-preset",
            "veryslow",
            "-crf",
            "16",
            "-profile:v",
            "high",
            "-bf",
            "2",
            "-g",
            "15",
            "-threads",
            "2",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-c:a",
            "aac",
            "-b:a",
            "384k",
            "-ar",
            "48000",
            "-shortest",
            "-movflags",
            "+faststart",
            "hallo37.mp4",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(build_args(&original_settings()), expected);
    }

    #[test]
    fn pretty_command_matches_hand_written_layout() {
        let text = pretty_command(&original_settings());
        println!("{text}");
        assert!(text.contains("\n-map '[v]' -map 1:a \\\n"));
        assert!(text.ends_with("\n-shortest -movflags +faststart hallo37.mp4"));
        assert!(text.contains("-c:a aac -b:a 384k -ar 48000 \\\n"));
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("aber.jpg"), "aber.jpg");
        assert_eq!(shell_quote("[v]"), "'[v]'");
        assert_eq!(shell_quote("my song.wav"), "'my song.wav'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn progress_lines() {
        assert_eq!(
            parse_progress_line("out_time_us=1234\n"),
            Some(("out_time_us", "1234"))
        );
        assert_eq!(
            parse_progress_line("progress=end"),
            Some(("progress", "end"))
        );
        assert_eq!(parse_progress_line("garbage"), None);
    }

    #[test]
    fn float_formatting() {
        assert_eq!(fmt_float(0.85), "0.85");
        assert_eq!(fmt_float(0.1 + 0.2), "0.3");
        assert_eq!(fmt_float(1.0), "1");
    }
}
