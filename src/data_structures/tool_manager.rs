use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use anyhow::{Context, Result, bail};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Wrap},
};

use crate::helpers::{
    exe_name, find_in_path, fmt_bytes, local_bin_dir, shell_quote, short_date, tool_version,
};

const RELEASE_API: &str = "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/latest";
const PROFILE_MARKER: &str = "# added by spectrocut";

struct ToolInfo {
    name: &'static str,
    required: bool,
    path: Option<PathBuf>,
    version: Option<String>,
}

/// A downloadable static build for this machine.
#[derive(Clone)]
struct Asset {
    name: String,
    url: String,
    size: u64,
    published: String,
}

enum InstallEvent {
    Status(String),
    Progress(u64, u64),
    Done(Result<String, String>),
}

/// Package-manager command for this system, run interactively (it may ask for a password).
pub struct PackageCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl PackageCommand {
    pub fn display(&self) -> String {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().map(|a| shell_quote(a)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// The "Tools" screen: finds ffmpeg/ffprobe, downloads or updates static builds
/// into a private folder, and puts that folder on PATH.
pub struct ToolManager {
    tools: Vec<ToolInfo>,
    has_x264: Option<bool>,
    latest: Option<Result<Asset, String>>,
    latest_rx: Option<Receiver<Result<Asset, String>>>,
    install_rx: Option<Receiver<InstallEvent>>,
    install_status: String,
    install_progress: Option<(u64, u64)>,
    installed_stamp: Option<String>,
    pub message: String,
}

impl ToolManager {
    pub fn new() -> Self {
        let mut tm = Self {
            tools: Vec::new(),
            has_x264: None,
            latest: None,
            latest_rx: None,
            install_rx: None,
            install_status: String::new(),
            install_progress: None,
            installed_stamp: None,
            message: String::new(),
        };
        tm.refresh();
        tm
    }

    /// Re-scan PATH for the tools.
    pub fn refresh(&mut self) {
        let mut names = vec![("ffmpeg", true), ("ffprobe", true)];
        if cfg!(unix) {
            names.push(("nice", false));
        }
        self.tools = names
            .into_iter()
            .map(|(name, required)| {
                let path = find_in_path(name);
                let version = match (&path, name) {
                    (Some(p), "ffmpeg" | "ffprobe") => tool_version(p),
                    (Some(_), _) => Some("ok".into()),
                    _ => None,
                };
                ToolInfo {
                    name,
                    required,
                    path,
                    version,
                }
            })
            .collect();
        self.has_x264 = find_in_path("ffmpeg").map(|p| {
            Command::new(p)
                .args(["-hide_banner", "-encoders"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("libx264"))
                .unwrap_or(false)
        });
        self.installed_stamp = local_bin_dir()
            .and_then(|d| std::fs::read_to_string(d.join("installed.txt")).ok())
            .map(|s| s.trim().to_string());
    }

    pub fn all_ok(&self) -> bool {
        self.tools.iter().all(|t| !t.required || t.path.is_some()) && self.has_x264 == Some(true)
    }

    pub fn is_busy(&self) -> bool {
        self.install_rx.is_some()
    }

    /// Ask GitHub (in the background) which build is newest.
    pub fn check_latest(&mut self) {
        if self.latest_rx.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(ToolManager::find_asset().map_err(|e| format!("{e:#}")));
        });
        self.latest_rx = Some(rx);
    }

    pub fn start_download(&mut self) {
        if self.is_busy() {
            return;
        }
        let Some(bin_dir) = local_bin_dir() else {
            self.message = "No data directory available on this system.".into();
            return;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = ToolManager::install(&bin_dir, &tx).map_err(|e| format!("{e:#}"));
            let _ = tx.send(InstallEvent::Done(result));
        });
        self.install_rx = Some(rx);
        self.install_status = "Starting…".into();
        self.install_progress = None;
        self.message.clear();
    }

    /// Process background results. Call every tick.
    pub fn poll(&mut self) {
        if let Some(rx) = &self.latest_rx
            && let Ok(result) = rx.try_recv()
        {
            self.latest = Some(result);
            self.latest_rx = None;
        }
        let mut done = None;
        if let Some(rx) = &self.install_rx {
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    InstallEvent::Status(s) => self.install_status = s,
                    InstallEvent::Progress(d, t) => self.install_progress = Some((d, t)),
                    InstallEvent::Done(r) => done = Some(r),
                }
            }
        }
        if let Some(result) = done {
            self.install_rx = None;
            self.install_progress = None;
            self.install_status.clear();
            // The private folder was put on PATH at startup, so a rescan finds the new binaries.
            self.refresh();
            self.message = match result {
                Ok(name) => format!("Installed {name}. It is used by spectrocut right away."),
                Err(e) => format!("Download failed: {e}"),
            };
        }
    }

    pub fn package_command(&self) -> Option<PackageCommand> {
        let cmd = |program: &str, args: &[&str]| PackageCommand {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
        };
        if cfg!(windows) {
            return find_in_path("winget").map(|_| {
                cmd(
                    "winget",
                    &[
                        "install",
                        "--id",
                        "Gyan.FFmpeg",
                        "-e",
                        "--accept-source-agreements",
                    ],
                )
            });
        }
        if find_in_path("brew").is_some() {
            return Some(cmd("brew", &["install", "ffmpeg"]));
        }
        // Prefix with sudo when available (Crostini has passwordless sudo by default).
        let sudo = |full: &[&str]| match find_in_path("sudo") {
            Some(_) => cmd("sudo", full),
            None => cmd(full[0], &full[1..]),
        };
        if find_in_path("apt-get").is_some() {
            Some(sudo(&[
                "sh",
                "-c",
                "apt-get update && apt-get install -y ffmpeg",
            ]))
        } else if find_in_path("dnf").is_some() {
            Some(sudo(&["dnf", "install", "-y", "ffmpeg"]))
        } else if find_in_path("pacman").is_some() {
            Some(sudo(&["pacman", "-S", "--noconfirm", "ffmpeg"]))
        } else if find_in_path("pkg").is_some() {
            Some(cmd("pkg", &["install", "-y", "ffmpeg"])) // Termux
        } else {
            None
        }
    }

    /// Make the private tool folder permanent in the user's shell PATH.
    pub fn add_to_shell_path(&mut self) {
        self.message = match Self::write_shell_path() {
            Ok(msg) => msg,
            Err(e) => format!("Could not update PATH: {e:#}"),
        };
    }

    pub fn draw(&self, frame: &mut Frame, area: Rect) {
        let [table_area, info_area, gauge_area] = Layout::vertical([
            Constraint::Length(self.tools.len() as u16 + 4),
            Constraint::Min(6),
            Constraint::Length(3),
        ])
        .areas(area);

        let mut rows: Vec<Line> = vec![Line::styled(
            format!(
                "{:<10} {:<8} {:<22} {}",
                "Tool", "Status", "Version", "Location"
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        for t in &self.tools {
            let (status, color) = match (&t.path, t.required) {
                (Some(_), _) => ("found", Color::Green),
                (None, true) => ("MISSING", Color::Red),
                (None, false) => ("missing", Color::Yellow),
            };
            let location = t.path.as_ref().map_or("-".to_string(), |p| {
                let local = local_bin_dir().is_some_and(|d| p.starts_with(d));
                format!(
                    "{}{}",
                    p.display(),
                    if local { "  (spectrocut)" } else { "" }
                )
            });
            rows.push(Line::from(vec![
                Span::raw(format!("{:<10} ", t.name)),
                Span::styled(format!("{status:<8} "), Style::default().fg(color)),
                Span::raw(format!("{:<22.22} ", t.version.as_deref().unwrap_or("-"))),
                Span::styled(location, Style::default().fg(Color::DarkGray)),
            ]));
        }
        let (x264, color) = match self.has_x264 {
            Some(true) => ("yes", Color::Green),
            Some(false) => ("NO: this ffmpeg cannot encode H.264", Color::Red),
            None => ("-", Color::DarkGray),
        };
        rows.push(Line::from(vec![
            Span::raw(format!("{:<10} ", "libx264")),
            Span::styled(x264, Style::default().fg(color)),
        ]));
        frame.render_widget(
            Paragraph::new(rows).block(Block::default().borders(Borders::ALL).title(" Tools ")),
            table_area,
        );

        let mut info: Vec<Line> = Vec::new();
        let latest = match (&self.latest, &self.latest_rx) {
            (_, Some(_)) => "checking…".to_string(),
            (Some(Ok(a)), _) => format!(
                "{}  ({}, built {})",
                a.name,
                fmt_bytes(a.size),
                short_date(&a.published)
            ),
            (Some(Err(e)), _) => format!("unavailable: {e}"),
            (None, _) => "press u to check".into(),
        };
        info.push(Line::from(vec![
            Span::styled("Latest static build: ", Style::default().fg(Color::Cyan)),
            Span::raw(latest),
        ]));
        let installed = self
            .installed_stamp
            .clone()
            .unwrap_or_else(|| "none".into());
        info.push(Line::from(vec![
            Span::styled(
                "Downloaded by spectrocut: ",
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(installed.replace('\n', ", built ")),
        ]));
        if let (Some(Ok(a)), Some(stamp)) = (&self.latest, &self.installed_stamp)
            && !stamp.starts_with(&a.name)
        {
            info.push(Line::styled(
                "→ A newer build series is available: press d to update.",
                Style::default().fg(Color::Yellow),
            ));
        }
        if let Some(dir) = local_bin_dir() {
            info.push(Line::from(vec![
                Span::styled("Private tool folder: ", Style::default().fg(Color::Cyan)),
                Span::raw(dir.display().to_string()),
            ]));
        }
        info.push(Line::raw(""));
        info.push(Line::raw(
            "d  download / update static ffmpeg + ffprobe (no sudo needed)",
        ));
        match self.package_command() {
            Some(p) => info.push(Line::raw(format!(
                "p  install with the system package manager: {}",
                p.display()
            ))),
            None => info.push(Line::styled(
                "p  (no supported package manager found)",
                Style::default().fg(Color::DarkGray),
            )),
        }
        info.push(Line::raw(
            "a  add the private tool folder to your shell PATH permanently",
        ));
        info.push(Line::raw(
            "u  check for updates    r  rescan    Esc  back to settings",
        ));
        if !self.message.is_empty() {
            info.push(Line::raw(""));
            info.push(Line::styled(
                self.message.clone(),
                Style::default().fg(Color::Yellow),
            ));
        }
        frame.render_widget(
            Paragraph::new(info).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Install / update "),
            ),
            info_area,
        );

        let (ratio, label) = match self.install_progress {
            Some((d, t)) if t > 0 => (
                (d as f64 / t as f64).min(1.0),
                format!(
                    "{} — {} / {}",
                    self.install_status,
                    fmt_bytes(d),
                    fmt_bytes(t)
                ),
            ),
            _ if self.is_busy() => (0.0, self.install_status.clone()),
            _ => (0.0, String::new()),
        };
        frame.render_widget(
            Gauge::default()
                .block(Block::default().borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Green))
                .ratio(ratio)
                .label(label),
            gauge_area,
        );
    }
}

/// Download, extraction and PATH plumbing (no `self` needed).
impl ToolManager {
    /// BtbN platform tag for this machine, or None if no static build exists.
    fn platform_tag() -> Option<&'static str> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Some("linux64"),
            ("linux", "aarch64") => Some("linuxarm64"),
            ("windows", "x86_64") => Some("win64"),
            ("windows", "aarch64") => Some("winarm64"),
            _ => None,
        }
    }

    /// Picks the newest release-branch GPL static build (falls back to master).
    fn find_asset() -> Result<Asset> {
        let Some(tag) = Self::platform_tag() else {
            bail!(
                "no static build for {}/{}; use the package manager (p)",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        };
        let mut resp = ureq::get(RELEASE_API)
            .header("Accept", "application/vnd.github+json")
            .call()
            .context("could not reach GitHub")?;
        let json: serde_json::Value = serde_json::from_reader(resp.body_mut().as_reader())?;
        let published = json["published_at"].as_str().unwrap_or("").to_string();
        let ext = if tag.starts_with("win") {
            ".zip"
        } else {
            ".tar.xz"
        };

        let mut best: Option<((u32, u32), Asset)> = None;
        for a in json["assets"].as_array().into_iter().flatten() {
            let name = a["name"].as_str().unwrap_or("");
            if !name.ends_with(ext)
                || name.contains("shared")
                || !name.contains(&format!("-{tag}-gpl"))
            {
                continue;
            }
            let asset = Asset {
                name: name.to_string(),
                url: a["browser_download_url"].as_str().unwrap_or("").to_string(),
                size: a["size"].as_u64().unwrap_or(0),
                published: published.clone(),
            };
            // "ffmpeg-n8.0-latest-linux64-gpl-8.0.tar.xz" -> (8, 0); master -> (0, 0)
            let version = name
                .strip_prefix("ffmpeg-n")
                .and_then(|r| r.split('-').next())
                .and_then(|v| {
                    let mut it = v.split('.').map(|n| n.parse::<u32>().ok());
                    Some((it.next()??, it.next().flatten().unwrap_or(0)))
                })
                .unwrap_or((0, 0));
            if best.as_ref().is_none_or(|(v, _)| version > *v) {
                best = Some((version, asset));
            }
        }
        best.map(|(_, a)| a)
            .context("no matching build found in the release")
    }

    fn install(bin_dir: &Path, tx: &Sender<InstallEvent>) -> Result<String> {
        let status = |s: &str| {
            let _ = tx.send(InstallEvent::Status(s.into()));
        };
        status("Looking up latest build");
        let asset = Self::find_asset()?;
        std::fs::create_dir_all(bin_dir)?;

        status(&format!("Downloading {}", asset.name));
        let archive = bin_dir.join("download.tmp");
        {
            let mut resp = ureq::get(&asset.url).call().context("download failed")?;
            let total = resp
                .headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(asset.size);
            let mut reader = resp.body_mut().as_reader();
            let mut file = File::create(&archive)?;
            let mut buf = vec![0u8; 256 * 1024];
            let mut done = 0u64;
            loop {
                let n = reader.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n])?;
                done += n as u64;
                let _ = tx.send(InstallEvent::Progress(done, total));
            }
        }

        status("Extracting");
        let wanted = [exe_name("ffmpeg"), exe_name("ffprobe")];
        let extracted = if asset.name.ends_with(".zip") {
            Self::extract_zip(&archive, bin_dir, &wanted)?
        } else {
            Self::extract_tar_xz(&archive, bin_dir, &wanted)?
        };
        let _ = std::fs::remove_file(&archive);
        if extracted < wanted.len() {
            bail!("archive did not contain ffmpeg and ffprobe");
        }
        std::fs::write(
            bin_dir.join("installed.txt"),
            format!("{}\n{}", asset.name, short_date(&asset.published)),
        )?;
        Ok(asset.name)
    }

    /// Writes `src` to `bin_dir/name` via a temp file, so a running binary is replaced safely.
    fn place_binary(mut src: impl Read, bin_dir: &Path, name: &str) -> Result<()> {
        let tmp = bin_dir.join(format!("{name}.new"));
        {
            let mut out = File::create(&tmp)?;
            std::io::copy(&mut src, &mut out)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        let dest = bin_dir.join(name);
        if dest.exists() {
            std::fs::remove_file(&dest)
                .with_context(|| format!("could not replace {name} (is it running?)"))?;
        }
        std::fs::rename(&tmp, &dest)?;
        Ok(())
    }

    fn wanted_name<'a>(entry_path: &str, wanted: &'a [String]) -> Option<&'a String> {
        let entry_path = entry_path.replace('\\', "/");
        wanted
            .iter()
            .find(|w| entry_path.ends_with(&format!("/bin/{w}")))
    }

    fn extract_zip(archive: &Path, bin_dir: &Path, wanted: &[String]) -> Result<usize> {
        let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
        let mut count = 0;
        for i in 0..zip.len() {
            let entry = zip.by_index(i)?;
            let entry_name = entry.name()?.to_string();
            if let Some(name) = Self::wanted_name(&entry_name, wanted) {
                let name = name.clone();
                Self::place_binary(entry, bin_dir, &name)?;
                count += 1;
            }
        }
        Ok(count)
    }

    fn extract_tar_xz(archive: &Path, bin_dir: &Path, wanted: &[String]) -> Result<usize> {
        // Stream xz -> tar through a pipe so the uncompressed tarball never sits in memory.
        let (reader, mut writer) = std::io::pipe()?;
        let mut input = BufReader::new(File::open(archive)?);
        let decoder = std::thread::spawn(move || lzma_rs::xz_decompress(&mut input, &mut writer));
        let mut count = 0;
        {
            let mut tar = tar::Archive::new(reader);
            for entry in tar.entries()? {
                let entry = entry?;
                let path = entry.path()?.to_string_lossy().to_string();
                if let Some(name) = Self::wanted_name(&path, wanted) {
                    let name = name.clone();
                    Self::place_binary(entry, bin_dir, &name)?;
                    count += 1;
                    if count == wanted.len() {
                        break;
                    }
                }
            }
        } // dropping the reader makes the decoder stop with a broken pipe, which is fine
        let _ = decoder.join();
        Ok(count)
    }

    fn write_shell_path() -> Result<String> {
        let dir = local_bin_dir().context("no data directory")?;
        let dir_str = dir.display().to_string();
        if cfg!(windows) {
            let script = format!(
                "$p=[Environment]::GetEnvironmentVariable('Path','User'); \
                 if (($p -split ';') -notcontains '{dir_str}') {{ \
                   [Environment]::SetEnvironmentVariable('Path', ($p.TrimEnd(';') + ';{dir_str}'), 'User'); 'added' }} else {{ 'present' }}"
            );
            let out = Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .stdin(Stdio::null())
                .output()?;
            let text = String::from_utf8_lossy(&out.stdout);
            return Ok(if text.contains("present") {
                format!("{dir_str} is already on your user PATH.")
            } else {
                format!("Added {dir_str} to your user PATH. Open a new terminal to use it.")
            });
        }
        let home = directories::BaseDirs::new()
            .context("no home directory")?
            .home_dir()
            .to_path_buf();
        let line = format!("\n{PROFILE_MARKER}\nexport PATH=\"{dir_str}:$PATH\"\n");
        let mut touched = Vec::new();
        for rc in [".bashrc", ".profile", ".zshrc"] {
            let path = home.join(rc);
            // Only create .bashrc/.profile; touch .zshrc only if the user has one.
            if rc == ".zshrc" && !path.exists() {
                continue;
            }
            let existing = std::fs::read_to_string(&path).unwrap_or_default();
            if existing.contains(PROFILE_MARKER) || existing.contains(&dir_str) {
                continue;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            f.write_all(line.as_bytes())?;
            touched.push(rc);
        }
        Ok(if touched.is_empty() {
            format!("{dir_str} is already in your shell profile.")
        } else {
            format!(
                "Added {dir_str} to ~/{}. Open a new terminal to use it.",
                touched.join(", ~/")
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Network test: `cargo test -- --ignored latest_asset`
    #[test]
    #[ignore]
    fn latest_asset_resolves() {
        let a = ToolManager::find_asset().expect("asset");
        println!("{} {} {}", a.name, a.size, a.url);
        assert!(a.url.starts_with("https://"));
    }

    /// Network test, downloads the full build: `cargo test -- --ignored --nocapture install_into_temp`
    #[test]
    #[ignore]
    fn install_into_temp() {
        let dir = std::env::temp_dir().join("spectrocut-install-test");
        let _ = std::fs::remove_dir_all(&dir);
        let (tx, _rx) = mpsc::channel();
        let name = ToolManager::install(&dir, &tx).expect("install");
        println!("installed {name}");
        let ffmpeg = dir.join(exe_name("ffmpeg"));
        println!("version: {:?}", tool_version(&ffmpeg));
        assert!(tool_version(&ffmpeg).is_some());
        assert!(dir.join(exe_name("ffprobe")).is_file());
    }
}
